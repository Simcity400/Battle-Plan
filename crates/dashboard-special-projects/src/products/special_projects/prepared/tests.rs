use super::*;
use crate::products::special_projects::history_tests::frame;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_base::test_fixtures::{CAMPAIGN, records};
use replay::{
    container::{Replay, ReplayStream},
    frame::TableView,
    rows::TableRows,
};

fn source(frames: &[DecodedFrame]) -> Source {
    super::super::history_tests::source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap(),
        "prepared",
    )
}
fn decoded(source: &Source) -> Vec<DecodedFrame> {
    let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
    let mut frames = Vec::new();
    while stream
        .next_block(source.replay_ref(), |frame| {
            frames.push(frame.clone());
            Ok(())
        })
        .unwrap()
    {}
    frames
}
fn params(tag: &str) -> SpecialProjectsParams {
    SpecialProjectsParams {
        branch_id: "1".into(),
        tag: tag.into(),
        selection_tags: Vec::new(),
        hour: None,
    }
}
fn compare(
    fixture: &Fixture,
    source: &Source,
    params: &SpecialProjectsParams,
    context: &LoadContext,
) -> SpecialProjectsReply {
    let flag = AtomicBool::new(false);
    let actual = complete_reply(&fixture.store, &fixture.key, params, &flag)
        .unwrap()
        .unwrap();
    let expected = SpecialProjects::load(source, params, context, &flag).unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    actual
}
fn publish(fixture: &Fixture, prepared: &mut Prepared) {
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}

#[test]
fn consecutive_project_progress_changes_match_hourly_reply_bytes() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 1);
    let mut next = first.clone();
    next.hour = 101;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *next.tables[TableId::Projects.index()]
    {
        rows[0].state.prototype_progress = 71;
    }
    let source = source(&[first, next]);
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in decoded(&source) {
        prepared.observe(&source, &context, &frame, &flag).unwrap();
    }
    let fixture = Fixture::new();
    publish(&fixture, &mut prepared);
    let params = params("opaque-1");
    let expected = SpecialProjects::load(&source, &params, &context, &flag).unwrap();
    assert_eq!(expected.projects.len(), 1);
    assert_eq!(
        expected.projects[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.progress))
            .collect::<Vec<_>>(),
        [(100, 100, 1), (101, 101, 71)]
    );
    let actual = complete_reply(&fixture.store, &fixture.key, &params, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}

#[test]
fn prepared_special_projects_match_two_commits_resume_append_progress_gaps_and_holder_changes() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut frames = vec![
        frame(100, true, 2),
        frame(101, true, 2),
        frame(102, true, 2),
        frame(103, true, 2),
    ];
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *frames[1].tables[TableId::Projects.index()]
    {
        rows[0].state.prototype_progress = 71;
    }
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *frames[2].tables[TableId::Projects.index()]
    {
        rows[0].state.current_offset = 0xe8;
        rows[0].state.not_completed = 0;
        rows[0].program_reference = 0;
    }
    frames[3] = frames[2].clone();
    frames[3].hour = 103;
    let full = source(&frames);
    let accepted = decoded(&full);
    let mut prepared = Prepared::new(full.replay_ref(), 1);
    let mut fixture = Fixture::new();
    for range in [0..2, 2..4] {
        for frame in &accepted[range.clone()] {
            prepared.observe(&full, &context, frame, &flag).unwrap();
        }
        publish(&fixture, &mut prepared);
        let prefix = source(&frames[..range.end]);
        compare(&fixture, &prefix, &params("opaque-1"), &context);
        compare(&fixture, &prefix, &params("opaque-2"), &context);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        prepared.restore_views(&accepted[range.end - 1]);
    }
    let mut missing = frame(105, true, 2);
    missing.tables[TableId::Projects.index()] = TableView::Unavailable {
        reason: "synthetic failed read".into(),
    }
    .into();
    frames.push(missing);
    frames.push(frame(106, true, 2));
    let mut changed = frame(107, true, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 101,
            canonical_slot: 0,
        });
        rows.sort_by_key(|row| row.id);
    }
    frames.push(changed);
    let appended = source(&frames);
    let accepted = decoded(&appended);
    prepared.restore_views(&accepted[3]);
    let mut writer = fixture.append_writer();
    for frame in &accepted[4..] {
        prepared.observe(&appended, &context, frame, &flag).unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    for tag in ["opaque-1", "opaque-2"] {
        let reply = compare(&fixture, &appended, &params(tag), &context);
        assert_eq!(reply.read_hours, vec![100, 101, 102, 103, 106, 107]);
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    assert!(reader.value_at(0, 105).unwrap().is_none());
    assert_eq!(reader.value_at(0, 106).unwrap().unwrap().last_hour, 107);
}

#[test]
fn prepared_projects_filter_native_holders_before_creating_episode_ids() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = frame(hour, true, 2);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = if matches!(hour, 100 | 102) {
                    b"root".to_vec()
                } else {
                    b"target".to_vec()
                };
                rows[1].tag = if hour == 102 {
                    b"target".to_vec()
                } else {
                    b"other".to_vec()
                };
            }
            frame
        })
        .collect();
    let source = source(&frames);
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for part in accepted.chunks(3) {
        for frame in part {
            prepared.observe(&source, &context, frame, &flag).unwrap();
        }
        publish(&fixture, &mut prepared);
    }
    let mut selected = params("target");
    selected.selection_tags = vec!["root".into()];
    let reply = compare(&fixture, &source, &selected, &context);
    assert_eq!(reply.projects.len(), 1);
    assert_eq!(reply.projects[0].id, "1:101:0");
    assert_eq!(
        reply.projects[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(101, 101), (103, 104)]
    );
    assert_eq!(
        compare(&fixture, &source, &params("target"), &context)
            .projects
            .len(),
        3
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_projects_keep_five_thousand_countries_and_continuation_does_not_grow_with_hours() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let frames = vec![frame(100, true, 5000), frame(101, true, 5000)];
    let source = source(&frames);
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted {
        prepared.observe(&source, &context, frame, &flag).unwrap();
        publish(&fixture, &mut prepared);
    }
    for tag in ["opaque-1", "opaque-2500", "opaque-5000"] {
        compare(&fixture, &source, &params(tag), &context);
    }
    assert_eq!(prepared.keys.len(), 5000);
    let first = serde_json::to_vec(&prepared).unwrap().len();
    let mut same = accepted.last().unwrap().clone();
    for hour in 102..=125 {
        same.hour = hour;
        prepared.observe(&source, &context, &same, &flag).unwrap();
        publish(&fixture, &mut prepared);
    }
    let last = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!(
        "special-project continuation: {first} bytes at 2 hours; {last} bytes at 26 hours for 5,000 countries"
    );
    assert!(last < first + first / 20);
    let (keys, _) =
        series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, "special-projects", 1)
            .unwrap()
            .unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    assert_eq!(
        reader
            .event_count_through(keys["opaque-5000"], i32::MAX)
            .unwrap(),
        1
    );
    // One table-level branch run, rather than one read event per country/hour.
    assert_eq!(reader.event_count_through(0, i32::MAX).unwrap(), 0);
    assert_eq!(reader.value_at(0, 125).unwrap().unwrap().first_hour, 100);
}

#[test]
fn prepared_project_completion_keeps_selected_scientist_dates_and_first_use_catalogue_order() {
    use replay::rows::{CharacterRow, ProjectHistoryRow, ProjectProgramRow};
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut first = frame(100, true, 1);
    first.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 73,
            token: b"literal scientist".to_vec(),
            ..Default::default()
        }]))
        .into();
    first.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 1,
            owner: 1,
            scientist: (73u64 << 32) | 73,
            ..Default::default()
        }]))
        .into();
    let mut done = first.clone();
    done.hour = 101;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *done.tables[TableId::Projects.index()]
    {
        rows[0].state.current_offset = 0xe8;
        rows[0].state.not_completed = 0;
        rows[0].program_reference = 0;
    }
    // Native completion adds its history entry. Without it the iteration count
    // drops from zero to minus one, correctly starting a different episode.
    done.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 1,
            project: 1,
            ordinal: 0,
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date: 101,
            ..Default::default()
        }]))
        .into();
    let mut carried = done.clone();
    carried.hour = 105;
    let frames = vec![first, done, carried];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for accepted in decoded(&source).chunks(2) {
        for frame in accepted {
            prepared.observe(&source, &context, frame, &flag).unwrap();
        }
        publish(&fixture, &mut prepared);
    }
    let reply = compare(&fixture, &source, &params("opaque-1"), &context);
    assert_eq!(reply.people.len(), 2);
    assert_eq!(reply.people[0].id, Some(73));
    assert_eq!(reply.people[1].assigned, Some(false));
    let completed = reply.projects[0]
        .runs
        .iter()
        .find(|run| run.state == ProjectState::Completed)
        .unwrap();
    assert_eq!(
        reply.presentations[completed.presentation as usize].scientist_as_of,
        Some(100)
    );
}

#[test]
fn prepared_scoped_project_names_use_the_same_frame_pass_and_preserve_country_refusals_after_resume()
 {
    use replay::rows::{
        CountryRow, ProvinceRow, ScriptVariableEntry, ScriptVariablesRow, StateRow,
    };
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = super::super::tests::TempRoot::new("prepared-project-scope");
    super::super::history_tests::content(&root.0, "[ROOT.ProjectStage]");
    let localisation = root.0.join("localisation/mod_l_english.yml");
    let mut text = std::fs::read_to_string(&localisation).unwrap();
    text.push_str(" opaque-2_mod_project:0 \"[ROOT.MissingProjectProperty]\"\n");
    std::fs::write(localisation, text).unwrap();
    std::fs::write(root.0.join("common/scripted_localisation/refusal.txt"), "entry={name=MissingProjectProperty text={trigger={has_country_flag=mod_project} localization_key=ACTIVE}}").unwrap();
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&root.0);
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = frame(hour, true, 2);
            frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=2)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("opaque-{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
            frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    (1..=2)
                        .map(|id| CountryLinkRow {
                            id,
                            canonical_slot: id,
                        })
                        .collect(),
                ))
                .into();
            frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                    id: 1,
                    state: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(
                    std::iter::once(ScriptVariablesRow {
                        scope_kind: 0,
                        scope_id: 0,
                        seed: replay::rows::Recorded::Value(None),
                        entries: Vec::new(),
                    })
                    .chain((1..=2).map(|id| ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id: id,
                        seed: replay::rows::Recorded::Value(None),
                        entries: vec![ScriptVariableEntry {
                            name: b"project_flag".to_vec(),
                            value_raw: if hour < 102 { 0 } else { 100_000 },
                        }],
                    }))
                    .collect(),
                ))
                .into();
            frame
        })
        .collect();
    let mut entries = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut entries[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = super::super::history_tests::source(
        Replay::from_records(CAMPAIGN.into(), &[entries]).unwrap(),
        "scope",
    );
    let flag = AtomicBool::new(false);
    let context = LoadContext {
        content: Some(options),
    };
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for range in [0..2, 2..4] {
        for frame in &accepted[range.clone()] {
            prepared.observe(&source, &context, frame, &flag).unwrap();
        }
        publish(&fixture, &mut prepared);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        prepared.restore_views(&accepted[range.end - 1]);
    }
    let reply = compare(&fixture, &source, &params("opaque-1"), &context);
    let names: Vec<_> = reply.projects[0]
        .runs
        .iter()
        .map(|run| reply.presentations[run.presentation as usize].name.as_str())
        .collect();
    assert_eq!(names, ["Waiting", "Active"]);
    let missing = compare(&fixture, &source, &params("opaque-2"), &context);
    assert!(
        missing
            .warnings
            .iter()
            .any(|warning| warning.contains("MissingProjectProperty"))
    );
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("MissingProjectProperty"))
    );
    assert_eq!(
        source
            .state::<history::Scans>()
            .lock()
            .unwrap()
            .decoded_blocks,
        source.replay_ref().index.len()
    );
}

/// Countries 1 to `countries`, each holding a project whose scoped name
/// reads its `project_flag` variable, which turns on for country 1 alone at
/// `on`; every other row equal at every hour.
fn scoped_countries_frame(hour: i32, countries: u32, on: i32) -> DecodedFrame {
    use replay::rows::{
        CountryRow, ProvinceRow, ScriptVariableEntry, ScriptVariablesRow, StateRow,
    };
    let mut frame = frame(hour, true, countries);
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("opaque-{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            std::iter::once(ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: replay::rows::Recorded::Value(None),
                entries: Vec::new(),
            })
            .chain((1..=countries).map(|id| ScriptVariablesRow {
                scope_kind: 1,
                scope_id: id,
                seed: replay::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"project_flag".to_vec(),
                    value_raw: if id == 1 && hour >= on { 100_000 } else { 0 },
                }],
            }))
            .collect(),
        ))
        .into();
    frame
}

/// With hundreds of countries' scoped names, an hour at which only some
/// names change revisits only those countries' presentations, and
/// publishes what revisiting every country does (the oracle).
#[test]
fn prepared_scoped_project_names_revisit_only_countries_whose_names_changed() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = super::super::tests::TempRoot::new("prepared-project-refresh");
    super::super::history_tests::content(&root.0, "[ROOT.ProjectStage]");
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&root.0);
    let countries = 300;
    let frames: Vec<_> = (100..=105)
        .map(|hour| scoped_countries_frame(hour, countries, 103))
        .collect();
    let mut entries = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut entries[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = super::super::history_tests::source(
        Replay::from_records(CAMPAIGN.into(), &[entries]).unwrap(),
        "refresh",
    );
    let flag = AtomicBool::new(false);
    let context = LoadContext {
        content: Some(options),
    };
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &decoded(&source) {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    // The first hour observes every country; of the five after it, only
    // country 1's name changes, at 103: revisiting every active country at
    // each of them would revisit 5 * 300.
    assert_eq!(prepared.refreshed, 1);
    let fixture = Fixture::new();
    publish(&fixture, &mut prepared);
    for (tag, expected) in [
        ("opaque-1", &["Waiting", "Active"][..]),
        ("opaque-2", &["Waiting"][..]),
        ("opaque-300", &["Waiting"][..]),
    ] {
        let reply = compare(&fixture, &source, &params(tag), &context);
        let names: Vec<_> = reply.projects[0]
            .runs
            .iter()
            .map(|run| reply.presentations[run.presentation as usize].name.as_str())
            .collect();
        assert_eq!(names, expected, "{tag}");
    }
}

#[test]
fn prepared_scoped_project_names_reuse_an_evaluation_whose_inputs_read_the_same() {
    use replay::rows::{
        CountryRow, ProvinceRow, ScriptVariableEntry, ScriptVariablesRow, StateRow,
    };
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = super::super::tests::TempRoot::new("prepared-project-reuse");
    // opaque-1's name reads its `project_flag` variable; opaque-2's reads a
    // country flag, never the variable.
    super::super::history_tests::content(&root.0, "[ROOT.ProjectStage]");
    let localisation = root.0.join("localisation/mod_l_english.yml");
    let mut text = std::fs::read_to_string(&localisation).unwrap();
    text.push_str(" opaque-2_mod_project:0 \"[ROOT.MissingProjectProperty]\"\n");
    std::fs::write(localisation, text).unwrap();
    std::fs::write(root.0.join("common/scripted_localisation/refusal.txt"), "entry={name=MissingProjectProperty text={trigger={has_country_flag=mod_project} localization_key=ACTIVE}}").unwrap();
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&root.0);
    // Every hour equal but the variable, which turns on at 103.
    let frames: Vec<_> = (100..=105)
        .map(|hour| {
            let mut frame = frame(hour, true, 2);
            frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=2)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("opaque-{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
            frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    (1..=2)
                        .map(|id| CountryLinkRow {
                            id,
                            canonical_slot: id,
                        })
                        .collect(),
                ))
                .into();
            frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                    id: 1,
                    state: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(
                    std::iter::once(ScriptVariablesRow {
                        scope_kind: 0,
                        scope_id: 0,
                        seed: replay::rows::Recorded::Value(None),
                        entries: Vec::new(),
                    })
                    .chain((1..=2).map(|id| ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id: id,
                        seed: replay::rows::Recorded::Value(None),
                        entries: vec![ScriptVariableEntry {
                            name: b"project_flag".to_vec(),
                            value_raw: if hour < 103 { 0 } else { 100_000 },
                        }],
                    }))
                    .collect(),
                ))
                .into();
            frame
        })
        .collect();
    let mut entries = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut entries[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = super::super::history_tests::source(
        Replay::from_records(CAMPAIGN.into(), &[entries]).unwrap(),
        "reuse",
    );
    let flag = AtomicBool::new(false);
    let context = LoadContext {
        content: Some(options),
    };
    let accepted = decoded(&source);
    let observe = |prepared: &mut Prepared| {
        for frame in &accepted {
            prepared.observe(&source, &context, frame, &flag).unwrap();
        }
    };
    // Without check mode, each name is evaluated at the first hour and
    // opaque-1's again where its variable changes; every other hour reuses.
    let mut reused = Prepared::new(source.replay_ref(), 1);
    dashboard_text::triggers::inputs::reusing(|| observe(&mut reused));
    let counts = reused.names.reuse;
    assert_eq!((counts.evaluated, counts.reused, counts.checked), (3, 9, 0));
    // Check mode evaluates every reused name anyway and finds it the same.
    let mut checked = Prepared::new(source.replay_ref(), 1);
    observe(&mut checked);
    let counts = checked.names.reuse;
    assert_eq!(
        (
            counts.evaluated,
            counts.reused,
            counts.checked,
            counts.mismatched
        ),
        (3, 9, 9, 0)
    );
    // Both publish what evaluating every name at every hour does.
    for prepared in [&mut reused, &mut checked] {
        let fixture = Fixture::new();
        publish(&fixture, prepared);
        let reply = compare(&fixture, &source, &params("opaque-1"), &context);
        let names: Vec<_> = reply.projects[0]
            .runs
            .iter()
            .map(|run| reply.presentations[run.presentation as usize].name.as_str())
            .collect();
        assert_eq!(names, ["Waiting", "Active"]);
        let missing = compare(&fixture, &source, &params("opaque-2"), &context);
        assert!(
            missing
                .warnings
                .iter()
                .any(|warning| warning.contains("MissingProjectProperty"))
        );
    }
}

#[test]
fn prepared_constant_completed_intervals_keep_attribution_boundary_and_count_unknown_hours() {
    use replay::rows::{CharacterRow, ProjectProgramRow};
    let mut initial = frame(100, true, 1);
    initial.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 73,
            ..Default::default()
        }]))
        .into();
    initial.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 1,
            owner: 1,
            scientist: (73u64 << 32) | 73,
            ..Default::default()
        }]))
        .into();
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *initial.tables[TableId::Projects.index()]
    {
        rows[0].state.current_offset = 0xe8;
        rows[0].state.not_completed = 0;
        let mut unknown = rows[0].clone();
        unknown.local_id = 2;
        unknown.state.current_offset = 0x777;
        rows.push(unknown);
    }
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let source = source(&frames);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for accepted in decoded(&source).chunks(3) {
        for frame in accepted {
            prepared.observe(&source, &context, frame, &flag).unwrap();
        }
        publish(&fixture, &mut prepared);
    }
    let reply = compare(&fixture, &source, &params("opaque-1"), &context);
    let completed = &reply.projects[0];
    assert_eq!(
        completed
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 104)]
    );
    assert_eq!(
        reply.presentations[completed.runs[0].presentation as usize].scientist_as_of,
        None
    );
    assert_eq!(
        reply.presentations[completed.runs[1].presentation as usize].scientist_as_of,
        Some(100)
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("0x777 (5 observations)"))
    );
}

fn completion_rows(
    fixture: &Fixture,
) -> (Vec<completion_stream::Change>, completion_stream::Metadata) {
    let (keys, metadata) = series::read_catalogue::<completion_stream::Metadata>(
        &fixture.store,
        &fixture.key,
        completion_stream::FAMILY,
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        keys,
        BTreeMap::from([("completions".into(), completion_stream::KEY)])
    );
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let mut rows = Vec::new();
    let mut cursor = 0;
    loop {
        let page = reader
            .events(&[completion_stream::KEY], i32::MIN, i32::MAX, cursor, 2)
            .unwrap();
        assert!(
            page.cursor
                .is_none_or(|next| next > cursor && !page.rows.is_empty())
        );
        for event in page.rows {
            let row: completion_stream::Change = serde_json::from_value(event.row).unwrap();
            assert_eq!(event.hour, row.observed_hour);
            rows.push(row);
        }
        let Some(next) = page.cursor else { break };
        cursor = next;
    }
    (rows, metadata)
}

fn completion_latest(rows: &[completion_stream::Change]) -> Vec<ProjectCompletion> {
    let mut latest = BTreeMap::new();
    for row in rows {
        latest.insert(row.id, row.completion.clone());
    }
    let mut rows: Vec<_> = latest.into_values().flatten().collect();
    rows.sort_by_key(|row| (row.date.hour, row.tag.clone(), row.definition_id));
    rows
}

fn completed(frame: &mut DecodedFrame, local_id: u32, date: Option<i32>) {
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *frame.tables[TableId::Projects.index()]
    {
        let project = rows
            .iter_mut()
            .find(|row| row.local_id == local_id)
            .unwrap();
        project.state.current_offset = 0xe8;
        project.state.not_completed = 0;
    }
    if let Some(date) = date {
        let mut rows = frame
            .complete::<tables::ProjectHistory>()
            .unwrap_or_default()
            .to_vec();
        rows.retain(|row| row.project != local_id);
        rows.push(ProjectHistoryRow {
            owner: 1,
            project: local_id,
            ordinal: 0,
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date,
            ..Default::default()
        });
        rows.sort_by_key(|row| (row.owner, row.project, row.ordinal));
        frame.tables[TableId::ProjectHistory.index()] =
            TableView::Complete(TableRows::ProjectHistory(rows)).into();
    } else {
        frame.tables[TableId::ProjectHistory.index()] = TableView::Unavailable {
            reason: "completion date not yet recorded".into(),
        }
        .into();
    }
}

fn publish_completions(
    fixture: &mut Fixture,
    prepared: &mut Prepared,
    source: &Source,
    native: &[i32],
    append: bool,
) -> usize {
    let expected: BTreeSet<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| {
            block.branch == 1
                && native
                    .iter()
                    .any(|hour| (block.first_hour..=block.last_hour).contains(hour))
        })
        .map(|block| block.offset)
        .collect();
    dashboard_core::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let mut writer = if append {
        fixture.append_writer()
    } else {
        fixture.writer()
    };
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let actual = dashboard_core::blocks::SCANNED_BLOCKS.with_borrow(Clone::clone);
    assert_eq!(
        actual.len(),
        expected.len(),
        "native blocks read once per commit batch"
    );
    assert_eq!(actual.into_iter().collect::<BTreeSet<_>>(), expected);
    expected.len()
}

fn compare_completions(fixture: &Fixture, source: &Source, context: &LoadContext) {
    let (rows, _) = completion_rows(fixture);
    let expected =
        completions::completion_events(source, 1, context, &AtomicBool::new(false)).unwrap();
    let mut expected = expected.events.clone();
    expected.sort_by_key(|row| (row.date.hour, row.tag.clone(), row.definition_id));
    assert_eq!(completion_latest(&rows), expected);
    // The existing selected-country interval reconstruction remains an
    // independent episode/date oracle, including bounded-to-exact updates.
    let reply = complete_reply(
        &fixture.store,
        &fixture.key,
        &params("opaque-1"),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let mut dates: Vec<_> = reply
        .projects
        .iter()
        .filter_map(|episode| {
            episode
                .runs
                .iter()
                .rev()
                .find_map(|run| run.completion.clone())
        })
        .collect();
    dates.sort_by_key(|date| (date.hour, date.exact));
    let mut actual: Vec<_> = completion_latest(&rows)
        .into_iter()
        .map(|row| row.date)
        .collect();
    actual.sort_by_key(|date| (date.hour, date.exact));
    assert_eq!(actual, dates);
}

#[test]
fn completion_stream_matches_intervals_after_two_commits_resume_append_and_late_exact_upgrade() {
    use dashboard_base::test_fixtures::continuation_records;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut initial = frame(100, true, 1);
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *initial.tables[TableId::Projects.index()]
    {
        let mut second = rows[0].clone();
        second.local_id = 2;
        rows.push(second);
    }
    let frames: Vec<_> = (100..=106)
        .map(|hour| {
            let mut frame = initial.clone();
            frame.hour = hour;
            if hour >= 102 {
                completed(&mut frame, 1, (hour >= 104).then_some(101));
            }
            if hour >= 106 {
                completed(&mut frame, 2, Some(105));
            }
            frame
        })
        .collect();
    let prefix = source(&frames[..4]);
    let first_records = records(1, frames[..4].to_vec());
    let second_records = continuation_records(prefix.replay_ref(), 1, frames[4..].to_vec());
    let full = super::super::history_tests::source(
        Replay::from_records(
            CAMPAIGN.into(),
            &[first_records.clone(), second_records.clone()],
        )
        .unwrap(),
        "two-commits",
    );
    let accepted = decoded(&full);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(full.replay_ref(), 1);
    for frame in &accepted[..4] {
        prepared.observe(&full, &context, frame, &flag).unwrap();
    }
    let first_reads = publish_completions(&mut fixture, &mut prepared, &full, &[102], false);
    assert_eq!(first_reads, 1);
    compare_completions(&fixture, &prefix, &context);
    let (bounded, metadata) = completion_rows(&fixture);
    assert_eq!(bounded.len(), 1);
    assert!(!bounded[0].date.exact);
    assert_eq!(
        metadata.late.between(i32::MIN, i32::MAX, i32::MAX).count(),
        0
    );
    let saved = serde_json::to_value(&prepared).unwrap();
    assert!(saved["completions"].get("pending").is_none());
    assert!(
        !serde_json::to_string(&saved["completions"])
            .unwrap()
            .contains("not_completed")
    );
    prepared = serde_json::from_value(saved).unwrap();
    prepared.restore_views(&accepted[3]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &full, &[], false),
        0
    );
    for frame in &accepted[4..] {
        prepared.observe(&full, &context, frame, &flag).unwrap();
    }
    let second_reads = publish_completions(&mut fixture, &mut prepared, &full, &[101, 105], false);
    assert_eq!(second_reads, 2);
    compare_completions(&fixture, &full, &context);
    let (rows, metadata) = completion_rows(&fixture);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], bounded[0], "the bounded row is never rewritten");
    assert_eq!(rows[0].id, rows[1].id);
    assert!(rows[1].date.exact);
    assert_eq!(
        metadata
            .late
            .between(i32::MIN, i32::MAX, i32::MAX)
            .copied()
            .collect::<Vec<_>>(),
        [(101, 104, 0, 1), (105, 106, 0, 2)]
    );
    assert_eq!(metadata.late.between(101, 101, 103).count(), 0);
    assert_eq!(metadata.late.between(101, 101, 104).count(), 1);
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[6]);
    let mut repeated = accepted[6].clone();
    repeated.hour = 107;
    prepared.observe(&full, &context, &repeated, &flag).unwrap();
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &full, &[], false),
        0
    );
    assert_eq!(completion_rows(&fixture).0, rows);

    let mut appended_frame = frames[6].clone();
    appended_frame.hour = 108;
    if let TableView::Complete(TableRows::Projects(projects)) =
        &mut *appended_frame.tables[TableId::Projects.index()]
    {
        let mut third = projects[0].clone();
        third.local_id = 3;
        projects.push(third);
    }
    completed(&mut appended_frame, 3, Some(100));
    let append_records = continuation_records(full.replay_ref(), 1, vec![appended_frame]);
    let appended = super::super::history_tests::source(
        Replay::from_records(
            CAMPAIGN.into(),
            &[first_records, second_records, append_records],
        )
        .unwrap(),
        "appended",
    );
    prepared
        .observe(
            &appended,
            &context,
            decoded(&appended).last().unwrap(),
            &flag,
        )
        .unwrap();
    let append_reads = publish_completions(&mut fixture, &mut prepared, &appended, &[100], true);
    assert_eq!(append_reads, 1);
    compare_completions(&fixture, &appended, &context);
    let (rows, metadata) = completion_rows(&fixture);
    assert_eq!(rows.len(), 4);
    assert_eq!(serde_json::to_vec(&metadata.late).unwrap().len(), 43);
    assert_eq!(
        metadata.late.between(i32::MIN, i32::MAX, i32::MAX).count(),
        3
    );
    eprintln!(
        "completion stream: native block reads {first_reads}, {second_reads}, {append_reads}; resume/repeat 0; late references 3/4, {} JSON bytes",
        serde_json::to_vec(&metadata.late).unwrap().len()
    );
}

#[test]
fn completion_stream_native_presentation_survives_late_tag_name_and_program_changes() {
    use replay::rows::{
        CharacterRow, LexicalTokenRow, ProjectProgramRow, ScriptVariableEntry, ScriptVariablesRow,
    };
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let folder = super::super::tests::TempRoot::new("prepared-completion-native");
    super::super::history_tests::content(&folder.0, "[ROOT.ProjectStage]");
    std::fs::write(
        folder.0.join("common/special_projects/projects/mod.txt"),
        "mod_project={specialization=mod_kind} mod_later={specialization=mod_kind}",
    )
    .unwrap();
    std::fs::write(
        folder.0.join("localisation/native_l_english.yml"),
        // Character names are plain: bracketed scope text is valid glyphs.
        // An authored control reaches the existing named plain-text refusal.
        "\u{feff}l_english:\n mod_later:0 \"Supplying name\"\n bad_scientist:0 \"Scientist\u{a4}\"\n",
    )
    .unwrap();
    let frames: Vec<_> = (100..=105)
        .map(|hour| {
            let mut frame = frame(hour, true, 1);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].canonical_slot = 1;
                rows[0].tag = if hour <= 101 {
                    b"native".to_vec()
                } else {
                    b"supplied".to_vec()
                };
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows[0].canonical_slot = 1;
            }
            frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![replay::rows::StateRow {
                    id: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![replay::rows::ProvinceRow {
                    id: 1,
                    state: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(vec![
                    ScriptVariablesRow {
                        scope_kind: 0,
                        scope_id: 0,
                        seed: replay::rows::Recorded::Value(None),
                        entries: vec![],
                    },
                    ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id: 1,
                        seed: replay::rows::Recorded::Value(None),
                        entries: vec![ScriptVariableEntry {
                            name: b"project_flag".to_vec(),
                            value_raw: if hour == 101 { 100_000 } else { 0 },
                        }],
                    },
                ]))
                .into();
            frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 8,
                    literal: if hour <= 101 {
                        b"mod_project".to_vec()
                    } else {
                        b"mod_later".to_vec()
                    },
                }]))
                .into();
            frame.tables[TableId::Characters.index()] =
                TableView::Complete(TableRows::Characters(vec![CharacterRow {
                    id: 73,
                    token: if hour == 101 {
                        b"bad_scientist".to_vec()
                    } else {
                        b"Saved scientist".to_vec()
                    },
                    name_token_id: -1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::ProjectPrograms.index()] =
                TableView::Complete(TableRows::ProjectPrograms(vec![
                    ProjectProgramRow {
                        id: 1,
                        owner: 1,
                        scientist: if hour <= 101 { (73u64 << 32) | 73 } else { 0 },
                        ..Default::default()
                    },
                    ProjectProgramRow {
                        id: 2,
                        owner: 1,
                        scientist: 0,
                        ..Default::default()
                    },
                ]))
                .into();
            if hour >= 103 {
                completed(&mut frame, 1, (hour == 105).then_some(101));
            }
            if hour == 105
                && let TableView::Complete(TableRows::Projects(rows)) =
                    &mut *frame.tables[TableId::Projects.index()]
            {
                rows[0].program_reference = (2u64 << 32) | 83;
            }
            frame
        })
        .collect();
    let (source, context) =
        dashboard_base::test_fixtures::source_with_content(frames.clone(), &folder.0);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..4] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    let first_reads = publish_completions(&mut fixture, &mut prepared, &source, &[103], false);
    assert_eq!(first_reads, 1);
    let (first, _) = completion_rows(&fixture);
    assert_eq!(
        first[0].completion.as_ref().unwrap().name.as_deref(),
        Some("Supplying name")
    );
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[3]);
    for frame in &accepted[4..] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    let second_reads = publish_completions(&mut fixture, &mut prepared, &source, &[101], false);
    assert_eq!(second_reads, 1);
    let (rows, metadata) = completion_rows(&fixture);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], first[0]);
    assert_eq!(rows[0].id, rows[1].id);
    assert_eq!(rows[1].identity, (1, 1));
    assert_eq!(serde_json::to_vec(&metadata.late).unwrap().len(), 15);
    assert_eq!(
        rows[1].program_reference,
        (1u64 << 32) | 83,
        "preserve the first completion's program operand"
    );
    let event = rows[1].completion.as_ref().unwrap();
    assert_eq!(event.tag, "native");
    assert_eq!(event.name.as_deref(), Some("Active"));
    assert_eq!(event.item_token.as_deref(), Some("mod_project"));
    assert_eq!(event.observed_hour, 105);
    assert_eq!(
        event.date,
        ProjectDate {
            hour: 101,
            exact: true
        }
    );
    assert!(
        rows[1]
            .refusals
            .diagnostic_lines("Project texts")
            .iter()
            .any(|line| line.contains("bad_scientist") && line.contains("control byte 0x14"))
    );

    // Build native presentation independently using the existing full-name
    // oracle, not the new selectively evaluated NameScan path.
    let scan = history::cached(&source, 1, &flag).unwrap();
    let mut content = prepared.content.as_ref().unwrap().clone();
    content.scoped = vec![(
        1,
        Arc::new(history::scoped_names(source.replay_ref(), &scan, 1, &content, &flag).unwrap()),
    )];
    let observation = CompletionObservation {
        project: accepted[3].complete::<tables::Projects>().unwrap()[0].clone(),
        identity: (1, 1),
        date: ProjectDate {
            hour: 101,
            exact: true,
        },
        observed_hour: 105,
    };
    let mut warnings = BTreeMap::new();
    let expected = completions::present_completion(
        "native".into(),
        observation,
        &accepted[1],
        Some(&mut content),
        &mut warnings,
    );
    assert_eq!(*event, expected);
    assert_eq!(rows[1].warnings, warnings.into_iter().collect::<Vec<_>>());
    assert_eq!(rows[1].refusals, take_refusals(Some(&content)));
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[5]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    let mut repeated = accepted[5].clone();
    repeated.hour = 106;
    prepared
        .observe(&source, &context, &repeated, &flag)
        .unwrap();
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    assert_eq!(completion_rows(&fixture).0, rows);
    eprintln!(
        "native presentation: {first_reads}/{second_reads} extra blocks, resume/repeat 0; late references {}, {} JSON bytes",
        metadata.late.between(i32::MIN, i32::MAX, i32::MAX).count(),
        serde_json::to_vec(&metadata.late).unwrap().len()
    );
    let (_, diagnostic_metadata) = series::read_catalogue::<completion_stream::Metadata>(
        &fixture.store,
        &fixture.key,
        completion_stream::FAMILY,
        1,
    )
    .unwrap()
    .unwrap();
    assert!(
        diagnostic_metadata
            .refusal_keys
            .keys()
            .any(|reason| reason.contains("bad_scientist"))
    );
    let keys: Vec<_> = diagnostic_metadata.refusal_keys.values().copied().collect();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    for (last, live) in [(0, 100), (3, 103), (5, 105)] {
        let values = reader.values_at(&keys, live).unwrap();
        let actual: BTreeMap<_, _> = diagnostic_metadata
            .refusal_keys
            .keys()
            .cloned()
            .zip(values)
            .filter_map(|(reason, value)| {
                value
                    .filter(|count| *count != 0)
                    .map(|count| (reason, count as usize))
            })
            .collect();
        let (prefix, prefix_context) =
            dashboard_base::test_fixtures::source_with_content(frames[..=last].to_vec(), &folder.0);
        let expected = completions::completion_events(&prefix, 1, &prefix_context, &flag).unwrap();
        assert_eq!(
            actual,
            expected
                .refusals
                .iter()
                .cloned()
                .collect::<BTreeMap<_, _>>(),
            "deduplicated native text refusal totals at Live {live}"
        );
    }
}

#[test]
fn completion_stream_exact_upgrade_with_missing_native_hour_withdraws_bounded_presentation() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 1);
    let mut bounded = frame(103, true, 1);
    completed(&mut bounded, 1, None);
    let mut exact = bounded.clone();
    exact.hour = 104;
    completed(&mut exact, 1, Some(101));
    let source = source(&[first, bounded, exact]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..2] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish_completions(&mut fixture, &mut prepared, &source, &[103], false);
    assert!(completion_rows(&fixture).0[0].completion.is_some());
    prepared
        .observe(&source, &context, &accepted[2], &flag)
        .unwrap();
    publish_completions(&mut fixture, &mut prepared, &source, &[101], false);
    let (rows, metadata) = completion_rows(&fixture);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, rows[1].id);
    assert!(rows[1].completion.is_none());
    assert_eq!(
        rows[1].warnings,
        [("Project completion hour was not recorded.".into(), 1)]
    );
    assert!(completion_latest(&rows).is_empty());
    assert_eq!(metadata.late.between(101, 101, 104).count(), 1);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: 5,000 completion presentations"
)]
fn completion_stream_keeps_five_thousand_countries_and_reads_each_native_block_once() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 5000);
    let mut middle = first.clone();
    middle.hour = 101;
    let mut done = middle.clone();
    done.hour = 102;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *done.tables[TableId::Projects.index()]
    {
        for row in rows {
            row.state.current_offset = 0xe8;
            row.state.not_completed = 0;
        }
    }
    done.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(
        (1..=5000)
            .map(|owner| ProjectHistoryRow {
                owner,
                project: owner,
                ordinal: 0,
                prototype_reward_handle: COMPLETION_MARKER,
                option_handle: COMPLETION_MARKER,
                date: 101,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    let source = source(&[first, middle, done]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..2] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[1]);
    prepared
        .observe(&source, &context, &accepted[2], &flag)
        .unwrap();
    let native_reads = publish_completions(&mut fixture, &mut prepared, &source, &[101], false);
    assert_eq!(native_reads, 1);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader
            .event_count_through(completion_stream::KEY, i32::MAX)
            .unwrap(),
        5000
    );
    let mut page_keys = Vec::new();
    let mut page_values = Vec::new();
    for ordinal in [0, 2499, 4999] {
        let row: completion_stream::Change = serde_json::from_value(
            reader
                .events(&[completion_stream::KEY], i32::MIN, i32::MAX, ordinal, 1)
                .unwrap()
                .rows
                .remove(0)
                .row,
        )
        .unwrap();
        assert_eq!(
            row.date,
            ProjectDate {
                hour: 101,
                exact: true
            }
        );
        assert!(row.completion.is_some());
        let key = completion_stream::episode_key(row.id).unwrap();
        assert_eq!(
            reader.values_at(&[key], 102).unwrap(),
            [Some(i128::from(ordinal))]
        );
        page_keys.push(key);
        page_values.push(Some(i128::from(ordinal)));
    }
    let (_, metadata) = series::read_catalogue::<completion_stream::Metadata>(
        &fixture.store,
        &fixture.key,
        completion_stream::FAMILY,
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(metadata.late.between(101, 101, 102).count(), 5000);
    assert_eq!(serde_json::to_vec(&metadata.late).unwrap().len(), 83_891);
    let saved = serde_json::to_vec(&prepared).unwrap();
    let first_size = saved.len();
    prepared = serde_json::from_slice(&saved).unwrap();
    prepared.restore_views(&accepted[2]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    let mut repeated = accepted[2].clone();
    for hour in 103..=126 {
        repeated.hour = hour;
        prepared
            .observe(&source, &context, &repeated, &flag)
            .unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    let last_size = serde_json::to_vec(&prepared).unwrap().len();
    assert!(last_size < first_size + first_size / 20);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader
            .event_count_through(completion_stream::KEY, i32::MAX)
            .unwrap(),
        5000
    );
    assert_eq!(
        reader.values_at(&page_keys, 126).unwrap(),
        page_values,
        "one values_at call validates the reached page after resume and repeated hours"
    );
    eprintln!(
        "5,000 completions: extra native blocks {native_reads}, repeat/resume 0; continuation {first_size}/{last_size} bytes; 5,000 late references, {} JSON bytes",
        serde_json::to_vec(&metadata.late).unwrap().len()
    );
}

#[test]
fn completion_stream_live_lookup_excludes_replaced_native_dates_after_two_commits_and_resume() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 1);
    let middle = frame(101, true, 1);
    let mut bounded = frame(102, true, 1);
    completed(&mut bounded, 1, None);
    let mut carried = bounded.clone();
    carried.hour = 103;
    let mut exact = bounded.clone();
    exact.hour = 104;
    completed(&mut exact, 1, Some(101));
    let mut after_gap = exact.clone();
    after_gap.hour = 107;
    let mut unchanged = after_gap.clone();
    unchanged.hour = 112;
    let source = source(&[first, middle, bounded, carried, exact, after_gap, unchanged]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..4] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish_completions(&mut fixture, &mut prepared, &source, &[102], false);
    let candidate = {
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            completion_stream::section(1),
            &flag,
        )
        .unwrap();
        // Reach just the candidate page, never the remaining events history.
        let mut page = reader
            .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 0, 1)
            .unwrap();
        let candidate = page.rows.remove(0);
        let row: completion_stream::Change = serde_json::from_value(candidate.row.clone()).unwrap();
        assert!(row.completion.is_some());
        let key = completion_stream::episode_key(row.id).unwrap();
        assert_ne!(key, completion_stream::KEY);
        assert!(reader.value_at(key, 101).unwrap().is_none());
        assert_eq!(
            reader.value_at(key, 102).unwrap().unwrap().value,
            i128::from(candidate.ordinal)
        );
        assert_eq!(
            reader.values_at(&[key], 103).unwrap(),
            [Some(i128::from(candidate.ordinal))]
        );
        candidate
    };
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[3]);
    for frame in &accepted[4..6] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish_completions(&mut fixture, &mut prepared, &source, &[101], false);
    // Restore again: the lookup, including its earlier Live values, is solely
    // prepared storage and remains readable without replaying the observer.
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[5]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    dashboard_core::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let mut page = reader
        .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 0, 1)
        .unwrap();
    assert_eq!(page.rows[0].ordinal, candidate.ordinal);
    assert_eq!(page.rows[0].row, candidate.row);
    assert!(
        page.cursor.is_some(),
        "later replacement stays outside the reached page"
    );
    let original: completion_stream::Change =
        serde_json::from_value(page.rows.remove(0).row).unwrap();
    let key = completion_stream::episode_key(original.id).unwrap();
    for live in [102, 103] {
        assert!(
            reader
                .value_at(key, live)
                .unwrap()
                .is_some_and(|run| run.value == i128::from(candidate.ordinal))
        );
    }
    for live in [104, 105, 107] {
        let current = reader.value_at(key, live).unwrap().unwrap();
        assert_ne!(
            current.value,
            i128::from(candidate.ordinal),
            "replaced candidate is excluded at Live {live}"
        );
        assert_eq!(current.value, 1);
    }
    assert_eq!(
        reader.values_at(&[key], 105).unwrap(),
        [Some(1)],
        "current references cross hour gaps"
    );
    let replacement = reader
        .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 1, 1)
        .unwrap()
        .rows
        .remove(0);
    let replacement_row: completion_stream::Change =
        serde_json::from_value(replacement.row).unwrap();
    assert_eq!(replacement_row.id, original.id);
    assert_ne!(replacement_row.date.hour, original.date.hour);
    assert_eq!(
        replacement_row.date,
        ProjectDate {
            hour: 101,
            exact: true
        }
    );
    assert!(
        reader
            .value_at(key, 103)
            .unwrap()
            .is_none_or(|run| run.value != i128::from(replacement.ordinal))
    );
    assert!(
        reader
            .value_at(key, 107)
            .unwrap()
            .is_some_and(|run| run.value == i128::from(replacement.ordinal))
    );
    assert!(
        dashboard_core::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty),
        "Live validation never reads the recording"
    );
    drop(reader);
    prepared
        .observe(&source, &context, &accepted[6], &flag)
        .unwrap();
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    assert_eq!(
        reader
            .event_count_through(completion_stream::KEY, i32::MAX)
            .unwrap(),
        2
    );
    for live in [108, 110, 112] {
        assert_eq!(
            reader.values_at(&[key], live).unwrap(),
            [Some(i128::from(replacement.ordinal))],
            "a batch with no new events carries the current reference across the gap"
        );
    }
}

#[test]
fn completion_stream_live_lookup_excludes_withdrawn_candidates_after_two_commits_and_resume() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 1);
    let mut bounded = frame(103, true, 1);
    completed(&mut bounded, 1, None);
    let mut exact = bounded.clone();
    exact.hour = 104;
    completed(&mut exact, 1, Some(101)); // Native hour 101 was never recorded.
    let mut carried = exact.clone();
    carried.hour = 108;
    let source = source(&[first, bounded, exact, carried]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..2] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish_completions(&mut fixture, &mut prepared, &source, &[103], false);
    let candidate = {
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            completion_stream::section(1),
            &flag,
        )
        .unwrap();
        let candidate = reader
            .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 0, 1)
            .unwrap()
            .rows
            .remove(0);
        let row: completion_stream::Change = serde_json::from_value(candidate.row.clone()).unwrap();
        assert!(row.completion.is_some());
        let key = completion_stream::episode_key(row.id).unwrap();
        assert!(
            reader
                .value_at(key, 103)
                .unwrap()
                .is_some_and(|run| run.value == i128::from(candidate.ordinal))
        );
        candidate
    };
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[1]);
    for frame in &accepted[2..] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish_completions(&mut fixture, &mut prepared, &source, &[101], false);
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[3]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    dashboard_core::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let mut page = reader
        .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 0, 1)
        .unwrap();
    assert_eq!(page.rows[0].ordinal, candidate.ordinal);
    assert_eq!(page.rows[0].row, candidate.row);
    assert!(
        page.cursor.is_some(),
        "withdrawal stays outside the reached candidate page"
    );
    let row: completion_stream::Change = serde_json::from_value(page.rows.remove(0).row).unwrap();
    let key = completion_stream::episode_key(row.id).unwrap();
    assert!(
        reader
            .value_at(key, 103)
            .unwrap()
            .is_some_and(|run| run.value == i128::from(candidate.ordinal)),
        "candidate remains included at its earlier Live"
    );
    for live in [104, 105, 108] {
        assert!(
            reader.value_at(key, live).unwrap().is_none(),
            "withdrawn candidate is excluded at Live {live}"
        );
        assert_eq!(reader.values_at(&[key], live).unwrap(), [None]);
    }
    let withdrawal: completion_stream::Change = serde_json::from_value(
        reader
            .events(&[completion_stream::KEY], i32::MIN, i32::MAX, 1, 1)
            .unwrap()
            .rows
            .remove(0)
            .row,
    )
    .unwrap();
    assert_eq!(withdrawal.id, row.id);
    assert!(withdrawal.completion.is_none());
    assert!(
        dashboard_core::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty),
        "Live validation never reads the recording"
    );
}

#[test]
fn completion_stream_orders_each_supplying_hour_by_native_presentation_tag_for_paging() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut first = frame(100, true, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        for (row, tag) in rows.iter_mut().zip([b"X", b"Y", b"Z"]) {
            row.tag = tag.to_vec();
        }
    }
    let mut native = first.clone();
    native.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *native.tables[TableId::Countries.index()]
    {
        for (row, tag) in rows.iter_mut().zip([b"C", b"B", b"A"]) {
            row.tag = tag.to_vec();
        }
    }
    // Recognition still happens in X/Y/Z order, with presentation tags C/B/A.
    let mut done = first.clone();
    done.hour = 102;
    for local_id in 1..=3 {
        completed(&mut done, local_id, Some(101));
    }
    if let TableView::Complete(TableRows::ProjectHistory(rows)) =
        &mut *done.tables[TableId::ProjectHistory.index()]
    {
        for row in rows {
            row.owner = row.project;
        }
    }
    let source = source(&[first, native, done]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[101], false),
        1
    );
    let (changes, _) = completion_rows(&fixture);
    assert_eq!(
        changes
            .iter()
            .map(|row| row.completion.as_ref().unwrap().tag.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert_eq!(
        changes.iter().map(|row| row.local_id).collect::<Vec<_>>(),
        [3, 2, 1],
        "sort by the resolved native tag, not the holder or current tag"
    );
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let first_page = reader
        .events(&[completion_stream::KEY], 102, 102, 0, 1)
        .unwrap();
    assert_eq!(first_page.rows.len(), 1);
    assert_eq!(first_page.cursor, Some(1));
    let first: completion_stream::Change =
        serde_json::from_value(first_page.rows[0].row.clone()).unwrap();
    assert_eq!(first.completion.unwrap().tag, "A");
    let page = reader
        .events(&[completion_stream::KEY], 102, 102, 0, 3)
        .unwrap();
    for event in page.rows {
        let row: completion_stream::Change = serde_json::from_value(event.row).unwrap();
        assert_eq!(
            reader
                .values_at(&[completion_stream::episode_key(row.id).unwrap()], 102)
                .unwrap(),
            [Some(i128::from(event.ordinal))],
            "Live references follow the sorted publication ordinals"
        );
    }
}

#[test]
fn completion_stream_equal_tags_keep_recognition_order_across_native_dates() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut first = frame(100, true, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"X".to_vec();
        rows[1].tag = b"Y".to_vec();
    }
    let mut native = first.clone();
    native.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *native.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Y".to_vec();
        rows[1].tag = b"X".to_vec();
    }
    let mut done = first.clone();
    done.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *done.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Z".to_vec();
        rows[1].tag = b"A".to_vec();
    }
    completed(&mut done, 1, Some(100));
    completed(&mut done, 2, Some(101));
    if let TableView::Complete(TableRows::ProjectHistory(rows)) =
        &mut *done.tables[TableId::ProjectHistory.index()]
    {
        for row in rows {
            row.owner = row.project;
        }
    }
    let source = source(&[first, native, done]);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[100, 101], false),
        1
    );
    let (changes, _) = completion_rows(&fixture);
    assert_eq!(
        changes
            .iter()
            .map(|row| row.completion.as_ref().unwrap().tag.as_str())
            .collect::<Vec<_>>(),
        ["X", "X"]
    );
    // Country 2 is recognised first at read 102 (current tag A), although its
    // episode ID and native date follow country 1. Both native tags are X.
    assert_eq!(
        changes
            .iter()
            .map(|row| (row.local_id, row.date.hour))
            .collect::<Vec<_>>(),
        [(2, 101), (1, 100)]
    );
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let first_page = reader
        .events(&[completion_stream::KEY], 102, 102, 0, 1)
        .unwrap();
    let first: completion_stream::Change =
        serde_json::from_value(first_page.rows[0].row.clone()).unwrap();
    assert_eq!(first.local_id, 2);
    for (ordinal, row) in changes.iter().enumerate() {
        assert_eq!(
            reader
                .values_at(&[completion_stream::episode_key(row.id).unwrap()], 102)
                .unwrap(),
            [Some(ordinal as i128)]
        );
    }
}

#[test]
fn completion_stream_refusal_counters_match_oracle_at_live_after_withdrawal_two_commits_and_resume()
{
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut first = frame(100, true, 1);
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *first.tables[TableId::Projects.index()]
    {
        let mut unknown = rows[0].clone();
        unknown.local_id = 2;
        unknown.state.current_offset = 0x777;
        let mut unmapped = rows[0].clone();
        unmapped.owner = 99;
        unmapped.local_id = 3;
        rows.extend([unknown, unmapped]);
    }
    let unmapped_history = ProjectHistoryRow {
        owner: 99,
        project: 3,
        date: 100,
        ..Default::default()
    };
    first.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![unmapped_history.clone()])).into();
    let mut carried = first.clone();
    carried.hour = 102;
    let mut bounded = first.clone();
    bounded.hour = 103;
    completed(&mut bounded, 1, None);
    let mut withdrawn = bounded.clone();
    withdrawn.hour = 104;
    completed(&mut withdrawn, 1, Some(101)); // The native hour was not recorded.
    if let TableView::Complete(TableRows::ProjectHistory(rows)) =
        &mut *withdrawn.tables[TableId::ProjectHistory.index()]
    {
        rows.push(unmapped_history);
    }
    let mut removed = withdrawn.clone();
    removed.hour = 105;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *removed.tables[TableId::Projects.index()]
    {
        rows.retain(|row| row.local_id != 1);
    }
    let mut closed = removed.clone();
    closed.hour = 106;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *closed.tables[TableId::Projects.index()]
    {
        rows.retain(|row| row.local_id != 2);
    }
    let mut after_gap = closed.clone();
    after_gap.hour = 110;
    let frames = vec![
        first, carried, bounded, withdrawn, removed, closed, after_gap,
    ];
    let source = source(&frames);
    let accepted = decoded(&source);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted[..3] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[103], false),
        1
    );
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[2]);
    for frame in &accepted[3..] {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[101], false),
        1
    );
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[6]);
    assert_eq!(
        publish_completions(&mut fixture, &mut prepared, &source, &[], false),
        0
    );
    let (_, metadata) = series::read_catalogue::<completion_stream::Metadata>(
        &fixture.store,
        &fixture.key,
        completion_stream::FAMILY,
        1,
    )
    .unwrap()
    .unwrap();
    assert!(
        metadata.refusal_keys.len() >= 5,
        "source, presentation and content-loading diagnostics each have keys"
    );
    assert!(
        metadata
            .refusal_keys
            .values()
            .all(|key| *key >= completion_stream::REFUSAL_FIRST_KEY)
    );
    let reason_keys: Vec<_> = metadata.refusal_keys.values().copied().collect();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    for (index, live) in [100, 102, 103, 104, 105, 106, 110].into_iter().enumerate() {
        // This is the feed's whole diagnostic read: one values_at for all reasons.
        dashboard_core::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
        let values = reader.values_at(&reason_keys, live).unwrap();
        let actual: BTreeMap<_, _> = metadata
            .refusal_keys
            .keys()
            .cloned()
            .zip(values)
            .filter_map(|(reason, value)| {
                value
                    .filter(|count| *count != 0)
                    .map(|count| (reason, count as usize))
            })
            .collect();
        assert!(dashboard_core::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty));
        let prefix = self::source(&frames[..=index]);
        let expected = completions::completion_events(&prefix, 1, &context, &flag).unwrap();
        let expected: BTreeMap<_, _> = expected.refusals.iter().cloned().collect();
        assert_eq!(
            actual, expected,
            "exact refusal reasons and branch counts at Live {live}"
        );
        if live < 104 {
            assert!(!actual.contains_key("Project completion hour was not recorded."));
        } else {
            assert_eq!(
                actual["Project completion hour was not recorded."], 1,
                "withdrawn and removed completions retain their refusal"
            );
        }
    }
    let (changes, _) = completion_rows(&fixture);
    assert_eq!(changes.len(), 2);
    assert!(changes[1].completion.is_none());
    assert!(
        reader
            .values_at(
                &[completion_stream::episode_key(changes[0].id).unwrap()],
                110
            )
            .unwrap()[0]
            .is_none()
    );
    eprintln!(
        "completion diagnostics: {} reasons, {} catalogue JSON bytes; two native reads, resume zero",
        metadata.refusal_keys.len(),
        serde_json::to_vec(&metadata.refusal_keys).unwrap().len()
    );
}

#[test]
fn project_changes_keep_unchanged_hours_and_closed_history_work_flat() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{assert_flat, scaled_work, unchanged_frames},
    };
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let initial = frame(100, true, 5000);
    let source = source(std::slice::from_ref(&initial));
    let format = source.replay_ref().format;
    assert_flat(
        scaled_work(10, |hours| {
            let mut prepared = Prepared::new(source.replay_ref(), 1);
            prepared.configure(&source, &context)?;
            let configured = prepared.work.content_loads;
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(&initial, &tracker.observe(&initial, format), &flag)?;
            assert_eq!(prepared.work.content_loads, configured);
            unchanged_frames(
                prepared.work,
                &initial,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_changed(frame, changes, &flag)?;
                    Ok(prepared.work)
                },
            )
        })
        .unwrap(),
    );
    let first = frame(100, true, 1);
    assert_flat(
        scaled_work(10, |history| {
            let mut prepared = Prepared::new(source.replay_ref(), 1);
            prepared.configure(&source, &context)?;
            let mut tracker = FrameChangeTracker::new();
            let mut current = first.clone();
            for index in 0..history {
                current.hour = 100 + index as i32;
                if let TableView::Complete(TableRows::Projects(rows)) =
                    &mut *current.tables[TableId::Projects.index()]
                {
                    rows[0].local_id = 1000 + index as u32;
                }
                prepared.observe_changed(&current, &tracker.observe(&current, format), &flag)?;
            }
            assert!(prepared.countries["opaque-1"].progress.len() >= history);
            unchanged_frames(prepared.work, &current, 10, format, |frame, changes| {
                prepared.observe_changed(frame, changes, &flag)?;
                Ok(prepared.work)
            })
        })
        .unwrap(),
    );
}

#[test]
fn project_lexical_and_program_dependencies_revisit_only_affected_countries_at_5000() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{Work, assert_flat, scaled_work},
    };
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    assert_flat(
        scaled_work(500, |count| {
            let initial = frame(100, true, count as u32);
            let source = source(std::slice::from_ref(&initial));
            let format = source.replay_ref().format;
            let mut prepared = Prepared::new(source.replay_ref(), 1);
            prepared.configure(&source, &context)?;
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(&initial, &tracker.observe(&initial, format), &flag)?;
            let before = prepared.work;
            let mut unrelated = initial.clone();
            unrelated.hour = 101;
            if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *unrelated.tables[TableId::LexicalTokens.index()]
            {
                rows.push(replay::rows::LexicalTokenRow {
                    token: 999,
                    literal: b"unrelated unit".to_vec(),
                });
            }
            prepared.observe_changed(&unrelated, &tracker.observe(&unrelated, format), &flag)?;
            let work = Work {
                row_visits: prepared.work.row_visits - before.row_visits,
                history_visits: prepared.work.history_visits - before.history_visits,
                emitted: prepared.work.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(
                (work.row_visits, work.history_visits, work.emitted),
                (2, 0, 0)
            );
            let mut scientist = unrelated;
            scientist.hour = 102;
            if let TableView::Complete(TableRows::ProjectPrograms(rows)) =
                &mut *scientist.tables[TableId::ProjectPrograms.index()]
            {
                rows[count / 2].scientist = (73u64 << 32) | 73;
            }
            let before = prepared.work;
            prepared.observe_changed(&scientist, &tracker.observe(&scientist, format), &flag)?;
            assert_eq!(prepared.work.history_visits - before.history_visits, 1);
            assert_eq!(prepared.work.emitted - before.emitted, 1);
            assert!(prepared.work.row_visits - before.row_visits <= count);
            Ok(work)
        })
        .unwrap(),
    );
}

#[test]
fn project_lexical_warning_revisions_match_oracle_after_two_commits_resume_and_append() {
    use dashboard_base::prepared::observer_tests::assert_reply_parity;
    use dashboard_base::test_fixtures::continuation_records;
    engine::executable_tables::initialize_for_tests().unwrap();
    let folder = super::super::tests::TempRoot::new("project-lexical-revisions");
    super::super::history_tests::content(&folder.0, "First project");
    std::fs::write(
        folder.0.join("common/special_projects/projects/mod.txt"),
        "mod_project={specialization=mod_kind} mod_later={specialization=mod_kind}",
    )
    .unwrap();
    std::fs::write(
        folder.0.join("localisation/later_l_english.yml"),
        "\u{feff}l_english:\n mod_later:0 \"Later project\"\n",
    )
    .unwrap();
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut current = frame(hour, true, 1);
            *current.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    replay::rows::LexicalTokenRow {
                        token: 8,
                        literal: if hour < 103 {
                            b"mod_project".to_vec()
                        } else {
                            b"mod_later".to_vec()
                        },
                    },
                    replay::rows::LexicalTokenRow {
                        token: 999,
                        literal: format!("unused {hour}").into_bytes(),
                    },
                ]));
            current
        })
        .collect();
    let (first, context) =
        dashboard_base::test_fixtures::source_with_content(frames[..2].to_vec(), &folder.0);
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(first.replay_ref(), 1);
    let fixture = Fixture::new();
    for current in decoded(&first) {
        prepared.observe(&first, &context, &current, &flag).unwrap();
    }
    publish(&fixture, &mut prepared);
    compare(&fixture, &first, &params("opaque-1"), &context);
    let second = super::super::history_tests::source(
        first
            .replay_ref()
            .append(&[continuation_records(
                first.replay_ref(),
                1,
                frames[2..4].to_vec(),
            )])
            .unwrap(),
        "lexical-second",
    );
    let accepted = decoded(&second);
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.restore_views(&accepted[1]);
    for current in &accepted[2..] {
        prepared.observe(&second, &context, current, &flag).unwrap();
    }
    publish(&fixture, &mut prepared);
    let actual = compare(&fixture, &second, &params("opaque-1"), &context);
    assert!(
        !actual.warnings.is_empty(),
        "the fixture proves counted warning reset parity"
    );
    let appended = super::super::history_tests::source(
        second
            .replay_ref()
            .append(&[continuation_records(
                second.replay_ref(),
                1,
                frames[4..].to_vec(),
            )])
            .unwrap(),
        "lexical-append",
    );
    prepared
        .observe(
            &appended,
            &context,
            decoded(&appended).last().unwrap(),
            &flag,
        )
        .unwrap();
    publish(&fixture, &mut prepared);
    let actual = complete_reply(&fixture.store, &fixture.key, &params("opaque-1"), &flag)
        .unwrap()
        .unwrap();
    let expected = SpecialProjects::load(&appended, &params("opaque-1"), &context, &flag).unwrap();
    assert_reply_parity(&actual, &expected).unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    assert_eq!(
        reader
            .event_count_through(prepared.keys["opaque-1"], i32::MAX)
            .unwrap(),
        2
    );
    assert_eq!(reader.runs(LEXICAL, 100, 104).unwrap().len(), 5);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: 720 accepted frames and 5,000 countries"
)]
fn project_unrelated_lexical_changes_publish_one_country_payload_over_720_hours_and_report_size() {
    let initial = frame(100, true, 5000);
    let source = source(std::slice::from_ref(&initial));
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for hour in 100..820 {
        let mut current = initial.clone();
        current.hour = hour;
        if let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut *current.tables[TableId::LexicalTokens.index()]
        {
            rows.push(replay::rows::LexicalTokenRow {
                token: 999,
                literal: format!("unused {hour}").into_bytes(),
            });
        }
        prepared
            .observe(&source, &context, &current, &flag)
            .unwrap();
    }
    publish(&fixture, &mut prepared);
    let bytes: u64 = (0..fixture
        .store
        .range_count(&fixture.key, &section(1))
        .unwrap()
        .unwrap())
        .map(|ordinal| {
            fixture
                .store
                .read(&fixture.key, &section(1), ordinal)
                .unwrap()
                .unwrap()
                .len()
        })
        .sum();
    let continuation = serde_json::to_vec(&prepared).unwrap().len();
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    for tag in ["opaque-1", "opaque-2500", "opaque-5000"] {
        assert_eq!(
            reader
                .event_count_through(prepared.keys[tag], i32::MAX)
                .unwrap(),
            1
        );
        assert_eq!(
            reader
                .value_at(prepared.keys[tag], 819)
                .unwrap()
                .unwrap()
                .first_hour,
            100
        );
    }
    assert_eq!(prepared.work.emitted, 5000);
    eprintln!(
        "special projects, 5,000 countries/720 h: section {bytes} bytes, continuation {continuation} bytes; country payloads 5,000 instead of 3,600,000 global-revision publications"
    );
}

#[test]
fn project_content_success_and_failure_are_bound_before_observation_and_shared_per_source() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker, observer_tests::unchanged_frames,
    };
    engine::executable_tables::initialize_for_tests().unwrap();
    let folder = super::super::tests::TempRoot::new("project-configure-once");
    super::super::history_tests::content(&folder.0, "Plain project");
    let initial = frame(100, true, 1);
    let (content_source, content_context) =
        dashboard_base::test_fixtures::source_with_content(vec![initial.clone()], &folder.0);
    let missing_source = source(std::slice::from_ref(&initial));
    let flag = AtomicBool::new(false);
    for (source, context, success) in [
        (&content_source, &content_context, true),
        (&missing_source, &LoadContext { content: None }, false),
    ] {
        let mut prepared = Prepared::new(source.replay_ref(), 1);
        prepared.configure(source, context).unwrap();
        assert_eq!(prepared.content.is_some(), success);
        assert_eq!(prepared.work.content_loads, 1);
        let mut tracker = FrameChangeTracker::new();
        prepared
            .observe_changed(
                &initial,
                &tracker.observe(&initial, source.replay_ref().format),
                &flag,
            )
            .unwrap();
        let work = unchanged_frames(
            prepared.work,
            &initial,
            20,
            source.replay_ref().format,
            |frame, changes| {
                prepared.observe_changed(frame, changes, &flag)?;
                Ok(prepared.work)
            },
        )
        .unwrap();
        assert_eq!(work.content_loads, 0);
        let mut second = Prepared::new(source.replay_ref(), 1);
        second.configure(source, context).unwrap();
        assert_eq!(second.content.is_some(), success);
        assert_eq!(
            second.work.content_loads, 0,
            "the typed source binding shares success and named failure"
        );
        assert_eq!(
            second.metadata.content_error,
            prepared.metadata.content_error
        );
    }
}

#[test]
fn project_history_change_revisits_one_country_without_reading_unchanged_project_or_holder_tables()
{
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{Work, assert_flat, scaled_work},
    };
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    assert_flat(
        scaled_work(500, |count| {
            let initial = frame(100, true, count as u32);
            let source = source(std::slice::from_ref(&initial));
            let mut prepared = Prepared::new(source.replay_ref(), 1);
            prepared.configure(&source, &context)?;
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(
                &initial,
                &tracker.observe(&initial, source.replay_ref().format),
                &flag,
            )?;
            let before = prepared.work;
            let mut changed = initial.clone();
            changed.hour = 101;
            *changed.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                    owner: 1,
                    project: 1,
                    ordinal: 0,
                    date: 100,
                    ..Default::default()
                }]));
            prepared.observe_changed(
                &changed,
                &tracker.observe(&changed, source.replay_ref().format),
                &flag,
            )?;
            let work = Work {
                row_visits: prepared.work.row_visits - before.row_visits,
                history_visits: prepared.work.history_visits - before.history_visits,
                emitted: prepared.work.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(
                (work.row_visits, work.history_visits, work.emitted),
                (1, 1, 1)
            );
            assert_eq!(
                prepared.countries["opaque-1"]
                    .current
                    .as_ref()
                    .unwrap()
                    .projects
                    .as_ref()
                    .unwrap()[0]
                    .0
                    .iterations,
                Some(1)
            );
            assert_eq!(
                prepared.countries[&format!("opaque-{count}")].event_hour,
                Some(100)
            );
            Ok(work)
        })
        .unwrap(),
    );
}

#[test]
fn project_history_only_spans_preserve_branch_diagnostic_rates_for_unchanged_unknown_projects() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut first = frame(100, true, 2);
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *first.tables[TableId::Projects.index()]
    {
        rows[1].state.current_offset = 0xdead;
    }
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 1,
            project: 1,
            ordinal: 0,
            date: 100,
            ..Default::default()
        }]));
    let source = source(&[first, second]);
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    prepared.configure(&source, &context).unwrap();
    for current in &accepted {
        prepared
            .observe_changed(
                current,
                &tracker.observe(current, source.replay_ref().format),
                &flag,
            )
            .unwrap();
    }
    publish(&fixture, &mut prepared);
    compare(&fixture, &source, &params("opaque-1"), &context);
    compare(&fixture, &source, &params("opaque-2"), &context);
    let (_, metadata) = completion_rows(&fixture);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let reason = "Project is in unsupported native state 0xdead";
    assert_eq!(
        reader
            .values_at(&[metadata.refusal_keys[reason]], 101)
            .unwrap(),
        [Some(2)]
    );
}

#[test]
fn project_holder_change_reuses_unchanged_project_history_link_and_program_tables() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let first = frame(100, true, 2);
    let mut renamed = first.clone();
    renamed.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed-holder".to_vec();
    }
    let source = source(&[first, renamed]);
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    prepared.configure(&source, &context).unwrap();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    prepared
        .observe_changed(
            &accepted[0],
            &tracker.observe(&accepted[0], source.replay_ref().format),
            &flag,
        )
        .unwrap();
    let before = prepared.work;
    prepared
        .observe_changed(
            &accepted[1],
            &tracker.observe(&accepted[1], source.replay_ref().format),
            &flag,
        )
        .unwrap();
    assert_eq!(prepared.work.row_visits - before.row_visits, 2);
    publish(&fixture, &mut prepared);
    for tag in ["opaque-1", "opaque-2", "renamed-holder"] {
        compare(&fixture, &source, &params(tag), &context);
    }
}

#[test]
fn prepared_project_events_name_each_presentation_once() {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut frames = vec![
        frame(100, true, 3),
        frame(101, true, 3),
        frame(102, true, 3),
    ];
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *frames[1].tables[TableId::Projects.index()]
    {
        rows[0].state.current_offset = 0xe8;
        rows[0].state.not_completed = 0;
    }
    frames[2] = frames[1].clone();
    frames[2].hour = 102;
    let source = source(&frames);
    let accepted = decoded(&source);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    for frame in &accepted {
        prepared.observe(&source, &context, frame, &flag).unwrap();
    }
    publish(&fixture, &mut prepared);
    for tag in ["opaque-1", "opaque-2", "opaque-3"] {
        compare(&fixture, &source, &params(tag), &context);
    }
    let (keys, _) =
        series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, "special-projects", 1)
            .unwrap()
            .unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, section(1), &flag).unwrap();
    let mut infos = BTreeSet::new();
    let mut events = 0;
    for tag in ["opaque-1", "opaque-2", "opaque-3"] {
        for row in reader
            .events(&[keys[tag]], i32::MIN, i32::MAX, 0, 16)
            .unwrap()
            .rows
        {
            events += 1;
            for project in row.row["projects"].as_array().unwrap() {
                // Old events repeated the whole presentation in every row.
                let info = project[1]["info"].as_u64().expect("a presentation key");
                assert!(project[1]["person"].is_u64());
                infos.insert(info);
            }
        }
    }
    assert_eq!(
        events, 4,
        "opaque-1's completion republishes its observation"
    );
    assert_eq!(infos.len(), 1, "equal presentations share one key");
    let info = *infos.first().unwrap();
    assert_eq!(reader.event_count_through(info, i32::MAX).unwrap(), 1);
}

/// Frames over `count` holders, each with one project: one holder's
/// prototype progress moves every hour (holder `(hour - 100) % count + 1`),
/// and the last holder's project is in an unsupported native state, whose
/// warning every span counts. Holder 3 completes at 110 with its date
/// recorded; holder 5's project is removed at 115; holder 7 starts a second
/// project at 118; the table is not read at 120, where holder 21's progress
/// moves unread; holder 2 completes at 125 with its date recorded in the
/// same hour.
fn owner_change_frames(count: u32) -> Vec<DecodedFrame> {
    (100..=131)
        .map(|hour| {
            let mut frame = frame(hour, hour != 120, count);
            let step = hour - 100;
            let mut history = Vec::new();
            if let TableView::Complete(TableRows::Projects(rows)) =
                &mut *frame.tables[TableId::Projects.index()]
            {
                for row in rows.iter_mut() {
                    let first = row.owner as i32 - 1;
                    if step >= first {
                        row.state.prototype_progress += (step - first) / count as i32 + 1;
                    }
                    if row.owner == count {
                        row.state.current_offset = 0xdead;
                    }
                }
                for (owner, at) in [(3, 110), (2, 125)] {
                    if hour >= at {
                        let row = rows.iter_mut().find(|row| row.owner == owner).unwrap();
                        row.state.current_offset = 0xe8;
                        row.state.not_completed = 0;
                        history.push(ProjectHistoryRow {
                            owner,
                            project: owner,
                            ordinal: 0,
                            prototype_reward_handle: COMPLETION_MARKER,
                            option_handle: COMPLETION_MARKER,
                            date: at,
                            ..Default::default()
                        });
                    }
                }
                if hour >= 115 {
                    rows.retain(|row| row.owner != 5);
                }
                if hour >= 118 {
                    rows.push(ProjectRow {
                        owner: 7,
                        local_id: 1007,
                        definition_id: 8,
                        started: 1,
                        program_reference: (7_u64 << 32) | 83,
                        state: replay::rows::ProjectState {
                            current_offset: 0x18,
                            not_completed: 1,
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                    rows.sort_by_key(|row| (row.owner, row.local_id));
                }
            }
            history.sort_by_key(|row| (row.owner, row.project, row.ordinal));
            frame.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(history)).into();
            frame
        })
        .collect()
}

/// The tables the projects' observer reads without content.
const OBSERVED_TABLES: [TableId; 10] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Projects,
    TableId::ProjectPrograms,
    TableId::ProjectHistory,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// Observe every block of `source` read alone, hour by hour, as preparation
/// does: with each block's recorded updates when `updates`, so a Projects
/// change can project its changed owners alone, or without them, so every
/// Projects change projects the whole table.
fn observe_blocks(source: &Source, prepared: &mut Prepared, updates: bool) {
    let replay = source.replay_ref();
    let format = replay.format;
    let flag = AtomicBool::new(false);
    let mut tables = OBSERVED_TABLES.to_vec();
    tables.sort_unstable();
    for index in &replay.index {
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        replay
            .read_block_alone(
                index,
                &tables,
                |raw, frame| {
                    let changes = if updates {
                        tracker.observe_updates(frame, Arc::new(raw), format)
                    } else {
                        tracker.observe(frame, format)
                    };
                    prepared.observe_changed(frame, &changes, &flag)
                },
                || Ok(()),
            )
            .unwrap();
    }
}

/// A Projects change recorded as an update projects only the owners whose
/// rows it changed and observes only their countries; every other holder
/// repeats the warnings observing it again would count. It publishes and
/// saves exactly what projecting the whole table does (which the oracle
/// confirms), through completions with their dates, a removed and an added
/// project, an unread hour whose holder's completions fall behind (the next
/// change takes the whole table), a history change in the same hour as a
/// project change, and an unsupported state counted at every span.
#[test]
fn project_changes_project_only_changed_owners_and_publish_what_the_whole_table_does() {
    let context = LoadContext { content: None };
    let count = 40;
    let source = source(&owner_change_frames(count));
    let run = |updates: bool| {
        let mut prepared = Prepared::new(source.replay_ref(), 1);
        prepared.configure(&source, &context).unwrap();
        observe_blocks(&source, &mut prepared, updates);
        let fixture = Fixture::new();
        publish(&fixture, &mut prepared);
        (prepared, fixture)
    };
    let (whole, whole_fixture) = run(false);
    let (owners, owners_fixture) = run(true);
    assert_eq!(
        serde_json::to_value(&owners).unwrap(),
        serde_json::to_value(&whole).unwrap(),
        "the saved state"
    );
    for id in [1, 2, 3, 5, 7, 21, count] {
        let params = params(&format!("opaque-{id}"));
        let actual = compare(&owners_fixture, &source, &params, &context);
        let expected = compare(&whole_fixture, &source, &params, &context);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "opaque-{id}"
        );
    }
    let (rows, metadata) = completion_rows(&owners_fixture);
    let (expected_rows, expected_metadata) = completion_rows(&whole_fixture);
    assert_eq!(rows, expected_rows, "completion changes");
    assert_eq!(
        serde_json::to_value(&metadata).unwrap(),
        serde_json::to_value(&expected_metadata).unwrap(),
        "completion metadata"
    );
    let reason = "Project is in unsupported native state 0xdead";
    assert!(metadata.refusal_keys.contains_key(reason));
    let keys: Vec<_> = metadata.refusal_keys.values().copied().collect();
    let flag = AtomicBool::new(false);
    let mut actual = Reader::new(
        &owners_fixture.store,
        &owners_fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    let mut expected = Reader::new(
        &whole_fixture.store,
        &whole_fixture.key,
        completion_stream::section(1),
        &flag,
    )
    .unwrap();
    for hour in 100..=131 {
        assert_eq!(
            actual.values_at(&keys, hour).unwrap(),
            expected.values_at(&keys, hour).unwrap(),
            "refusal counts at {hour}"
        );
    }
    assert!(
        owners.work.row_visits * 4 < whole.work.row_visits,
        "changed owners alone: {} rows visited, {} by the whole table",
        owners.work.row_visits,
        whole.work.row_visits
    );
    assert_eq!(owners.work.emitted, whole.work.emitted);
}

/// At 2,000 holders, an hour where one owner's progress moves visits that
/// owner's row alone and emits its country alone.
#[test]
fn project_change_of_one_owner_visits_its_rows_alone_at_2000_holders() {
    let context = LoadContext { content: None };
    let count = 2_000;
    let first = frame(100, true, count);
    let mut moved = first.clone();
    moved.hour = 101;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *moved.tables[TableId::Projects.index()]
    {
        rows[count as usize / 2].state.prototype_progress += 1;
    }
    let source = source(&[first, moved]);
    let mut prepared = Prepared::new(source.replay_ref(), 1);
    prepared.configure(&source, &context).unwrap();
    let replay = source.replay_ref();
    let format = replay.format;
    let flag = AtomicBool::new(false);
    let mut tables = OBSERVED_TABLES.to_vec();
    tables.sort_unstable();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    let mut before = None;
    replay
        .read_block_alone(
            &replay.index[0],
            &tables,
            |raw, frame| {
                if frame.hour == 101 {
                    before = Some(prepared.work);
                }
                let changes = tracker.observe_updates(frame, Arc::new(raw), format);
                prepared.observe_changed(frame, &changes, &flag)
            },
            || Ok(()),
        )
        .unwrap();
    let before = before.expect("the second hour");
    assert_eq!(prepared.work.row_visits - before.row_visits, 1);
    assert_eq!(prepared.work.emitted - before.emitted, 1);
}

/// Holders 1 and 2 observed at 100; a recorded update changes only holder
/// 1's progress at 101 and again at 103, and nothing at 102. Projecting the
/// changed owner alone skips holder 2 at 101 and 103, yet it saves what
/// projecting the whole table saves: holder 2's last observation at 103 and
/// its completions' compact run started again at 103 after the hour none
/// observed, with the same completion changes and refusal counts.
#[test]
fn skipped_holders_save_what_observing_them_again_saves() {
    let context = LoadContext { content: None };
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = frame(hour, true, 2);
            if let TableView::Complete(TableRows::Projects(rows)) =
                &mut *frame.tables[TableId::Projects.index()]
            {
                rows[0].state.prototype_progress += match hour {
                    100 => 0,
                    101 | 102 => 1,
                    _ => 2,
                };
            }
            frame
        })
        .collect();
    let source = source(&frames);
    let run = |updates: bool| {
        let mut prepared = Prepared::new(source.replay_ref(), 1);
        prepared.configure(&source, &context).unwrap();
        observe_blocks(&source, &mut prepared, updates);
        let fixture = Fixture::new();
        publish(&fixture, &mut prepared);
        (prepared, fixture)
    };
    let (whole, whole_fixture) = run(false);
    let (owners, owners_fixture) = run(true);
    assert!(
        owners.work.row_visits < whole.work.row_visits,
        "holder 2 was skipped"
    );
    assert_eq!(
        whole.countries["opaque-2"].last.map(|(hour, _)| hour),
        Some(103)
    );
    assert_eq!(
        serde_json::to_value(&owners).unwrap(),
        serde_json::to_value(&whole).unwrap(),
        "the saved state"
    );
    let (rows, metadata) = completion_rows(&owners_fixture);
    let (expected_rows, expected_metadata) = completion_rows(&whole_fixture);
    assert_eq!(rows, expected_rows);
    assert_eq!(
        serde_json::to_value(&metadata).unwrap(),
        serde_json::to_value(&expected_metadata).unwrap()
    );
}

#[test]
fn current_cards_select_the_exact_holder_before_identity_preparation() {
    let memory = source(&[frame(100, true, 1)]);
    let (source, root) = dashboard_base::test_fixtures::file_source(
        replay::container::Replay::open(memory.replay_ref().bytes().to_vec()).unwrap(),
    );
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let mut request = params("opaque-1");
    request.selection_tags = vec!["opaque-1".into()];
    let selected = frame_reply(&source, &context, &request, 100, &flag).unwrap();
    assert!(
        !selected.projects.is_empty(),
        "the exact frame supplies the selected new holder"
    );
    request.selection_tags = vec!["unseen holder".into()];
    let unselected = frame_reply(&source, &context, &request, 100, &flag).unwrap();
    assert!(unselected.projects.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn equal_project_read_markers_carry_without_numeric_publication() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let format = replay::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut input = frame(100, true, 64);
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe_changed(&input, &tracker.observe(&input, format), &flag)
        .unwrap();
    let visits = prepared.values.key_visits;
    input.hour += 1;
    prepared
        .observe_changed(&input, &tracker.observe(&input, format), &flag)
        .unwrap();
    assert_eq!(prepared.values.key_visits, visits);
    prepared.values.seal();
    assert_eq!(
        prepared.values.state(0).unwrap().last.unwrap().last_hour,
        101
    );
}
