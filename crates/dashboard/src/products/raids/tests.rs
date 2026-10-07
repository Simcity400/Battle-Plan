use super::*;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, source};
use replay::{
    frame::TableView,
    rows::{CountryAppearanceRow, LexicalTokenRow, RaidCountryRow, RaidTargetRow, TableRows},
};

fn raid(id: u32, phase: i32, outcome: i32) -> RaidRow {
    RaidRow {
        id,
        reference_type: native::REFERENCE_TYPE_VALUE,
        parent_slot: 7,
        phase,
        outcome,
        preparation_hours: 37,
        ..Default::default()
    }
}

fn frame(hour: i32, raids: Vec<RaidRow>) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::RaidCountries.index()] =
        TableView::Complete(TableRows::RaidCountries(vec![RaidCountryRow {
            canonical_slot: 7,
            country: 11,
            eligible_types: vec![50],
            ..Default::default()
        }]));
    *frame.tables[TableId::RaidTargets.index()] = TableView::Complete(TableRows::RaidTargets(
        raids
            .iter()
            .map(|raid| RaidTargetRow {
                id: raid.id,
                reference_type: raid.reference_type,
                raid_type: 50,
                scope: RaidScope {
                    state: 41,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(raids));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 50,
            literal: b"mod_raid".to_vec(),
        }]));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![replay::rows::StateRow {
            id: 41,
            owner: 11,
            controller: 11,
            custom_name: b"Recorded state".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![replay::rows::ProvinceRow {
            id: 101,
            state: 41,
            owner: 11,
            controller: 11,
            ..Default::default()
        }]));
    frame
}

#[test]
fn raids_native_transitions_count_observed_launch_once_and_native_success_only() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 2, 0)]), None)
        .unwrap();
    scan.observe(&frame(101, vec![raid(1, 4, 0)]), None)
        .unwrap();
    scan.observe(&frame(102, vec![raid(1, 5, 2)]), None)
        .unwrap();
    scan.observe(&frame(103, vec![raid(1, 5, 2), raid(2, 5, 3)]), None)
        .unwrap();
    scan.observe(
        &frame(
            104,
            vec![raid(1, 5, 2), raid(2, 5, 3), raid(3, 5, 4), raid(4, 5, 5)],
        ),
        None,
    )
    .unwrap();
    let result = scan.finish();
    let run = result.series["GER"].last().unwrap();
    assert_eq!(
        (run.launched, run.succeeded),
        (1, 2),
        "success can exceed observed launches; no clamping"
    );
    let terminal = &result.episodes[1];
    assert_eq!(terminal.start, RaidDate { hour: 103 });
    assert_eq!(
        terminal.runs[0].launched, None,
        "console End may bypass launch"
    );
    assert_eq!(terminal.runs[0].succeeded, Some(true));
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("terminal result") && line.contains("3 raid facts"))
    );
    assert_eq!(
        result.episodes[0].preparation[0],
        RaidPreparationRun(100, 104, 37, 0)
    );
}

#[test]
fn raids_nonzero_recorded_flight_distance_proves_between_read_launch_by_the_read() {
    let mut scan = Scan::new();
    scan.observe(
        &frame(100, (1..=7).map(|id| raid(id, 3, 0)).collect()),
        None,
    )
    .unwrap();
    let mut ended = raid(1, 5, 3);
    ended.travelled = Some(10);
    let mut canceled = raid(2, 5, 5);
    canceled.travelled = Some(-10);
    let mut zero = raid(3, 5, 3);
    zero.travelled = Some(0);
    let mut next = vec![ended, canceled, zero];
    next.extend(
        [i64::MIN, i64::MIN + 1, i64::MAX]
            .into_iter()
            .enumerate()
            .map(|(index, distance)| {
                let mut sentinel = raid(index as u32 + 4, 5, 5);
                sentinel.travelled = Some(distance);
                sentinel
            }),
    );
    let mut in_progress = raid(7, 4, 0);
    in_progress.travelled = Some(i64::MAX);
    next.push(in_progress);
    scan.observe(&frame(101, next), None).unwrap();
    let result = scan.finish();
    assert_eq!(result.series["GER"].last().unwrap().launched, 3);
    assert_eq!(result.series["GER"].last().unwrap().succeeded, 2);
    assert_eq!(result.episodes[0].runs[1].first_hour, 101);
    assert_eq!(result.episodes[0].runs[1].launched, Some(true));
    assert_eq!(
        result.episodes[1].runs[1].launched,
        Some(true),
        "negative mod speed also witnesses flight"
    );
    assert_eq!(
        result.episodes[2].runs[1].launched, None,
        "zero is not a flight witness"
    );
    for episode in &result.episodes[3..6] {
        assert_eq!(
            episode.runs[1].launched, None,
            "native non-finite sentinels do not prove flight"
        );
    }
    assert_eq!(
        result.episodes[6].runs[1].launched,
        Some(true),
        "recorded phase 4 independently proves launch"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("4 raid facts")
                && line.contains("non-finite recorded flight distance")
                && line.contains("mod_raid"))
    );
}

#[test]
fn raids_missing_country_rows_and_unread_hours_never_become_zero_or_bridge_gaps() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 4, 0)]), None)
        .unwrap();
    let mut missing = frame(101, vec![]);
    *missing.tables[TableId::RaidCountries.index()] =
        TableView::Complete(TableRows::RaidCountries(vec![]));
    scan.observe(&missing, None).unwrap();
    let mut unread = frame(102, vec![]);
    *unread.tables[TableId::Raids.index()] = TableView::Unavailable {
        reason: "synthetic unread".into(),
    };
    scan.observe(&unread, None).unwrap();
    scan.observe(&frame(105, vec![raid(1, 5, 3)]), None)
        .unwrap();
    let result = scan.finish();
    assert_eq!(
        result.series["GER"]
            .iter()
            .map(|r| (r.first_hour, r.last_hour, r.launched))
            .collect::<Vec<_>>(),
        [(100, 100, 1), (105, 105, 1)]
    );
    assert_eq!(result.read_hours, [100, 105]);
    assert_eq!(
        result.episodes[1].start.hour, 105,
        "a complete absence ends an episode; a gap never invents an exact date"
    );
}

#[test]
fn raids_target_controller_uses_native_scope_priority_including_captive_leader() {
    let frame = frame(100, vec![]);
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                building_type: 90,
                building_country: 99,
                province: 101,
                ..Default::default()
            }
        ),
        Ok(Some(99))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                building_type: 90,
                building_flags: 1,
                building_location: 101,
                leader_country: Some(99),
                ..Default::default()
            }
        ),
        Ok(Some(11))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                province: 101,
                state: 41,
                leader_country: Some(99),
                ..Default::default()
            }
        ),
        Ok(Some(11))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                state: 41,
                leader_country: Some(99),
                ..Default::default()
            }
        ),
        Ok(Some(11))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                leader_country: Some(99),
                leader_province: 101,
                ..Default::default()
            }
        ),
        Ok(Some(99))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                leader_country: Some(-1),
                leader_province: 101,
                ..Default::default()
            }
        ),
        Ok(Some(u32::MAX))
    );
    assert_eq!(
        target_country(
            &frame,
            &RaidScope {
                leader_province: 101,
                ..Default::default()
            }
        ),
        Ok(Some(11))
    );
    assert!(
        target_country(
            &frame,
            &RaidScope {
                province: 404,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn raids_five_thousand_countries_all_enter_scan_and_reply_budget_includes_metadata() {
    let mut frame = scale_frame(1, 100, 5_000);
    *frame.tables[TableId::RaidCountries.index()] = TableView::Complete(TableRows::RaidCountries(
        (0..5_000)
            .map(|slot| RaidCountryRow {
                canonical_slot: slot,
                country: (100_000 + slot) as i32,
                eligible_types: vec![50],
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![RaidRow {
        id: 1,
        reference_type: native::REFERENCE_TYPE_VALUE,
        parent_slot: 4_999,
        phase: 4,
        ..Default::default()
    }]));
    *frame.tables[TableId::RaidTargets.index()] =
        TableView::Complete(TableRows::RaidTargets(vec![RaidTargetRow {
            id: 1,
            reference_type: native::REFERENCE_TYPE_VALUE,
            raid_type: 50,
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 50,
            literal: b"mod_raid".to_vec(),
        }]));
    let mut scan = Scan::new();
    scan.observe(&frame, None).unwrap();
    frame.hour = 101;
    scan.observe(&frame, None).unwrap();
    let result = scan.finish();
    assert_eq!(result.series.len(), 5_000);
    assert!(
        result
            .series
            .values()
            .all(|runs| runs.len() == 1 && runs[0].last_hour == 101)
    );
    let winner = String::from_utf8(crate::test_fixtures::scale_tag(4_999)).unwrap();
    assert_eq!(result.series[&winner][0].launched, 1);
    let params = RaidsParams {
        branch_id: "1".into(),
        view: RaidsView::Tally,
        tags: vec![],
        top: Some(RaidTop {
            count: 10,
            metric: RaidMetric::Launched,
        }),
    };
    let series = selected_series(&result, &params, &AtomicBool::new(false)).unwrap();
    assert_eq!(series.len(), 1, "zero countries do not pad a ranking");
    assert!(series.iter().any(|s| s.tag == winner));
    let all = tally_params(result.series.keys().cloned().collect(), None);
    assert!(Raids::validate(&all).is_ok());
    assert_eq!(
        selected_series(&result, &all, &AtomicBool::new(false))
            .unwrap()
            .len(),
        5_000
    );
    let mut reply = RaidsReply {
        branch_id: "1".into(),
        phase_preparing: native::PHASE_PREPARING,
        phase_ended: native::PHASE_ENDED,
        outcome_none: native::OUTCOME_NONE,
        available: Some(true),
        unavailable: None,
        types: vec![],
        artwork: vec![],
        episodes: vec![],
        series,
        read_hours: result.read_hours,
        warnings: vec![],
    };
    measure_reply(&reply, &[]).unwrap();
    reply.warnings.push("x".repeat(MAX_PRODUCT_REPLY_BYTES));
    assert!(
        measure_reply(&reply, &[]).is_err(),
        "complete response includes non-series fields"
    );
}

#[test]
fn raids_log_and_tally_share_one_source_scan_and_types_only_does_not_build_history() {
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    frame(100, vec![raid(1, 4, 0)]),
                    frame(101, vec![raid(1, 5, 3)]),
                ],
            )],
        )
        .unwrap(),
    );
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "raids-shared".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = |view| RaidsParams {
        branch_id: "1".into(),
        view,
        tags: vec!["GER".into()],
        top: None,
    };
    let context = LoadContext { content: None };
    let catalogue = Raids::load(
        &source,
        &params(RaidsView::Catalogue),
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(catalogue.available, Some(true));
    assert!(source.state::<Cache>().lock().unwrap().scans.is_empty());
    let log = Raids::load(
        &source,
        &params(RaidsView::Log),
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let tally = Raids::load(
        &source,
        &params(RaidsView::Tally),
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(log.episodes.len(), 1);
    assert_eq!(tally.series[0].runs.last().unwrap().succeeded, 1);
    assert_eq!(source.state::<Cache>().lock().unwrap().scans.len(), 1);
}

#[test]
fn raids_picker_checks_later_reads_after_empty_initial_rosters_without_content() {
    for actual_raid in [false, true] {
        let mut first = frame(100, vec![]);
        if let TableView::Complete(TableRows::RaidCountries(rows)) =
            &mut *first.tables[TableId::RaidCountries.index()]
        {
            rows[0].eligible_types.clear();
        }
        let mut later = frame(
            101,
            if actual_raid {
                vec![raid(1, 4, 0)]
            } else {
                vec![]
            },
        );
        if actual_raid
            && let TableView::Complete(TableRows::RaidCountries(rows)) =
                &mut *later.tables[TableId::RaidCountries.index()]
        {
            rows[0].eligible_types.clear();
        }
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, later])]).unwrap(),
        );
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: format!("raids-later-{actual_raid}"),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap();
        let params = RaidsParams {
            branch_id: "1".into(),
            view: RaidsView::Catalogue,
            tags: vec![],
            top: None,
        };
        let reply = Raids::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            reply.available,
            Some(true),
            "either later actual raids or an eligible type proves feature presence"
        );
        assert!(
            source.state::<Cache>().lock().unwrap().scans.is_empty(),
            "availability never builds log/tally history"
        );
    }
}

struct Scratch(std::path::PathBuf, std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("BATTLE_PLAN_WIDGET_TEST_SCRATCH").or_else(|| std::env::var_os("HOI4_WIDGET_TEST_SCRATCH"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("raids-tests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path, root.canonicalize().unwrap())
    }
    fn write(&self, relative: &str, text: &str) {
        let file = self.0.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let base = &self.1;
        let target = self.0.canonicalize().unwrap();
        assert!(target.starts_with(base) && &target != base);
        let _ = std::fs::remove_dir_all(target);
    }
}

fn mod_content() -> (Scratch, Content) {
    let scratch = Scratch::new();
    scratch.write(
        "base/common/buildings/buildings.txt",
        "buildings={mod_factory={}}",
    );
    scratch.write(
        "base/common/raids/types.txt",
        "types={base_raid={custom_map_icon=GFX_base}} ",
    );
    scratch.write("mod/common/raids/mod.txt", "types={mod_raid={custom_map_icon=GFX_mod days_to_prepare=4} refused_named={starting_point={types={building}}}} ");
    scratch.write("mod/interface/raids.gfx", "spriteTypes={spriteType={name=GFX_mod texturefile=\"gfx/missing.dds\"} spriteType={name=GFX_mod_mod_factory texturefile=\"gfx/missing.dds\"} spriteType={name=GFX_raid_type_icon_empty texturefile=\"gfx/missing.dds\"}} ");
    scratch.write(
        "base/localisation/english/base_l_english.yml",
        "\u{feff}l_english:\n raid_type_mod_raid:0 \"Old raid\"\n GER:0 \"Old country\"\n",
    );
    scratch.write("mod/localisation/english/replace/mod_l_english.yml", "\u{feff}l_english:\n raid_type_mod_raid:0 \"Modded raid\"\n GER_NEW:0 \"Renamed country\"\n RAID_INST_PHASE_PREPARING:0 \"Mod preparing\"\n RAID_INST_PHASE_IN_PROGRESS:0 \"Mod flight\"\n RAID_INST_OUTCOME_NONE:0 \"Mod none\"\n");
    let base = scratch.0.join("base");
    let added = scratch.0.join("mod");
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&base, &[][..]), (&added, &[][..])]).unwrap();
    let content = read_content(Arc::new(Catalogue::of_layers(layers, "english").unwrap())).unwrap();
    (scratch, content)
}

#[test]
fn raids_offline_fallbacks_only_cover_absent_localisation_and_omit_reader_notes() {
    use crate::content_check::Kind;
    let scratch = Scratch::new();
    scratch.write(
        "common/buildings/buildings.txt",
        "buildings={mod_factory={}}",
    );
    scratch.write(
        "common/raids/types.txt",
        "types={named={} missing={} refused={} duplicate={} duplicate={} unsupported={starting_point={types={building}}} noted={unit_model={type=x}}}",
    );
    let refused_phase = native::PHASE_KEYS[0];
    let missing_phase = native::PHASE_KEYS[1];
    let mut localisation = String::from(
        "\u{feff}l_english:\n raid_type_named:0 \"Named raid\"\n raid_type_refused:0 \"\u{a4}unsupported_control\"\n raid_type_noted:0 \"Noted raid\"\n",
    );
    for key in native::PHASE_KEYS.into_iter().chain(native::OUTCOME_KEYS) {
        if key == missing_phase {
            continue;
        }
        let text = if key == refused_phase {
            "\u{a4}unsupported_control"
        } else {
            "Known label"
        };
        localisation.push_str(&format!(" {key}:0 \"{text}\"\n"));
    }
    scratch.write("localisation/types_l_english.yml", &localisation);
    let layers = installed_content::ResolvedLayers::from_folders(&[(&scratch.0, &[][..])]).unwrap();
    let raids = installed_content::raids::read(&layers).unwrap();
    assert!(!raids.misreads.is_empty(), "{raids:?}");
    assert_eq!(raids.uncertain.len(), 2);
    let cat = Catalogue::of_layers_for_check(layers, "english").unwrap();
    assert_eq!(
        prepare_types(&cat, &raids)[0].name.as_deref(),
        Some("Named raid")
    );
    cat.take_findings();
    let findings = check_content(&cat).unwrap();
    for subject in ["raid type duplicate", "raid type unsupported"] {
        assert!(
            findings.iter().all(|finding| finding.subject != subject),
            "{findings:?}"
        );
    }
    assert!(
        findings.iter().all(|finding| finding.kind != Kind::Misread),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| !(finding.kind == Kind::Fallback
                && finding.subject == "raid type refused")),
        "{findings:?}"
    );
    for subject in ["raid_type_refused", refused_phase] {
        let matching: Vec<_> = findings
            .iter()
            .filter(|finding| finding.subject == subject)
            .collect();
        assert_eq!(matching.len(), 1, "{findings:?}");
        assert_eq!(matching[0].kind, Kind::Unsupported);
    }
    assert!(
        findings
            .iter()
            .any(|finding| finding.subject == "raid type missing"
                && finding.kind == Kind::Fallback
                && finding.message
                    == "raid_type_missing: name localisation is missing; shown as unknown"),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.subject == missing_phase
                && finding.kind == Kind::Fallback
                && finding.message == "raid phase/outcome name is missing; shown as unknown"),
        "{findings:?}"
    );
}

#[test]
fn raids_live_reply_warns_only_for_unproven_state_reading() {
    use replay::spool::Record;
    let scratch = Scratch::new();
    for (path, text) in [
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
        (
            "common/defines/defines.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
        ),
        (
            "common/buildings/buildings.txt",
            "buildings={mod_factory={}}",
        ),
        ("common/raids/types.txt", "types={mod_raid={}}"),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        (
            "history/states/1-kept.txt",
            "@bad nope orphan=yes state={id=41 name=STATE_KEPT impassable=maybe provinces={1} resources={unknown=2} history={victory_points={1} buildings=4} state_startup_effect={free_operative={operative=authored}}}",
        ),
        (
            "history/states/2-duplicate.txt",
            "state={id=41 name=STATE_DUPLICATE}",
        ),
        (
            "localisation/raids_l_english.yml",
            "\u{feff}l_english:\n raid_type_mod_raid:0 \"Mod raid\"\n STATE_KEPT:0 \"Kept state\"\n STATE_DUPLICATE:0 \"Discarded state\"\n",
        ),
    ] {
        scratch.write(path, text);
    }
    let (options, identity) = crate::test_fixtures::content_identity(&scratch.0);
    let mut observed = frame(100, vec![raid(1, 2, 0)]);
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *observed.tables[TableId::States.index()]
    {
        rows[0].custom_name.clear();
    }
    let mut input = records(1, vec![observed]);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "raid-duplicate-state".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let catalogue = catalogue::catalogue(&source, &options).unwrap();
    let mut state_content = scripted_text::read_state_content(&catalogue.content.layers).unwrap();
    assert!(
        state_content
            .warnings
            .iter()
            .any(|note| note.message.contains("state 41 is defined by both"))
    );
    state_content.warnings.push(installed_content::states::Diagnostic {
        kind: installed_content::DiagnosticKind::Unsupported,
        message: "fixture state form is unsupported".into(),
        cause: None,
    });
    assert!(catalogue.state_content.set(Ok(state_content)).is_ok());
    let state_content = catalogue.cached_states().unwrap();
    let proven: Vec<_> = state_content
        .warnings
        .iter()
        .filter(|note| note.cause.is_some())
        .collect();
    assert_eq!(proven.len(), 5, "{proven:?}");
    let reply = Raids::load(
        &source,
        &RaidsParams {
            branch_id: "1".into(),
            view: RaidsView::Log,
            tags: vec!["GER".into()],
            top: None,
        },
        &LoadContext {
            content: Some(options),
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply.episodes[0].runs[0].target_name.as_deref(),
        Some("Kept state")
    );
    assert!(
        reply
            .warnings
            .iter()
            .all(|note| !note.contains("state 41 is defined by both")),
        "{:?}",
        reply.warnings
    );
    assert_eq!(
        reply
            .warnings
            .iter()
            .filter(|note| note.as_str() == "fixture state form is unsupported")
            .count(),
        1
    );
    for note in proven {
        assert!(
            !reply.warnings.contains(&note.message),
            "{:?}",
            reply.warnings
        );
    }
    for note in state_content
        .warnings
        .iter()
        .filter(|note| note.cause.is_none() && note.kind != installed_content::DiagnosticKind::Warning)
    {
        assert!(
            reply.warnings.contains(&note.message),
            "{:?}",
            reply.warnings
        );
    }
    assert!(
        reply
            .warnings
            .iter()
            .any(|note| note.contains("definition `@bad` has no `=`"))
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|note| note.contains("Deferred effect `free_operative`"))
    );
}

#[test]
fn raids_modded_types_use_content_names_and_refused_forms_are_named_and_counted() {
    let (_scratch, content) = mod_content();
    assert_eq!(
        content
            .types
            .iter()
            .find(|r| r.token == "mod_raid")
            .unwrap()
            .name
            .as_deref(),
        Some("Modded raid")
    );
    assert!(
        content
            .types
            .iter()
            .any(|r| r.token == "refused_named" && r.missing.is_some())
    );
    assert!(
        content
            .warnings
            .iter()
            .any(|line| line.contains("1 raid types") && line.contains("refused_named"))
    );
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 2, 0)]), Some(&content))
        .unwrap();
    let result = scan.finish();
    let run = &result.episodes[0].runs[0];
    assert_eq!(run.raid_type.as_deref(), Some("mod_raid"));
    assert_eq!(run.phase_name.as_deref(), Some("Mod preparing"));
    assert_eq!(run.target_name.as_deref(), Some("Recorded state"));
    assert_eq!(run.icon.as_deref(), Some("GFX_mod"));
    assert!(
        content.icons.lock().unwrap().is_empty(),
        "scanning never decodes artwork"
    );
    let artwork = log_artwork(&content, &result.episodes);
    assert_eq!(artwork.len(), 1);
    assert!(
        artwork[0]
            .missing
            .as_deref()
            .is_some_and(|reason| reason.contains("raid sprite GFX_mod"))
    );
}

#[test]
fn raids_native_icon_empty_override_and_target_building_specialisation_use_shared_reader() {
    let (_scratch, content) = mod_content();
    let empty = RaidType {
        name: "empty".into(),
        custom_map_icon: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(
        icon_key(&content.catalogue, &empty, None).as_deref(),
        Ok("GFX_raid_type_icon_empty")
    );
    let modded = content.raids.get("mod_raid").unwrap();
    assert_eq!(
        icon_key(&content.catalogue, modded, Some("mod_factory")).as_deref(),
        Ok("GFX_mod_mod_factory")
    );
    assert_eq!(
        icon_key(&content.catalogue, modded, Some("absent")).as_deref(),
        Ok("GFX_mod")
    );
    let unknown = RaidType {
        name: "missing".into(),
        ..Default::default()
    };
    assert_eq!(
        icon_key(&content.catalogue, &unknown, None).as_deref(),
        Ok("GFX_raid_type_icon_unknown")
    );
}

#[test]
fn raids_renamed_country_and_target_are_resolved_at_the_hour_without_baking_country_names() {
    let (_scratch, content) = mod_content();
    let mut before = frame(100, vec![raid(1, 4, 0)]);
    *before.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 9001,
            ..Default::default()
        }]));
    *before.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            ruling_party: None,
            ..Default::default()
        }]));
    *before.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![]));
    *before.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(vec![]));
    let mut after = before.clone();
    after.hour = 101;
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *after.tables[TableId::CountryAppearance.index()]
    {
        rows[0].cosmetic_tag = b"GER_NEW".to_vec();
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *after.tables[TableId::States.index()]
    {
        rows[0].custom_name = b"Renamed target".to_vec();
    }
    let old = crate::products::country_styles::HourStyles::read(
        &before,
        Some(&content.catalogue.localisation),
    )
    .unwrap();
    let new = crate::products::country_styles::HourStyles::read(
        &after,
        Some(&content.catalogue.localisation),
    )
    .unwrap();
    assert_eq!(old.get("GER").unwrap().name.as_deref(), Some("Old country"));
    assert_eq!(
        new.get("GER").unwrap().name.as_deref(),
        Some("Renamed country")
    );
    let mut scan = Scan::new();
    scan.observe(&before, Some(&content)).unwrap();
    scan.observe(&after, Some(&content)).unwrap();
    let result = scan.finish();
    assert_eq!(
        result.episodes[0]
            .runs
            .iter()
            .map(|r| r.target_name.as_deref())
            .collect::<Vec<_>>(),
        [Some("Recorded state"), Some("Renamed target")]
    );
    assert!(
        result.episodes[0]
            .runs
            .iter()
            .all(|r| r.raider.as_deref() == Some("GER"))
    );
}

#[test]
fn raids_unsupported_native_enums_are_unknown_with_named_counted_refusal() {
    let (_scratch, content) = mod_content();
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 77, 88)]), Some(&content))
        .unwrap();
    let result = scan.finish();
    let run = &result.episodes[0].runs[0];
    assert_eq!(
        (
            run.phase_name.clone(),
            run.outcome_name.clone(),
            run.launched,
            run.succeeded
        ),
        (None, None, None, None)
    );
    assert!(result.warnings.iter().any(|line| line.contains("1 raid facts") && line.contains("unsupported raid phase 77")));
    assert!(
        result
            .warnings
            .iter()
            .any(|line| line.contains("unsupported raid outcome 88"))
    );
}

fn tally_params(tags: Vec<String>, top: Option<RaidTop>) -> RaidsParams {
    RaidsParams {
        branch_id: "1".into(),
        view: RaidsView::Tally,
        tags,
        top,
    }
}

#[test]
fn raids_preparation_runs_bound_growth_and_preserve_each_read_without_splitting_facts() {
    let mut scan = Scan::new();
    let mut observed = frame(100, vec![raid(1, 2, 0), raid(2, 2, 0)]);
    for hour in 100..1_100 {
        observed.hour = hour;
        *observed.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(
            (1..=2)
                .map(|id| RaidRow {
                    preparation_hours: if id == 1 { 37 } else { hour - 100 },
                    ..raid(id, 2, 0)
                })
                .collect(),
        ));
        scan.observe(&observed, None).unwrap();
    }
    let result = scan.finish();
    assert_eq!(result.episodes.len(), 2);
    for episode in &result.episodes {
        assert_eq!(episode.runs.len(), 1);
        assert_eq!(
            (episode.runs[0].first_hour, episode.runs[0].last_hour),
            (100, 1_099)
        );
        assert_eq!(episode.preparation.len(), 1);
    }
    assert_eq!(
        result.episodes[0].preparation[0],
        RaidPreparationRun(100, 1_099, 37, 0)
    );
    assert_eq!(
        result.episodes[1].preparation[0],
        RaidPreparationRun(100, 1_099, 0, 1)
    );
    for hour in 100..1_100 {
        for (index, episode) in result.episodes.iter().enumerate() {
            let run = &episode.preparation[0];
            assert_eq!(
                i64::from(run.2) + i64::from(hour - run.0) * run.3,
                if index == 0 {
                    37
                } else {
                    i64::from(hour - 100)
                }
            );
        }
    }
    let reply = RaidsReply {
        branch_id: "1".into(),
        phase_preparing: native::PHASE_PREPARING,
        phase_ended: native::PHASE_ENDED,
        outcome_none: native::OUTCOME_NONE,
        available: Some(true),
        unavailable: None,
        types: vec![],
        artwork: vec![],
        episodes: vec![],
        series: vec![],
        read_hours: result.read_hours.clone(),
        warnings: vec![],
    };
    measure_reply(&reply, &result.episodes).unwrap();
    assert_eq!(
        serde_json::to_string(&RaidPreparationRun(100, 1_099, 37, 0)).unwrap(),
        "[100,1099,37,0]"
    );
}

#[test]
fn raids_preparation_has_exact_runs_and_gaps_without_interpolation() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 2, 0)]), None)
        .unwrap();
    let mut missed = frame(101, vec![raid(1, 2, 0)]);
    *missed.tables[TableId::Raids.index()] = TableView::Unavailable {
        reason: "synthetic missing read".into(),
    };
    scan.observe(&missed, None).unwrap();
    scan.observe(&frame(103, vec![raid(1, 2, 0)]), None)
        .unwrap();
    let result = scan.finish();
    assert_eq!(
        result.episodes[0].preparation,
        [
            RaidPreparationRun(100, 100, 37, 0),
            RaidPreparationRun(103, 103, 37, 0)
        ]
    );
    assert_eq!(
        result.episodes[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
}

#[test]
fn raids_preparation_slope_changes_and_signed_extremes_preserve_exact_reads() {
    let values = [37, 38, 39, 37, 37, 37, i32::MIN, i32::MAX, 0];
    let mut scan = Scan::new();
    for (offset, hours) in values.into_iter().enumerate() {
        scan.observe(
            &frame(
                100 + offset as i32,
                vec![RaidRow {
                    preparation_hours: hours,
                    ..raid(1, 2, 0)
                }],
            ),
            None,
        )
        .unwrap();
    }
    let result = scan.finish();
    let preparation = &result.episodes[0].preparation;
    assert_eq!(
        preparation,
        &[
            RaidPreparationRun(100, 102, 37, 1),
            RaidPreparationRun(103, 105, 37, 0),
            RaidPreparationRun(106, 107, i32::MIN, 4_294_967_295),
            RaidPreparationRun(108, 108, 0, 0),
        ]
    );
    assert_eq!(result.episodes[0].runs.len(), 1);
    for (offset, hours) in values.into_iter().enumerate() {
        let hour = 100 + offset as i32;
        let run = preparation
            .iter()
            .find(|run| run.0 <= hour && hour <= run.1)
            .unwrap();
        assert_eq!(
            i64::from(run.2) + i64::from(hour - run.0) * run.3,
            i64::from(hours)
        );
    }
}

#[test]
fn raids_top_leaves_zero_and_absent_countries_out_but_chosen_zero_survives() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![]), None).unwrap();
    scan.observe(&frame(101, vec![raid(1, 4, 0)]), None)
        .unwrap();
    let mut absent = frame(102, vec![raid(1, 4, 0)]);
    *absent.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![]));
    scan.observe(&absent, None).unwrap();
    let result = scan.finish();
    assert_eq!(result.read_hours, [100, 101]);
    let top = selected_series(
        &result,
        &tally_params(
            vec![],
            Some(RaidTop {
                count: 10,
                metric: RaidMetric::Launched,
            }),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(
        top[0].membership.as_deref(),
        Some(
            &[HourSpan {
                first_hour: 101,
                last_hour: 101
            }][..]
        )
    );
    assert_eq!(top[0].runs[0].first_hour, 101);
    assert_eq!(top[0].runs[0].last_hour, 101);
    let chosen = selected_series(
        &result,
        &tally_params(vec!["GER".into()], None),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(chosen[0].runs[0].launched, 0);
    assert_eq!(chosen[0].runs[0].first_hour, 100);
    let successes = selected_series(
        &result,
        &tally_params(
            vec![],
            Some(RaidTop {
                count: 10,
                metric: RaidMetric::Succeeded,
            }),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(successes.is_empty());
}

#[test]
fn raids_both_ranking_preserves_success_without_a_proven_launch() {
    let mut scan = Scan::new();
    scan.observe(
        &frame(
            100,
            vec![raid(1, native::PHASE_ENDED, native::OUTCOME_SUCCESS)],
        ),
        None,
    )
    .unwrap();
    let result = scan.finish();
    let params = |metric| tally_params(vec![], Some(RaidTop { count: 1, metric }));
    let cancelled = AtomicBool::new(false);
    assert!(
        selected_series(&result, &params(RaidMetric::Launched), &cancelled)
            .unwrap()
            .is_empty()
    );
    let both = selected_series(&result, &params(RaidMetric::Both), &cancelled).unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(
        (both[0].runs[0].launched, both[0].runs[0].succeeded),
        (0, 1)
    );
}

#[test]
fn raids_partial_parent_or_identity_reads_are_unknown_for_an_empty_race() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![]), None).unwrap();
    let mut missing_parent = frame(101, vec![]);
    *missing_parent.tables[TableId::RaidCountries.index()] =
        TableView::Complete(TableRows::RaidCountries(vec![]));
    scan.observe(&missing_parent, None).unwrap();
    let mut unresolved = frame(102, vec![]);
    *unresolved.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    scan.observe(&unresolved, None).unwrap();
    let mut unread = frame(103, vec![]);
    *unread.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic unread countries".into(),
    };
    scan.observe(&unread, None).unwrap();
    let result = scan.finish();
    assert_eq!(result.read_hours, [100]);
    assert_eq!(result.series["GER"].len(), 1);
    assert_eq!(result.series["GER"][0].last_hour, 100);
}

#[test]
fn raids_same_native_handle_is_counted_once_across_complete_absences() {
    let mut scan = Scan::new();
    scan.observe(&frame(100, vec![raid(1, 4, 0)]), None)
        .unwrap();
    scan.observe(&frame(101, vec![]), None).unwrap();
    scan.observe(&frame(102, vec![raid(1, 4, 0)]), None)
        .unwrap();
    scan.observe(&frame(103, vec![raid(1, 5, 3)]), None)
        .unwrap();
    scan.observe(&frame(104, vec![]), None).unwrap();
    let mut another_reference = raid(1, 4, 0);
    another_reference.reference_type += 1;
    scan.observe(&frame(105, vec![raid(1, 5, 3), another_reference]), None)
        .unwrap();
    let result = scan.finish();
    assert_eq!(
        (
            result.series["GER"].last().unwrap().launched,
            result.series["GER"].last().unwrap().succeeded
        ),
        (2, 1)
    );
    assert_eq!(result.episodes.len(), 4);
}

fn changed_controller(mut observed: DecodedFrame, controller: u32) -> DecodedFrame {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        rows.push(replay::rows::CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"MOD".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *observed.tables[TableId::CountryLinks.index()]
    {
        rows.extend([
            replay::rows::CountryLinkRow {
                id: 22,
                canonical_slot: 8,
            },
            replay::rows::CountryLinkRow {
                id: 9002,
                canonical_slot: 8,
            },
        ]);
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *observed.tables[TableId::States.index()]
    {
        rows[0].controller = controller;
    }
    if let TableView::Complete(TableRows::RaidCountries(rows)) =
        &mut *observed.tables[TableId::RaidCountries.index()]
    {
        rows.push(RaidCountryRow {
            canonical_slot: 8,
            country: 22,
            ..Default::default()
        });
    }
    observed
}

#[test]
fn raids_link_tags_keep_balance_reads_and_follow_identity_changes_without_bridging_unread_hours() {
    let mut first = changed_controller(frame(100, vec![raid(1, 4, 0)]), 11);
    if let TableView::Complete(TableRows::RaidCountries(rows)) =
        &mut *first.tables[TableId::RaidCountries.index()]
    {
        rows[0].country = 9001;
    }
    let mut balance = first.clone();
    balance.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *balance.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(2);
    }
    let mut renamed = balance.clone();
    renamed.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"NEW".to_vec();
    }
    let mut repointed = renamed.clone();
    repointed.hour = 103;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *repointed.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 11)
            .unwrap()
            .canonical_slot = 8;
    }
    let mut unread_countries = repointed.clone();
    unread_countries.hour = 104;
    *unread_countries.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic unread countries".into(),
    };
    let mut unread_links = repointed.clone();
    unread_links.hour = 105;
    *unread_links.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic unread links".into(),
    };
    let mut recovered = repointed.clone();
    recovered.hour = 106;
    let mut scan = Scan::new();
    for observed in [
        first,
        balance,
        renamed,
        repointed,
        unread_countries,
        unread_links,
        recovered,
    ] {
        scan.observe(&observed, None).unwrap();
    }
    let result = scan.finish();
    assert_eq!(result.read_hours, [100, 101, 102, 103, 106]);
    assert_eq!(result.episodes.len(), 1);
    assert_eq!(
        result.episodes[0].preparation,
        [
            RaidPreparationRun(100, 103, 37, 0),
            RaidPreparationRun(106, 106, 37, 0),
        ]
    );
    assert_eq!(
        result.episodes[0]
            .runs
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.raider.as_deref(),
                run.victim.as_deref(),
            ))
            .collect::<Vec<_>>(),
        [
            (100, 101, Some("GER"), Some("GER")),
            (102, 102, Some("NEW"), Some("NEW")),
            (103, 103, Some("NEW"), Some("MOD")),
            (106, 106, Some("NEW"), Some("MOD")),
        ]
    );
    assert_eq!(result.series["GER"][0].launched, 1);
    assert_eq!(result.series["GER"][0].last_hour, 101);
    assert_eq!(
        result.series["NEW"]
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.launched))
            .collect::<Vec<_>>(),
        [(102, 103, 0), (106, 106, 0)],
        "a tag change does not re-count the same native raid handle"
    );
    assert_eq!(
        result.segments["GER"],
        [HourSpan {
            first_hour: 100,
            last_hour: 101,
        }]
    );
    assert_eq!(
        result.segments["NEW"],
        [
            HourSpan {
                first_hour: 102,
                last_hour: 103,
            },
            HourSpan {
                first_hour: 106,
                last_hour: 106,
            },
        ]
    );
}

#[test]
fn raids_ended_victim_freezes_at_first_phase_five_read_including_unknown() {
    let mut scan = Scan::new();
    scan.observe(
        &changed_controller(frame(100, vec![raid(1, 4, 0)]), 22),
        None,
    )
    .unwrap();
    scan.observe(
        &changed_controller(frame(101, vec![raid(1, 5, 3)]), 11),
        None,
    )
    .unwrap();
    scan.observe(
        &changed_controller(frame(102, vec![raid(1, 5, 3)]), 22),
        None,
    )
    .unwrap();
    let result = scan.finish();
    assert_eq!(result.episodes[0].runs[0].victim.as_deref(), Some("MOD"));
    assert_eq!(
        result.episodes[0].runs.last().unwrap().victim.as_deref(),
        Some("GER")
    );
    assert_eq!(result.episodes[0].runs.last().unwrap().first_hour, 101);
    assert_eq!(result.episodes[0].runs.last().unwrap().last_hour, 102);
    let mut scan = Scan::new();
    let mut unknown = frame(100, vec![raid(1, 5, 3)]);
    *unknown.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "synthetic first ended victim unavailable".into(),
    };
    scan.observe(&unknown, None).unwrap();
    scan.observe(&frame(101, vec![raid(1, 5, 3)]), None)
        .unwrap();
    assert!(
        scan.finish().episodes[0]
            .runs
            .iter()
            .all(|run| run.victim.is_none())
    );
}

#[test]
fn raids_log_artwork_is_lazy_unique_and_limited_to_used_episode_keys() {
    let (_scratch, content) = mod_content();
    assert!(content.icons.lock().unwrap().is_empty());
    let mut scan = Scan::new();
    scan.observe(
        &frame(100, vec![raid(1, 2, 0), raid(2, 2, 0)]),
        Some(&content),
    )
    .unwrap();
    let result = scan.finish();
    assert!(
        content.icons.lock().unwrap().is_empty(),
        "Tally scan never decodes pixels"
    );
    {
        let mut icons = content.icons.lock().unwrap();
        icons.insert("GFX_mod".into(), Ok("used-image".into()));
        icons.insert("GFX_unused".into(), Ok("unused-image".into()));
    }
    let art = log_artwork(&content, &result.episodes);
    assert_eq!(art.len(), 1);
    assert_eq!(art[0].key, "GFX_mod");
    assert_eq!(art[0].image.as_deref(), Some("used-image"));
    assert!(log_artwork(&content, &[]).is_empty());
    let types = serde_json::to_value(&content.types).unwrap();
    assert!(
        types
            .as_array()
            .unwrap()
            .iter()
            .all(|kind| kind.get("icon").is_none())
    );
}

#[test]
fn raids_selection_sizes_from_the_recording_and_reply_refuses_the_transport_budget() {
    let all_countries = tally_params(
        (0..5_000)
            .map(|i| String::from_utf8(crate::test_fixtures::scale_tag(i as u32)).unwrap())
            .collect(),
        None,
    );
    assert!(Raids::validate(&all_countries).is_ok());
    assert_eq!(
        Raids::validate(&tally_params(
            vec![],
            Some(RaidTop {
                count: 0,
                metric: RaidMetric::Launched
            })
        )),
        Err("Invalid top countries")
    );
    assert!(
        Raids::validate(&tally_params(
            vec![],
            Some(RaidTop {
                count: u32::MAX,
                metric: RaidMetric::Launched
            })
        ))
        .is_ok()
    );
    let mut reply = RaidsReply {
        branch_id: "1".into(),
        phase_preparing: native::PHASE_PREPARING,
        phase_ended: native::PHASE_ENDED,
        outcome_none: native::OUTCOME_NONE,
        available: Some(true),
        unavailable: None,
        types: vec![],
        artwork: vec![],
        episodes: vec![],
        series: vec![],
        read_hours: vec![],
        warnings: vec![String::new()],
    };
    let envelope = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    reply.warnings[0] = "x".repeat(MAX_PRODUCT_REPLY_BYTES - envelope);
    assert_eq!(
        json_len(&reply, MAX_PRODUCT_REPLY_BYTES),
        Some(MAX_PRODUCT_REPLY_BYTES)
    );
    measure_reply(&reply, &[]).unwrap();
    reply.warnings[0].push('x');
    assert!(
        measure_reply(&reply, &[])
            .unwrap_err()
            .to_string()
            .starts_with("RaidReplyTooLarge:"),
        "complete product body refuses at the shared 24 MiB reserve"
    );
    assert!(
        serde_json::to_value(RaidDate { hour: 100 })
            .unwrap()
            .get("exact")
            .is_none()
    );
}

#[test]
fn raids_target_names_use_original_tags_and_empty_qualified_names_mask_bare_keys() {
    use replay::rows::{Recorded, ReservedCountryNamingRow};
    let (_scratch, mut content) = mod_content();
    let cat = Arc::get_mut(&mut content.catalogue).unwrap();
    let mut entries: Vec<_> = cat
        .localisation
        .iter()
        .map(|(key, text)| (key.to_owned(), text.to_owned()))
        .collect();
    entries.extend([
        ("original/mod_VICTORY_POINTS_101".into(), "".into()),
        (
            "other/mod_VICTORY_POINTS_101".into(),
            "Original target".into(),
        ),
        ("GER_VICTORY_POINTS_101".into(), "Wrong current tag".into()),
        ("controls/mod_VICTORY_POINTS_101".into(), "§Y§!".into()),
        ("VICTORY_POINTS_101".into(), "Wrong bare retry".into()),
        ("_VICTORY_POINTS_101".into(), "".into()),
    ]);
    let mut localisation: installed_content::localisation::Localisation = entries.into_iter().collect();
    localisation.language = "english".into();
    cat.localisation = Arc::new(localisation);
    let mut observed = frame(100, vec![raid(1, 2, 0)]);
    let scope = RaidScope {
        province: 101,
        ..Default::default()
    };
    let country_id = observed.complete::<tables::Countries>().unwrap()[0].id;
    *observed.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: country_id,
            name_tag: b"unrelated_generator".to_vec(),
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(b"original/mod".to_vec())),
            ..Default::default()
        }]));
    assert_eq!(
        target_name(&observed, &scope, &content).as_deref(),
        Ok("Recorded state")
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *observed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = Recorded::Value(Some(b"other/mod".to_vec()));
    }
    assert_eq!(
        target_name(&observed, &scope, &content).as_deref(),
        Ok("Original target")
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *observed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = Recorded::Value(Some(b"controls/mod".to_vec()));
    }
    assert_eq!(
        target_name(&observed, &scope, &content).as_deref(),
        Ok(""),
        "native emptiness is checked before presentation strips controls"
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *observed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = Recorded::Value(Some(Vec::new()));
    }
    assert_eq!(
        target_name(&observed, &scope, &content).as_deref(),
        Ok("Recorded state"),
        "a positive original reference with empty bytes still chooses the empty qualified key"
    );
    for (reference, bytes, reason) in [
        (
            Recorded::Value(None),
            Recorded::Value(Some(b"other/mod".to_vec())),
            "original_tag_reference capture refused",
        ),
        (
            Recorded::NotRecorded,
            Recorded::Value(Some(b"other/mod".to_vec())),
            "original_tag_reference not recorded",
        ),
        (
            Recorded::Value(Some(1)),
            Recorded::Value(None),
            "original_tag capture refused",
        ),
        (
            Recorded::Value(Some(1)),
            Recorded::NotRecorded,
            "original_tag not recorded",
        ),
    ] {
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *observed.tables[TableId::CountryAppearance.index()]
        {
            rows[0].original_tag_reference = reference;
            rows[0].original_tag = bytes;
        }
        assert!(
            target_name(&observed, &scope, &content)
                .unwrap_err()
                .contains(reason),
            "an unknown earlier qualified key cannot fall back to a known state name"
        );
    }
    if let TableView::Complete(TableRows::RaidTargets(rows)) =
        &mut *observed.tables[TableId::RaidTargets.index()]
    {
        rows[0].scope = scope;
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *observed.tables[TableId::States.index()]
    {
        rows[0].owner = 0;
    }
    *observed.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                name_tag: b"unrelated_generator".to_vec(),
                original_tag_reference: Recorded::NotRecorded,
                original_tag: Recorded::Value(Some(b"other/mod".to_vec())),
                ..Default::default()
            },
        ]));
    let mut scan = Scan::new();
    scan.observe(&observed, Some(&content)).unwrap();
    let result = scan.finish();
    assert!(result.episodes[0].runs[0].target_name.is_none());
    assert!(result.warnings.iter().any(|line| {
        line.contains("ReservedCountryNaming.original_tag_reference not recorded")
            && line.contains("1 raid facts")
    }));
}
