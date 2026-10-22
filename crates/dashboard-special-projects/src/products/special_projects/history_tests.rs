use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay::container::Replay;
use replay_data::{
    rows::CountryRow, rows::LexicalTokenRow, rows::ProjectProgramRow, rows::ScriptVariableEntry,
    rows::ScriptVariablesRow, rows::TableRows,
};
use replay_frame::frame::TableView;

pub(super) fn frame(hour: i32, read: bool, countries: u32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, None);
    for table in [
        TableId::ProjectHistory,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
    ] {
        frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
    }
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id - 1,
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
                canonical_slot: id - 1,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
        (1..=countries)
            .map(|owner| ProjectRow {
                runtime: replay_data::rows::Recorded::Value(
                    replay_data::rows::ProjectRuntime::default(),
                ),
                owner,
                local_id: owner,
                definition_id: 8,
                started: 1,
                program_reference: (u64::from(owner) << 32) | 83,
                state: replay_data::rows::ProjectState {
                    current_offset: 0x18,
                    not_completed: 1,
                    prototype_progress: (owner % 100) as i32,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(
            (1..=countries)
                .map(|id| ProjectProgramRow {
                    assignments: replay_data::rows::Recorded::Value(Some(Vec::new())),
                    id,
                    owner: id,
                    scientist: 0,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]))
        .into();
    frame.metrics.table_ns[TableId::Projects.index()] = u64::from(read);
    frame
}
pub(super) fn source(replay: Replay, revision: &str) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "projects-shared".into(),
            revision: revision.into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn params(id: u32) -> SpecialProjectsParams {
    SpecialProjectsParams {
        branch_id: "1".into(),
        tag: format!("opaque-{id}"),
        selection_tags: Vec::new(),
        hour: None,
    }
}
fn decoded(source: &Source) -> usize {
    source
        .state::<history::Scans>()
        .lock()
        .unwrap()
        .decoded_blocks
}

#[test]
fn hourly_country_balances_do_not_retain_hourly_country_snapshots() {
    let frames = (100..108)
        .map(|hour| {
            let mut frame = frame(hour, hour == 100, 12);
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                for country in countries {
                    country.political_power_raw = Some(i64::from(hour));
                }
            }
            frame
        })
        .collect();
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap(),
        "1",
    );
    let scan = history::cached(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scan.spans.len(), 1);
    assert_eq!(
        (scan.spans[0].first_hour, scan.spans[0].last_hour),
        (100, 107)
    );
}

#[test]
fn spans_keep_only_the_person_rows_a_projection_reads() {
    use replay_data::rows::CharacterRow;
    use replay_frame::frame::SharedView;
    // Program 1 names scientist 73 of a 100-character roster read anew each hour.
    let frames: Vec<_> = (100..104)
        .map(|hour| {
            let mut frame = frame(hour, hour == 100, 1);
            frame.tables[TableId::ProjectPrograms.index()] =
                TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                    id: 1,
                    owner: 1,
                    scientist: (73_u64 << 32)
                        | u64::from(engine_native::ccharacter::REFERENCE_TYPE),
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
                (1..=100)
                    .map(|id| CharacterRow {
                        id,
                        ordinal: if id == 100 { hour as u32 } else { 0 },
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
            frame
        })
        .collect();
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap(),
        "1",
    );
    let cancelled = AtomicBool::new(false);
    let scan = history::cached(&source, 1, &cancelled).unwrap();
    // Each roster read still starts a span, but no span keeps the roster.
    assert_eq!(scan.spans.len(), 4);
    let views: Vec<_> = scan
        .spans
        .iter()
        .map(|span| span.inputs.world_frame(1).tables[TableId::Characters.index()].clone())
        .collect();
    assert_eq!(
        *views[0],
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 73,
            ..Default::default()
        }]))
    );
    assert!(views.iter().all(|view| SharedView::ptr_eq(view, &views[0])));
    let reply = project(&scan, &params(1).country(), None, None, &cancelled)
        .unwrap()
        .0;
    let mut expected = Builder::default();
    for frame in &frames {
        expected.observe(frame, &params(1).tag, None);
    }
    assert_eq!(reply.people, expected.people);
    assert_eq!(reply.people[0].id, Some(73));
}

#[test]
fn verified_append_shares_prior_inputs_and_decodes_only_new_blocks() {
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100, true, 12)])]).unwrap();
    let next = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(124, true, 12)],
        )])
        .unwrap();
    let first = source(first, "1");
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    SpecialProjects::load(&first, &params(12), &context, &cancelled).unwrap();
    let old = history::cached(&first, 1, &cancelled).unwrap();
    let next = source(next, "2");
    next.inherit_append_states(&first);
    let reply = SpecialProjects::load(&next, &params(12), &context, &cancelled).unwrap();
    let extended = history::cached(&next, 1, &cancelled).unwrap();
    assert!(Arc::ptr_eq(&old.spans[0].inputs, &extended.spans[0].inputs));
    assert_eq!(decoded(&next), next.replay_ref().index.len());
    assert_eq!(old.spans.len(), 1);
    assert_eq!(
        reply.projects[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (124, 124)]
    );
    assert_eq!(
        SpecialProjects::load(&first, &params(12), &context, &cancelled)
            .unwrap()
            .available
            .last()
            .unwrap()
            .last_hour,
        100
    );
}

pub(super) fn content(root: &std::path::Path, project_name: &str) {
    for (path, body) in [
        ("hoi4.exe", "test".to_owned()),
        ("user/settings.txt", "language=l_english force_pow2_textures=no".to_owned()),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#.to_owned()),
        ("common/defines/mod.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} }, NBuildings = { MAX_BUILDING_LEVELS = 15, INFRASTRUCTURE_RESOURCE_BONUS = 0, SUPPLY_ROUTE_RESOURCE_BONUS = 0 } }".to_owned()),
        ("common/special_projects/projects/mod.txt", "mod_project={specialization=mod_kind}".to_owned()),
        ("common/special_projects/specialization/mod.txt", "mod_kind={color={1 2 3}}".to_owned()),
        ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp".to_owned()),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n".to_owned()),
        ("common/scripted_localisation/mod.txt", "entry={name=ProjectStage text={trigger={check_variable={project_flag=1}} localization_key=ACTIVE} text={localization_key=WAITING}}".to_owned()),
        ("localisation/mod_l_english.yml", format!("\u{feff}l_english:\n mod_project:0 \"{project_name}\"\n mod_kind:0 \"Mod science\"\n ACTIVE:0 \"Active\"\n WAITING:0 \"Waiting\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Count $NUMBER$\"\n")),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
}

#[test]
fn scoped_world_variable_names_share_one_pass_and_failures_belong_only_to_the_requested_actor() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let folder = tests::TempRoot::new("shared-scope");
    content(&folder.0, "[ROOT.ProjectStage]");
    let localisation = folder.0.join("localisation/mod_l_english.yml");
    let mut text = std::fs::read_to_string(&localisation).unwrap();
    text.push_str(" opaque-12_mod_project:0 \"[ROOT.MissingProjectProperty]\"\n");
    std::fs::write(localisation, text).unwrap();
    // An absent scripted property is empty text; use a reached flag lookup
    // whose recorded table is missing for this actor.
    std::fs::write(
        folder.0.join("common/scripted_localisation/refusal.txt"),
        "entry={name=MissingProjectProperty text={trigger={has_country_flag=mod_project} localization_key=ACTIVE}}",
    )
    .unwrap();
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let frames: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = frame(hour, hour == 100, 12);
            // Tag text uses the reserved tag at slot zero.
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            for country in countries {
                country.canonical_slot = country.id;
            }
            let TableView::Complete(TableRows::CountryLinks(links)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            for link in links {
                link.canonical_slot = link.id;
            }
            // Scripted text's strict World needs complete core scope tables.
            frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![replay_data::rows::StateRow {
                    id: 1,
                    owner: 1,
                    controller: 1,
                    ..Default::default()
                }]))
                .into();
            frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![replay_data::rows::ProvinceRow {
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
                        seed: replay_data::rows::Recorded::Value(None),
                        entries: vec![],
                    })
                    .chain((1..=12).map(|id| ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id: id,
                        seed: replay_data::rows::Recorded::Value(None),
                        entries: vec![ScriptVariableEntry {
                            name: b"project_flag".to_vec(),
                            value_raw: if hour == 100 { 0 } else { 100_000 },
                        }],
                    }))
                    .collect(),
                ))
                .into();
            frame
        })
        .collect();
    let mut records = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records]).unwrap(),
        "1",
    );
    let context = LoadContext {
        content: Some(options),
    };
    let cancelled = AtomicBool::new(false);
    for id in 1..=12 {
        let reply = SpecialProjects::load(&source, &params(id), &context, &cancelled).unwrap();
        let runs = &reply.projects[0].runs;
        if id < 12 {
            assert_eq!(
                reply.presentations[runs[0].presentation as usize].name,
                "Waiting"
            );
            assert_eq!(
                reply.presentations[runs[1].presentation as usize].name,
                "Active"
            );
            assert!(
                !reply
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("MissingProjectProperty"))
            );
        } else {
            assert!(
                reply
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("MissingProjectProperty"))
            );
        }
    }
    assert_eq!(decoded(&source), source.replay_ref().index.len());
    let cache = source.state::<Cache>();
    let cache = cache.lock().unwrap();
    assert_eq!(
        cache.content.as_ref().unwrap().scoped[0].1.decoded_blocks,
        source.replay_ref().index.len()
    );
}

#[test]
fn content_readiness_changes_rename_cached_data_without_another_recording_scan() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let folder = tests::TempRoot::new("shared-ready");
    content(&folder.0, "Prepared project");
    let path = folder.0.join("localisation/mod_l_english.yml");
    let mut localisation = std::fs::read_to_string(&path).unwrap();
    localisation.push_str(" opaque-2_mod_project:0 \"\"\n");
    std::fs::write(path, localisation).unwrap();
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let mut records = records(1, vec![frame(100, true, 12)]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records]).unwrap(),
        "1",
    );
    let cancelled = AtomicBool::new(false);
    let missing = SpecialProjects::load(
        &source,
        &params(1),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    let context = LoadContext {
        content: Some(options),
    };
    let ready = SpecialProjects::load(&source, &params(1), &context, &cancelled).unwrap();
    assert_eq!(missing.presentations[0].name, "Project name unavailable");
    assert_eq!(ready.presentations[0].name, "Prepared project");
    assert_eq!(
        SpecialProjects::load(&source, &params(2), &context, &cancelled)
            .unwrap()
            .presentations[0]
            .name,
        ""
    );
    assert_eq!(decoded(&source), source.replay_ref().index.len());
    let missing_again = SpecialProjects::load(
        &source,
        &params(1),
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        missing_again.presentations[0].name,
        "Project name unavailable"
    );
    assert_eq!(decoded(&source), source.replay_ref().index.len());
}

#[test]
fn scientist_name_refusals_stay_local_through_person_and_reply_caches() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{
        rows::CharacterPortrait, rows::CharacterPortraitRow, rows::CharacterRow,
        rows::PORTRAIT_LARGE,
    };

    let folder = tests::TempRoot::new("scientist-refusals");
    content(&folder.0, "Prepared project");
    std::fs::create_dir_all(folder.0.join("common/characters")).unwrap();
    std::fs::write(
        folder.0.join("common/characters/mod.txt"),
        concat!(
            "characters={mod_ambiguous={",
            "instance={allowed={always=yes} name=FIRST_NAME}",
            "instance={allowed={always=yes} name=SECOND_NAME}}",
            "mod_refused={name=SCIENTIST_AUTHORED}}",
        ),
    )
    .unwrap();
    let path = folder.0.join("localisation/mod_l_english.yml");
    let mut localisation = std::fs::read_to_string(&path).unwrap();
    localisation.push_str(concat!(
        " SCIENTIST_SAVED:0 \"Saved\u{a4}name\"\n",
        " SCIENTIST_AUTHORED:0 \"Authored\u{a4}name\"\n",
        " SCIENTIST_UNUSED:0 \"Unused\u{a4}name\"\n",
    ));
    std::fs::write(path, localisation).unwrap();
    std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
    let mut portrait = vec![0; 18];
    portrait[2] = 2;
    portrait[12] = 1;
    portrait[14] = 1;
    portrait[16] = 32;
    portrait[17] = 0x28;
    portrait.extend_from_slice(&[0, 0, 255, 255]);
    std::fs::write(folder.0.join("gfx/scientist.tga"), portrait).unwrap();
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = frame(hour, hour == 100, 20);
            frame.tables[TableId::ProjectPrograms.index()] =
                TableView::Complete(TableRows::ProjectPrograms(
                    (1..=20)
                        .map(|id| ProjectProgramRow {
                            id,
                            owner: id,
                            scientist: (u64::from(id) << 32) | 73,
                            ..Default::default()
                        })
                        .collect(),
                ))
                .into();
            frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
                (1..=20)
                    .map(|id| CharacterRow {
                        id,
                        country: id,
                        name_token_id: 1000 + id as i32,
                        token: match id {
                            1 => b"SCIENTIST_SAVED".to_vec(),
                            2 | 4 => vec![],
                            20 => b"SCIENTIST_UNUSED".to_vec(),
                            _ => format!("Scientist {id}").into_bytes(),
                        },
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
            frame.tables[TableId::CharacterPortraits.index()] =
                TableView::Complete(TableRows::CharacterPortraits(
                    (1..=20)
                        .map(|character| CharacterPortraitRow {
                            character,
                            portraits: vec![CharacterPortrait {
                                category: 4,
                                size: PORTRAIT_LARGE,
                                is_set: 1,
                                kind: 1,
                                identity: b"gfx/scientist.tga".to_vec(),
                            }],
                        })
                        .collect(),
                ))
                .into();
            frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    LexicalTokenRow {
                        token: 8,
                        literal: b"mod_project".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 1002,
                        literal: b"mod_ambiguous".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 1004,
                        literal: b"mod_refused".to_vec(),
                    },
                ]))
                .into();
            frame
        })
        .collect();
    let mut records = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records]).unwrap(),
        "1",
    );
    let context = LoadContext {
        content: Some(options),
    };
    let cancelled = AtomicBool::new(false);
    let unused = SpecialProjects::load(&source, &params(20), &context, &cancelled).unwrap();
    assert!(
        unused
            .warnings
            .iter()
            .any(|warning| warning.contains("SCIENTIST_UNUSED"))
    );
    let mut expected = Vec::new();
    for (id, subject) in [
        (1, "SCIENTIST_SAVED"),
        (2, "mod_ambiguous"),
        (4, "SCIENTIST_AUTHORED"),
    ] {
        let reply = SpecialProjects::load(&source, &params(id), &context, &cancelled).unwrap();
        assert_eq!(reply.people.len(), 1);
        assert_eq!(reply.people[0].name, None);
        assert!(reply.people[0].portrait.is_some());
        assert_eq!(reply.people[0].missing, None);
        let warnings: Vec<_> = reply
            .warnings
            .iter()
            .filter(|warning| warning.contains(subject))
            .collect();
        assert_eq!(warnings.len(), 1, "{:?}", reply.warnings);
        assert!(warnings[0].starts_with("1 Project texts not shown:"));
        assert!(
            !reply
                .warnings
                .iter()
                .any(|warning| warning.contains("SCIENTIST_UNUSED"))
        );
        assert_eq!(
            SpecialProjects::load(&source, &params(id), &context, &cancelled)
                .unwrap()
                .warnings,
            reply.warnings
        );
        expected.push((id, reply));
    }
    for id in 3..=19 {
        SpecialProjects::load(&source, &params(id), &context, &cancelled).unwrap();
    }
    assert!(
        !source
            .state::<Cache>()
            .lock()
            .unwrap()
            .replies
            .iter()
            .any(|((_, tag, _), _)| tag == &params(1).tag)
    );
    for (id, expected) in expected {
        let reply = SpecialProjects::load(&source, &params(id), &context, &cancelled).unwrap();
        assert_eq!(reply.warnings, expected.warnings);
        assert_eq!(reply.people, expected.people);
    }
    assert_eq!(decoded(&source), source.replay_ref().index.len());
}

#[test]
fn projection_preserves_incomplete_reads_native_identity_changes_and_cancellation() {
    let start = frame(100, true, 1);
    let mut incomplete = frame(101, false, 1);
    incomplete.tables[TableId::Projects.index()] = TableView::Unavailable {
        reason: "project walk failed".into(),
    }
    .into();
    incomplete.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 5001,
            canonical_slot: 0,
            tag: b"opaque-1".to_vec(),
            ..Default::default()
        }]))
        .into();
    incomplete.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 5001,
                canonical_slot: 0,
            },
        ]))
        .into();
    let frames = vec![start, incomplete, frame(102, false, 1), frame(124, true, 1)];
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap(),
        "1",
    );
    let cancelled = AtomicBool::new(false);
    let scan = history::cached(&source, 1, &cancelled).unwrap();
    let actual = project(&scan, &params(1).country(), None, None, &cancelled)
        .unwrap()
        .0;
    let mut expected = Builder::default();
    for frame in &frames {
        expected.observe(frame, &params(1).tag, None);
    }
    assert_eq!(
        serde_json::to_value(&actual.projects).unwrap(),
        serde_json::to_value(&expected.episodes).unwrap()
    );
    assert_eq!(actual.available, expected.available);
    assert!(
        !actual
            .available
            .iter()
            .any(|span| span.first_hour <= 101 && span.last_hour >= 101)
    );
    let before = decoded(&source);
    assert_eq!(
        history::cached(&source, 1, &AtomicBool::new(true))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        project(
            &scan,
            &params(1).country(),
            None,
            None,
            &AtomicBool::new(true)
        )
        .err()
        .unwrap()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(decoded(&source), before);
}

#[test]
fn projected_alias_requests_keep_all_literal_holders_but_not_unrelated_alias_reuse() {
    let mut frames = Vec::new();
    for (hour, tags) in [
        (100, ["X", "other", "third"]),
        (101, ["Y", "X", "third"]),
        (102, ["Y", "X", "third"]),
        (103, ["former", "X", "Y"]),
        (104, ["Y", "X", "third"]),
    ] {
        let mut sample = frame(hour, true, 3);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *sample.tables[TableId::Countries.index()]
        {
            for (row, tag) in rows.iter_mut().zip(tags) {
                row.tag = tag.as_bytes().to_vec();
            }
        }
        if hour == 102 {
            sample.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "Synthetic identity gap".into(),
            }
            .into();
        }
        frames.push(sample);
    }
    let source = source(
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap(),
        "1",
    );
    let context = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let request = |tag: &str, roots: &[&str]| SpecialProjectsParams {
        branch_id: "1".into(),
        tag: tag.into(),
        selection_tags: roots.iter().map(|root| (*root).to_owned()).collect(),
        hour: None,
    };
    let renamed =
        SpecialProjects::load(&source, &request("Y", &["X"]), &context, &cancelled).unwrap();
    assert_eq!(
        renamed
            .identities
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.slot, run.country_id))
            .collect::<Vec<_>>(),
        [(101, 101, 0, 1), (104, 104, 0, 1)]
    );
    assert_eq!(renamed.read_hours, [101, 104]);
    assert_eq!(renamed.projects.len(), 1);
    assert!(renamed.projects[0].id.starts_with("1:"));
    assert_eq!(
        renamed.projects[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(101, 101), (104, 104)]
    );
    let literal =
        SpecialProjects::load(&source, &request("X", &["X"]), &context, &cancelled).unwrap();
    assert_eq!(
        literal
            .identities
            .iter()
            .map(|run| run.country_id)
            .collect::<Vec<_>>(),
        [1, 2, 2]
    );
    assert_eq!(
        literal.projects.len(),
        2,
        "every holder of literal X is selected"
    );
    // Projecting Y is not itself a new literal choice. Conversely an actual Y
    // choice includes its early X alias, excluding unrelated holder 2 under X.
    let original_y =
        SpecialProjects::load(&source, &request("X", &["Y"]), &context, &cancelled).unwrap();
    assert_eq!(original_y.read_hours, [100]);
    assert_eq!(original_y.identities[0].country_id, 1);
    assert_eq!(original_y.projects.len(), 1);
    let unknown = SpecialProjects::load(
        &source,
        &request("Y", &["missing literal"]),
        &context,
        &cancelled,
    )
    .unwrap();
    assert!(unknown.identities.is_empty());
    assert!(unknown.available.is_empty());
    assert!(unknown.projects.is_empty());
    // The cache key must retain selection roots; a plain Y request includes
    // holder 3 at 103 and cannot reuse the narrower X-selected response.
    let unfiltered =
        SpecialProjects::load(&source, &request("Y", &[]), &context, &cancelled).unwrap();
    assert!(unfiltered.identities.iter().any(|run| run.country_id == 3));
    assert_eq!(decoded(&source), source.replay_ref().index.len());
}

#[test]
fn identity_metadata_is_budgeted_before_copying_into_the_complete_project_reply() {
    let cancelled = AtomicBool::new(false);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100, true, 1)])]).unwrap();
    let scan = history::BranchScan::scan(&replay, 1, &cancelled).unwrap();
    let mut reply = project(&scan, &params(1).country(), None, None, &cancelled)
        .unwrap()
        .0;
    let identity = CountryIdentityRun {
        first_hour: 100,
        last_hour: 100,
        slot: 0,
        country_id: 1,
    };
    append_identities(&mut reply, &[&identity]).unwrap();
    assert_eq!(reply.identities.as_slice(), std::slice::from_ref(&identity));
    reply.identities.clear();
    reply.warnings.push(String::new());
    let bytes = dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    *reply.warnings.last_mut().unwrap() = "x".repeat(MAX_PRODUCT_REPLY_BYTES - bytes);
    assert_eq!(
        dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES),
        Some(MAX_PRODUCT_REPLY_BYTES)
    );
    assert!(
        append_identities(&mut reply, &[&identity])
            .unwrap_err()
            .to_string()
            .contains("reply budget")
    );
    assert!(
        reply.identities.is_empty(),
        "oversized metadata is refused before copying"
    );
}
