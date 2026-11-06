use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
use replay::publication::replace_file;
use replay_data::{rows::CompletedFocusRow, rows::FocusRow, rows::TableRows};
use replay_frame::frame::TableView;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A mode a family is prepared in: its name and how it prepares a source.
type PrepareMode<'a> = (&'a str, &'a dyn Fn(&Source));
/// A [`PrepareMode`] that may also reload the prepared data before the walk.
type PrepareReloadMode<'a> = (&'a str, &'a dyn Fn(&Source), bool);

mod catalogue_refusals;
mod cold_defaults;
mod commit_points;
mod driver_plan;
mod equipment_loss_pending;
mod family_lanes;
mod family_refusal;
mod forward_windows;
mod independent_families;
mod independent_feed;
mod independent_groups;
mod independent_identities;
mod independent_leaders;
mod independent_ledgers;
mod independent_map_pages;
mod independent_sampled;
mod independent_superiority;
mod map_export;
mod parallel_flush;
mod production_scans;
mod publication_gates;
mod reply_gates;
mod serialization_order;
mod walk_pace;
mod walk_recording;
mod walk_selection;

#[test]
fn family_commits_rewrite_no_unchanged_map_binding_bytes() {
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut progress = dashboard_base::prepared_map::Progress {
        stride: 720,
        selected: vec![TableId::Countries as u16],
        ..Default::default()
    };
    let mut writer = fixture.writer();
    write_map_progress(&mut writer, &progress).unwrap();
    assert_eq!(
        writer.bytes_written(),
        serde_json::to_vec(&progress).unwrap().len() as u64
    );
    writer.commit().unwrap();
    let original = fixture
        .store
        .read(&fixture.key, MAP_PROGRESS, 0)
        .unwrap()
        .unwrap()
        .id();
    for append in [false, true] {
        let mut writer = if append {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        write_map_progress(&mut writer, &progress).unwrap();
        assert_eq!(writer.bytes_written(), 0);
        assert_eq!(writer.payload_bytes_change(), 0);
        writer.commit().unwrap();
        assert_eq!(
            fixture
                .store
                .read(&fixture.key, MAP_PROGRESS, 0)
                .unwrap()
                .unwrap()
                .id(),
            original
        );
    }
    progress.resume_range = Some(3);
    let expected = serde_json::to_vec(&progress).unwrap();
    let mut writer = fixture.writer();
    write_map_progress(&mut writer, &progress).unwrap();
    assert_eq!(writer.bytes_written(), expected.len() as u64);
    writer.commit().unwrap();
    assert_eq!(fixture.payloads()[&(MAP_PROGRESS.to_owned(), 0)], expected);
}

/// Whether `query`'s data covers the whole branch ([`product_coverage`]).
fn complete(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    Ok(product_coverage(source, content, branch, query, cancelled)?
        .is_some_and(|range| range.complete))
}

/// A first view may sample the tail, but no hour is accepted before its
/// family's first publication. Every recorded tail hour stays explicitly
/// approximate or pending, and neither mark invents hours inside a gap.
fn assert_cold_prefix(source: &Source, branch: u64, reply: &serde_json::Value) {
    let replay = source.replay_ref();
    let mut recorded = std::collections::BTreeSet::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        recorded.extend(block.first_hour..=block.last_hour);
    }
    for gap in replay.gaps.iter().filter(|gap| gap.branch == branch) {
        for hour in gap.first_hour..=gap.last_hour {
            recorded.remove(&hour);
        }
    }
    let first = *recorded.first().unwrap();
    assert_eq!(
        reply["coverage"],
        serde_json::json!({"schema": SCHEMA, "firstHour": first,
            "lastHour": first - 1, "complete": false}),
        "{reply}"
    );
    let mut marked = std::collections::BTreeSet::new();
    for field in ["approximate", "pending"] {
        for span in reply[field].as_array().unwrap() {
            let first = i32::try_from(span["firstHour"].as_i64().unwrap()).unwrap();
            let last = i32::try_from(span["lastHour"].as_i64().unwrap()).unwrap();
            assert!(first <= last, "{field}: {span}");
            for hour in first..=last {
                assert!(marked.insert(hour), "{field}: overlapping mark at {hour}");
            }
        }
    }
    assert_eq!(marked, recorded, "{reply}");
}

#[test]
fn measurement_time_box_commits_a_prefix_and_resume_counts_only_new_hours() {
    let replay = recording();
    for serial in [true, false] {
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("measurement");
        let flag = AtomicBool::new(false);
        let (prefix, timing) =
            run_measured(&source, None, &flag, serial, Duration::ZERO, |_| {}).unwrap();
        assert_eq!(prefix.next_block, 1);
        assert_eq!(prefix.new_blocks, 1);
        assert_eq!(timing.hours, 2);
        assert!(timing.wall >= timing.observe);
        let (store, key) = super::super::files(&source, None).unwrap();
        assert_eq!(
            checkpoint(&store, &key, None)
                .unwrap()
                .unwrap()
                .0
                .cursor
                .next_block,
            1
        );
        assert_eq!(
            coverage(&source, None, 1, &flag)
                .unwrap()
                .unwrap()
                .last_hour,
            11
        );
        assert!(coverage(&source, None, 2, &flag).unwrap().is_none());
        let (tail, timing) =
            run_measured(&source, None, &flag, serial, Duration::ZERO, |_| {}).unwrap();
        assert_eq!(tail.next_block, 2);
        assert_eq!(tail.new_blocks, 1);
        assert_eq!(timing.hours, 2);
    }
}

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    stores: Vec<Arc<store::Store>>,
}

#[test]
fn observer_timing_table_is_filled_only_in_measured_mode() {
    let mut absent: Option<BTreeMap<&'static str, FamilyTimings>> = None;
    let mut calls = 0;
    time_family!(absent, "probe", observe, {
        calls += 1;
    });
    assert_eq!(calls, 1);
    assert!(absent.is_none());
    assert!(PreparationTimings::default().families.is_none());

    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("observer-timing");
    // Shared-read counters may be zero; their accounting is checked by the
    // shared-reader tests.
    let (_, timing) = independent::shared::with_test_unshared(|| {
        prepass::with_test_prepass(2, None, || {
            run_measured(
                &source,
                None,
                &AtomicBool::new(false),
                true,
                Duration::ZERO,
                |_| {},
            )
        })
    })
    .unwrap();
    let families = timing.families.expect("measured call sites fill the table");
    assert_eq!(families["store_commit"].calls, 1);
    assert!(families.values().all(|family| family.calls > 0));
    for name in [
        "setup.files",
        "setup.state",
        "setup.ordinals",
        "setup.overhead",
        "content.land_names",
        "content.production",
        "context.construct",
        "construct.decoder",
        "resume.saved-key",
        "resume.land_owners",
        "branch.restore-or-construct",
        "branch.flush_overhead",
        "branch.drop",
        "block.overhead",
        "frame.overhead",
        "stream.acquire",
        "progress.frame",
        "progress.accepted",
        "publication.open",
        "preparation.finalize",
        "preparation.drop",
    ] {
        assert!(families.contains_key(name), "missing timing row {name}");
    }
    // Block extracts run on pre-pass workers, in serial runs too.
    assert!(family_time(&families) - timing.worker_observe <= timing.wall);
    assert_eq!(
        family_time(&families) - timing.worker_observe + timing.residual,
        timing.wall
    );

    let (_, resumed) = run_measured(
        &source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        |_| {},
    )
    .unwrap();
    let families = resumed.families.unwrap();
    for name in [
        "resume.checkpoint",
        "resume.prefix",
        "resume.branches",
        "resume.observer-state",
        "resume.frame",
        "resume.decoder",
        "resume.observers",
    ] {
        assert!(
            families.contains_key(name),
            "missing resume timing row {name}"
        );
    }
    assert_eq!(
        family_time(&families) - resumed.worker_observe + resumed.residual,
        resumed.wall
    );

    // Prefix mode ignores the short time box, emits once per real commit, and
    // resumes at the accepted cursor without writing the earlier row again.
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("through-hours");
    let journal_path = fixture.root.join("blocks.jsonl");
    let mut journal = std::fs::File::create(&journal_path).unwrap();
    let flag = AtomicBool::new(false);
    for (target, expected_blocks) in [(1, 1), (3, 2)] {
        let (accepted, _) = run_measured_through(
            &source,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(target),
            |block| {
                use std::io::Write;
                let (store, key) = super::super::files(&source, None)?;
                assert_eq!(
                    checkpoint(&store, &key, None)?.unwrap().0.cursor.next_block,
                    block.ordinal + 1
                );
                assert_eq!(block.hours, 2);
                assert_eq!(
                    family_time(&block.families) - block.worker_observe + block.residual,
                    block.wall
                );
                serde_json::to_writer(
                    &mut journal,
                    &serde_json::json!({
                        "ordinal": block.ordinal, "cumulativeGameHours": block.cumulative_hours,
                        "storeBytes": block.store_bytes,
                    }),
                )?;
                journal.write_all(b"\n")?;
                journal.flush()
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(accepted.next_block, expected_blocks);
        assert_eq!(accepted.new_blocks, 1);
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(&journal_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), expected_blocks);
        assert_eq!(rows.last().unwrap()["ordinal"], expected_blocks - 1);
        assert_eq!(
            rows.last().unwrap()["cumulativeGameHours"],
            (expected_blocks * 2) as u64
        );
    }
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    let replay = second
        .append(&[continuation_records(
            &second,
            1,
            vec![frame(1, 14, 2), frame(1, 15, 2)],
        )])
        .unwrap();
    assert_eq!(replay.index.len(), 3);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("through-hours-ignores-time-box");
    let mut commits = Vec::new();
    let (accepted, _) = run_measured_through(
        &source,
        None,
        &flag,
        true,
        Duration::ZERO,
        Some(3),
        |block| {
            commits.push(block.ordinal);
            Ok(())
        },
        |_| {},
    )
    .unwrap();
    assert_eq!(accepted.next_block, 2);
    assert_eq!(commits, [0, 1]);
    let (unchanged, timings) = run_measured_through(
        &source,
        None,
        &flag,
        true,
        Duration::ZERO,
        Some(3),
        |_| panic!("an already reached prefix must not admit another block"),
        |_| {},
    )
    .unwrap();
    assert_eq!(unchanged.next_block, 2);
    assert_eq!(unchanged.new_blocks, 0);
    assert_eq!(timings.hours, 0);
}
impl Fixture {
    fn new(replay: &Replay) -> Self {
        let root = std::env::temp_dir().join(format!("replay-pipeline-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("campaign.battleplan");
        std::fs::write(&path, replay.bytes()).unwrap();
        Self {
            root,
            path,
            stores: Vec::new(),
        }
    }
    fn source(&mut self, revision: &str) -> Arc<Source> {
        let source = Arc::new(
            Source::open(
                crate::SourceKey {
                    source_id: revision.into(),
                    revision: crate::protocol::replay_revision(&self.path).unwrap(),
                },
                self.path.clone(),
                &AtomicBool::new(false),
            )
            .unwrap(),
        );
        self.stores
            .push(super::super::files(&source, None).unwrap().0);
        source
    }
    fn corrupt_block_preserving_stamp(&self, block: &BlockIndex) {
        self.corrupt_bytes_preserving_stamp(block.offset, block.length);
    }
    fn corrupt_diagnostics_preserving_stamp(&self, replay: &Replay, block: &BlockIndex) {
        use replay::container::Section;
        // Native keyframe anchors may need an accepted block's directory and
        // table sections. Poison only diagnostics: reading accepted frames still
        // fails, while the unaccepted tail keeps all its native dependencies.
        let sections = replay.block_sections(block).unwrap().unwrap();
        let stored: u64 = sections.iter().map(|section| section.compressed).sum();
        let mut offset = block.offset + block.length - stored;
        for section in sections {
            if section.section == Section::Diagnostics {
                self.corrupt_bytes_preserving_stamp(offset, section.compressed);
                return;
            }
            offset += section.compressed;
        }
        panic!("synthetic grouped block has no diagnostics");
    }
    fn corrupt_bytes_preserving_stamp(&self, offset: u64, length: u64) {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&self.path)
            .unwrap();
        let modified = file.metadata().unwrap().modified().unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&vec![0; usize::try_from(length).unwrap()])
            .unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
    fn replace(&self, replay: &Replay) {
        let pending = self.root.join("candidate");
        std::fs::write(&pending, replay.bytes()).unwrap();
        replace_file(&pending, &self.path).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for store in &self.stores {
            let _ = store.clear();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn frame(branch: u64, hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(branch, hour, count);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for country in rows {
            country.political_power_raw = Some(i64::from(hour) * 1000);
            country.command_power_raw = i64::from(hour) * 500;
        }
    }
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        (0..count)
            .map(|index| FocusRow {
                country: 100_000 + index,
                current: Some(b"mod/focus".to_vec()),
                progress: i64::from(hour),
                completed_count: 1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            (0..count)
                .map(|index| CompletedFocusRow {
                    country: 100_000 + index,
                    ordinal: 0,
                    token: b"mod/completed".to_vec(),
                })
                .collect(),
        ));
    frame
}
fn recording() -> Replay {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    first
        .append(&[records(2, vec![frame(2, 20, 2), frame(2, 21, 2)])])
        .unwrap()
}

fn assert_prepared_identities(source: &Source, branch: u64) {
    let flag = AtomicBool::new(false);
    let prepared =
        crate::products::country_history::prepared_branch_histories(source, branch, None, &flag)
            .unwrap()
            .expect("covered identities");
    let oracle =
        crate::products::country_history::branch_histories(source, branch, false, &[], &flag)
            .unwrap();
    assert_eq!(
        serde_json::to_value(&*prepared).unwrap(),
        serde_json::to_value(&*oracle).unwrap()
    );
}

#[test]
fn prepared_pipeline_keeps_other_families_when_raid_buildings_are_missing() {
    use crate::products::{
        Product,
        country_groups::{CountryGroupParams, CountryGroups},
        raids::{Raids, RaidsParams, RaidsView},
    };
    use engine_native::craid_instance as native;
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for raid content metadata");
    let frames: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000));
            frame.metrics.table_ns[TableId::Raids.index()] = 1;
            *frame.tables[TableId::RaidCountries.index()] =
                TableView::Complete(TableRows::RaidCountries(vec![
                    replay_data::rows::RaidCountryRow {
                        canonical_slot: 7,
                        country: 11,
                        eligible_types: vec![50],
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::Raids.index()] =
                TableView::Complete(TableRows::Raids(vec![replay_data::rows::RaidRow {
                    id: 1,
                    reference_type: native::REFERENCE_TYPE_VALUE,
                    parent_slot: 7,
                    phase: if hour == 100 {
                        native::PHASE_IN_PROGRESS
                    } else {
                        native::PHASE_ENDED
                    },
                    outcome: if hour == 100 {
                        native::OUTCOME_NONE
                    } else {
                        native::OUTCOME_SUCCESS
                    },
                    ..Default::default()
                }]));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let game = fixture.root.join("content-without-buildings");
    let (with_content, context) = crate::test_fixtures::source_with_content(frames, &game);
    assert!(!game.join("common/buildings").exists());
    fixture.replace(with_content.replay_ref());
    let source = fixture.source("raids-without-buildings");
    let flag = AtomicBool::new(false);
    let raids = RaidsParams {
        branch_id: "1".into(),
        view: RaidsView::Tally,
        tags: vec!["GER".into()],
        top: None,
    };
    let refusal = Raids::load(&source, &raids, &context, &flag)
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("no common/buildings file declares any building"),
        "{refusal}"
    );
    let accepted = run(&source, context.content.as_ref(), &flag, |_| {}).unwrap();
    assert_eq!(accepted.new_blocks, 1);
    assert!(accepted.coverage["1"].complete);
    let query = |product: &str, params: serde_json::Value| {
        super::super::query::query(
            &source,
            &crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params,
            },
            context.content.as_ref(),
            &flag,
        )
    };
    let group_params = CountryGroupParams {
        branch_id: "1".into(),
    };
    let expected_groups = CountryGroups::load(&source, &group_params, &context, &flag).unwrap();
    let groups: serde_json::Value = serde_json::from_str(
        &query(
            "country-groups",
            serde_json::to_value(&group_params).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        groups["data"],
        serde_json::to_value(expected_groups).unwrap()
    );
    // The content error still belongs to raids; it does not become an empty reply.
    assert_eq!(
        query("raids", serde_json::to_value(&raids).unwrap())
            .unwrap_err()
            .to_string(),
        refusal
    );
}

#[test]
fn project_completions_survive_a_missing_advisor_define() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::special_projects::{completion_events, row};
    use replay_data::{rows::LexicalTokenRow, rows::ProjectRow, rows::ProjectState};

    let frames: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000));
            frame.metrics.table_ns[TableId::Projects.index()] = 1;
            *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
                (1..=2)
                    .map(|local_id| ProjectRow {
                        owner: 11,
                        local_id,
                        definition_id: 8,
                        started: 1,
                        state: ProjectState {
                            current_offset: 0xe8,
                            not_completed: 0,
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::empty(TableId::ProjectHistory));
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 8,
                    literal: b"mod_project".to_vec(),
                }]));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let game = fixture.root.join("content-without-advisor-define");
    std::fs::create_dir_all(game.join("common/defines")).unwrap();
    std::fs::write(
        game.join("common/defines/synthetic.lua"),
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )
    .unwrap();
    let (with_content, context) = crate::test_fixtures::source_with_content(frames, &game);
    fixture.replace(with_content.replay_ref());
    let source = fixture.source("projects-without-advisor-define");
    let flag = AtomicBool::new(false);
    let accepted = run(&source, context.content.as_ref(), &flag, |_| {}).unwrap();
    assert_eq!(accepted.new_blocks, 1);
    assert!(accepted.coverage["1"].complete);

    let oracle = completion_events(&source, 1, &context, &flag).unwrap();
    let reason = "Project content, names and art unavailable: missing NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU";
    assert_eq!(oracle.refusals, [(reason.to_owned(), 1)]);
    assert_eq!(oracle.events.len(), 2);
    // The feed the projects family publishes, as read from the same hours.
    let feed = row::feed(&source, 1, &context, &flag).unwrap();
    assert_eq!(feed.content_error.as_deref(), Some(reason));
    let events: Vec<_> = feed
        .changes
        .into_iter()
        .filter_map(|change| change.completion)
        .collect();
    assert_eq!(events, oracle.events);
    assert!(events.iter().all(|event| event.name.is_none()
        && event.icon.is_none()
        && event.icon_missing.as_deref() == Some("Content is unavailable.")));
}

#[test]
fn prepared_country_identities_match_after_commits_resume_and_append() {
    let initial = recording();
    let replay = initial
        .append(&[records(3, vec![frame(3, 30, 2), frame(3, 31, 2)])])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("first");
    let flag = AtomicBool::new(false);
    assert!(
        crate::products::country_history::prepared_branch_histories(&source, 3, None, &flag,)
            .unwrap()
            .is_none()
    );
    let mut commits = Vec::new();
    assert_eq!(
        independent::with_test_one_after_another(|| run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |next| {
                commits.push(next);
                if next == 3 {
                    flag.store(true, Ordering::Release);
                }
                Ok(())
            }
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(commits, [1, 2, 3]);
    flag.store(false, Ordering::Release);
    // This run explicitly finishes the families first: branch 3 answers
    // from the identities' coverage although the walk stopped before it.
    assert!(
        !coverage(&source, None, 3, &flag)
            .unwrap()
            .is_some_and(|coverage| coverage.complete)
    );
    assert_prepared_identities(&source, 3);
    assert_prepared_identities(&source, 2);
    let resumed = fixture.source("resume");
    let mut visited = Vec::new();
    run_checked(
        &resumed,
        None,
        &flag,
        |_| {},
        |next| {
            visited.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(visited, [3]);
    assert_prepared_identities(&resumed, 3);
    let mut renamed = frame(3, 32, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"mod/renamed".to_vec();
    }
    let grown = replay
        .append(&[continuation_records(&replay, 3, vec![renamed])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("append");
    visited.clear();
    run_checked(
        &appended,
        None,
        &flag,
        |_| {},
        |next| {
            visited.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(visited, [4]);
    assert_prepared_identities(&appended, 3);
    let (store, key) = super::super::files(&appended, None).unwrap();
    assert_eq!(
        store.range_count(&key, "country-identities-3").unwrap(),
        Some(1)
    );
}

#[test]
fn cancellation_discards_whole_block_and_resume_visits_only_unaccepted_tail() {
    let replay = recording();
    assert_eq!(replay.index.len(), 2);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let mut published = Vec::new();
    let error = run_checked(
        &source,
        None,
        &flag,
        |accepted| published.push(accepted.clone()),
        |next| {
            if next == 2 {
                flag.store(true, Ordering::Release);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let mut published: Vec<_> = published
        .into_iter()
        .filter(|accepted| accepted.next_block > 0)
        .collect();
    published.dedup_by_key(|accepted| accepted.next_block);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].coverage["1"].first_hour, 10);
    assert_eq!(published[0].coverage["1"].last_hour, 11);
    assert!(published[0].coverage["1"].complete);
    assert!(
        !published[0].coverage.contains_key("2"),
        "uncommitted decoded blocks never reach status coverage"
    );

    assert_eq!(
        coverage(&source, None, 1, &AtomicBool::new(false))
            .unwrap()
            .unwrap()
            .last_hour,
        11
    );
    assert!(
        coverage(&source, None, 2, &AtomicBool::new(false))
            .unwrap()
            .is_none()
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert_eq!(saved.cursor.next_block, 1);
    assert!(store.read(&key, &branch_section(2), 0).unwrap().is_none());
    let original = store
        .read(&key, "country-identities-1", 0)
        .unwrap()
        .map(|mut page| {
            let mut bytes = Vec::new();
            page.read_to_end(&mut bytes).unwrap();
            bytes
        });
    flag.store(false, Ordering::Release);
    let mut accepted_blocks = Vec::new();
    let completed = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |next| {
            accepted_blocks.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(accepted_blocks, [2]);
    assert_eq!(
        completed
            .sections
            .iter()
            .find(|section| section.id == "country-identities")
            .unwrap()
            .state,
        State::Ready
    );
    assert_eq!(
        coverage(&source, None, 2, &flag)
            .unwrap()
            .unwrap()
            .last_hour,
        21
    );
    let (saved, bytes) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert_eq!(saved.cursor.next_block, 2);
    assert_eq!(bytes, completed.bytes);
    assert_eq!(store.range_count(&key, CHECKPOINT).unwrap(), Some(1));
    assert_eq!(store.range_count(&key, "map.resume").unwrap(), Some(1));
    let mut current = Vec::new();
    store
        .read(&key, "country-identities-1", 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut current)
        .unwrap();
    if let Some(original) = original {
        assert_eq!(current, original);
    }
    let mut notifications = 0;
    let mut processed = 0;
    let again = run_checked(
        &source,
        None,
        &flag,
        |_| {
            notifications += 1;
        },
        |_| {
            processed += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(notifications, 1);
    assert_eq!(processed, 0);
    assert_eq!(again.bytes, completed.bytes);
}

#[test]
fn appended_recording_binds_accepted_prefix_before_observing_new_frames() {
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let first = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&first, None, &flag, |_| {}).unwrap();
    let grown = old
        .append(&[continuation_records(
            &old,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let next = fixture.source("2");
    let (store, key) = super::super::files(&next, None).unwrap();
    let mut readable = Vec::new();
    run_checked(
        &next,
        None,
        &flag,
        |_| {},
        |_| {
            // Before the first new block commits, earlier ranges answer current-key reads.
            readable.push(store.read(&key, "country-identities-1", 0)?.is_some());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(readable, [true]);
}

#[test]
fn appended_recording_reuses_accepted_domain_state_and_only_prepares_new_blocks() {
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let first = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&first, None, &flag, |_| {}).unwrap();
    let grown = old
        .append(&[continuation_records(
            &old,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let next = fixture.source("2");
    assert!(next.is_append_of(&first));
    let mut blocks = Vec::new();
    let result = run_checked(
        &next,
        None,
        &flag,
        |_| {},
        |cursor| {
            blocks.push(cursor);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(blocks, [2]);
    assert!(result.bytes > 0);
    let range = coverage(&next, None, 1, &flag).unwrap().unwrap();
    assert_eq!(
        (range.first_hour, range.last_hour, range.complete),
        (10, 13, true)
    );
    let (store, key) = super::super::files(&next, None).unwrap();
    let branch = load_branch(
        &store,
        &key,
        Some(&key),
        next.replay_ref(),
        1,
        10,
        &Refused::default(),
    )
    .unwrap();
    assert_eq!(store.range_count(&key, "map.resume").unwrap(), Some(1));
    let mut second_run_blocks = 0;
    run_checked(
        &next,
        None,
        &flag,
        |_| {},
        |_| {
            second_run_blocks += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(second_run_blocks, 0);
}

#[test]
fn cancelled_initial_block_publishes_no_country_or_day_state() {
    let replay = recording();
    let check = || {
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("1");
        let flag = AtomicBool::new(false);
        assert!(
            run_checked(
                &source,
                None,
                &flag,
                |accepted| {
                    assert_eq!(accepted.next_block, 0);
                    assert!(accepted.coverage.is_empty());
                },
                |_| {
                    flag.store(true, Ordering::Release);
                    Ok(())
                },
            )
            .is_err()
        );
        // Cancellation can race with any family commit, including the first
        // one. Side jobs can also commit without advancing the walk.
        let (store, key) = super::super::files(&source, None).unwrap();
        if let Some((saved, _)) = checkpoint(&store, &key, None).unwrap() {
            assert_eq!(saved.cursor.next_block, 0);
            assert!(metadata_prefix(&saved, &replay, None).unwrap());
            let status = family_coverages(&saved, &replay, &last_blocks(&replay));
            for (name, family) in &saved.independent {
                assert!(family.next_block <= replay.index.len(), "{name}");
                for (&branch, &last) in &last_blocks(&replay) {
                    let accepted = replay.index[..family.next_block]
                        .iter()
                        .rev()
                        .find(|block| block.branch == branch);
                    let own = status
                        .get(name)
                        .and_then(|branches| branches.get(&branch.to_string()));
                    let page = own_page(&store, &key, name, branch).unwrap();
                    match accepted {
                        Some(block) => {
                            let own = own.expect("accepted family prefix");
                            assert_eq!(own.last_hour, block.last_hour, "{name}");
                            assert_eq!(own.complete, family.next_block > last, "{name}");
                            let page = page.expect("committed family coverage page");
                            assert_eq!(page.coverage.last_hour, own.last_hour, "{name}");
                            assert_eq!(page.coverage.complete, own.complete, "{name}");
                        }
                        None => {
                            assert!(own.is_none(), "{name}: no accepted branch");
                            assert!(page.is_none(), "{name}: no committed coverage page");
                        }
                    }
                }
            }
        } else {
            assert!(store.saved_key().unwrap().is_none());
        }
        for branch in [1, 2] {
            assert!(
                store
                    .read(&key, &branch_section(branch), 0)
                    .unwrap()
                    .is_none()
            );
            assert!(
                store
                    .read(&key, &coverage_section(branch), 0)
                    .unwrap()
                    .is_none()
            );
        }
        flag.store(false, Ordering::Release);
        let result = run(&source, None, &flag, |_| {});
        assert!(result.is_ok(), "{result:?}");
    };
    independent::with_test_hourly(check);
    check();
}

#[test]
fn recordings_of_the_same_campaign_keep_distinct_prepared_directories_and_clear_held_files() {
    let replay = recording();
    let mut left = Fixture::new(&replay);
    let mut right = Fixture::new(&replay);
    let one = left.source("1");
    let two = right.source("2");
    let flag = AtomicBool::new(false);
    let accepted = run(&one, None, &flag, |_| {}).unwrap();
    assert!(accepted.bytes > 0);
    let (left_store, left_key) = super::super::files(&one, None).unwrap();
    let (right_store, _) = super::super::files(&two, None).unwrap();
    assert!(right_store.saved_key().unwrap().is_none());
    let held = left_store
        .read(&left_key, &coverage_section(1), 0)
        .unwrap()
        .unwrap();
    super::super::clear(&one, None, &flag).unwrap();
    assert!(
        left_store
            .read(&left_key, &coverage_section(1), 0)
            .unwrap()
            .is_none()
    );
    let old: Coverage = serde_json::from_reader(std::io::BufReader::new(held)).unwrap();
    assert_eq!(old.last_hour, 11);
    assert_eq!(
        super::super::status(&one, "one", "request").state,
        State::Idle
    );
}

#[test]
fn unknown_checkpoint_schema_rebuilds_without_refusing_recording() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let file = store.read(&key, CHECKPOINT, 0).unwrap().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_reader(std::io::BufReader::new(file)).unwrap();
    value["schema"] = 999.into();
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at(
            CHECKPOINT,
            0,
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    assert!(restore(&source, None, &flag).unwrap().is_none());
    let rebuilt = run(&source, None, &flag, |_| {}).unwrap();
    assert_eq!(rebuilt.new_blocks, replay.index.len());
    assert!(
        source
            .replay_ref()
            .seek_selected(1, 10, Some(&[TableId::Countries]))
            .is_ok()
    );
}

#[test]
fn old_domain_accumulator_schema_rebuilds_completed_cache() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let section = branch_section(1);
    let mut saved = Vec::new();
    store
        .read(&key, &section, 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut saved)
        .unwrap();
    // The continuation is rewritten at every commit, so it is saved compressed.
    assert_eq!(saved[..4], [0x28, 0xb5, 0x2f, 0xfd]);
    let mut value: serde_json::Value =
        serde_json::from_reader(zstd::stream::Decoder::new(saved.as_slice()).unwrap()).unwrap();
    value["schema"] = 999.into();
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at(
            &section,
            0,
            zstd::bulk::compress(&serde_json::to_vec(&value).unwrap(), 1)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    assert!(restore(&source, None, &flag).unwrap().is_none());
    let rebuilt = run(&source, None, &flag, |_| {}).unwrap();
    assert_eq!(rebuilt.new_blocks, replay.index.len());
    assert_eq!(rebuilt.next_block, replay.index.len());
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 0);
}

#[test]
fn source_open_holds_a_file_revision_and_preserves_append_state_from_index_metadata() {
    let old = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(1, 10, 2)])]).unwrap();
    let mut fixture = Fixture::new(&old);
    let first = fixture.source("1");
    assert!(first.replay_ref().is_file_backed());
    first.append_state::<Vec<u64>>().lock().unwrap().push(9);
    let grown = old
        .append(&[continuation_records(&old, 1, vec![frame(1, 11, 2)])])
        .unwrap();
    fixture.replace(&grown);
    let next = fixture.source("2");
    next.inherit_append_states(&first);
    assert_eq!(*next.state::<Vec<u64>>().lock().unwrap(), [9]);
    assert!(
        first
            .replay_ref()
            .seek_selected(1, 10, Some(&[TableId::Countries]))
            .is_ok()
    );
    let changed =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(1, 10, 3)])]).unwrap();
    fixture.replace(&changed);
    let replacement = fixture.source("3");
    assert!(!replacement.is_append_of(&first));
}

#[test]
fn queued_cancellation_and_clear_wait_for_the_real_worker_to_release_its_writer() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let runner = super::super::runner::Runner::new().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let blockers: Vec<_> = (0..2).map(|_| Arc::new(AtomicBool::new(false))).collect();
    for flag in &blockers {
        let send = send.clone();
        runner
            .submit(Arc::clone(flag), move |cancelled| {
                send.send(()).unwrap();
                while !cancelled.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            })
            .unwrap();
    }
    for _ in 0..2 {
        receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
    super::super::start(
        &runner,
        Arc::clone(&source),
        None,
        super::restore(&source, None, &AtomicBool::new(false)),
        super::super::store_epoch(&source),
        super::super::change_runs::native_revision(source.path()).ok(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        super::super::status(&source, "source", "request").state,
        State::Queued
    );
    super::super::cancel(&source);
    for blocker in blockers {
        blocker.store(true, Ordering::Release);
    }
    super::super::wait_cancelled(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        super::super::status(&source, "source", "request").state,
        State::Cancelled
    );
    super::super::clear(&source, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        super::super::status(&source, "source", "request").state,
        State::Idle
    );
    let (store, _) = super::super::files(&source, None).unwrap();
    assert!(store.saved_key().unwrap().is_none());
}

#[test]
fn refused_block_preserves_the_accepted_prefix_and_its_country_pages() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let error = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |next| {
            if next == 2 {
                Err(invalid("Synthetic unsupported accepted block"))
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(coverage(&source, None, 1, &flag).unwrap().is_some());
    assert!(coverage(&source, None, 2, &flag).unwrap().is_none());
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_eq!(
        checkpoint(&store, &key, None)
            .unwrap()
            .unwrap()
            .0
            .cursor
            .next_block,
        1
    );
    let mut blocks = Vec::new();
    run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |next| {
            blocks.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(blocks, [2]);
}

#[test]
fn changed_old_index_and_compaction_shaped_metadata_rebuild_from_scratch() {
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let (mut state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    state.index[0].length += 1;
    assert!(!metadata_prefix(&state, &old, None).unwrap());
    // A changed offset/length is the shape of an old block republished by compaction.
    state.index[0].length -= 1;
    state.index[0].offset += 1;
    assert!(!metadata_prefix(&state, &old, None).unwrap());
    let mut writer = store.extend(&key).unwrap();
    json(&mut writer, CHECKPOINT, &state).unwrap();
    writer.commit().unwrap();
    let mut blocks = Vec::new();
    run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |cursor| {
            blocks.push(cursor);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(blocks, [1]);
    assert_eq!(
        checkpoint(&store, &key, None).unwrap().unwrap().0.index[0],
        Block::from(&old.index[0])
    );
}

#[test]
fn completed_cache_restores_before_queue_admission_and_old_schema_rebuilds() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let expected = run(&source, None, &flag, |_| {}).unwrap();
    let runner = super::super::runner::Runner::new().unwrap();
    super::super::start(
        &runner,
        Arc::clone(&source),
        None,
        super::restore(&source, None, &flag),
        super::super::store_epoch(&source),
        super::super::change_runs::native_revision(source.path()).ok(),
        &flag,
    )
    .unwrap();
    let status = super::super::status(&source, "source", "q");
    assert_eq!(status.state, State::Ready);
    assert_eq!(status.bytes, expected.bytes.to_string());
    let (store, key) = super::super::files(&source, None).unwrap();
    let file = store.read(&key, CHECKPOINT, 0).unwrap().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_reader(std::io::BufReader::new(file)).unwrap();
    value["schema"] = 999.into();
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at(
            CHECKPOINT,
            0,
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    assert!(restore(&source, None, &flag).unwrap().is_none());
    let rebuilt = run(&source, None, &flag, |_| {}).unwrap();
    assert_eq!(rebuilt.new_blocks, replay.index.len());
    super::super::start(
        &runner,
        Arc::clone(&source),
        None,
        super::restore(&source, None, &flag),
        super::super::store_epoch(&source),
        super::super::change_runs::native_revision(source.path()).ok(),
        &flag,
    )
    .unwrap();
    assert_eq!(
        super::super::status(&source, "source", "q2").state,
        State::Ready
    );
    assert!(
        source
            .replay_ref()
            .seek_selected(1, 10, Some(&[TableId::Countries]))
            .is_ok()
    );
}

#[test]
fn cached_raw_and_named_metadata_roundtrip_and_unknown_schema_falls_back() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    source.name_countries(None).unwrap();
    let expected = source.metadata().unwrap();
    run(&source, None, &flag, |_| {}).unwrap();
    let reopened = fixture.source("2");
    assert_eq!(run(&reopened, None, &flag, |_| {}).unwrap().new_blocks, 0);
    reopened.name_countries(None).unwrap();
    assert_eq!(reopened.metadata().unwrap(), expected);
    let (store, key) = super::super::files(&reopened, None).unwrap();
    let file = store.read(&key, "source-metadata-v1", 0).unwrap().unwrap();
    let mut value: serde_json::Value =
        serde_json::from_reader(std::io::BufReader::new(file)).unwrap();
    value["schema"] = 999.into();
    let mut writer = store.extend(&key).unwrap();
    writer
        .write_at(
            "source-metadata-v1",
            0,
            serde_json::to_vec(&value).unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    let fallback = fixture.source("3");
    assert_eq!(
        fallback.metadata().unwrap().countries.len(),
        expected.countries.len()
    );
}

#[test]
fn lightweight_map_binding_reads_the_published_map_page_before_whole_source_completion() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let error = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |next| {
            if next == 2 {
                Err(invalid("stop after one block"))
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "stop after one block");
    let binding = map_checkpoint_binding(&source, None).unwrap().unwrap();
    assert_eq!(binding.2.branches[&1].last_hour, Some(11));
    assert!(!binding.2.branches.contains_key(&2));
}

#[test]
fn first_preparation_key_refuses_recording_changed_since_source_open() {
    let replay = recording();
    let fixture = Fixture::new(&replay);
    let source = Source::open(
        crate::SourceKey {
            source_id: "stale".into(),
            revision: crate::protocol::replay_revision(&fixture.path).unwrap(),
        },
        fixture.path.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let appended = replay.append(&[records(3, vec![frame(3, 30, 2)])]).unwrap();
    fixture.replace(&appended);
    let error = super::super::files(&source, None).err().unwrap();
    assert!(error.to_string().contains("reopen"));
}

#[test]
fn one_branch_batches_many_replay_blocks_into_shared_commits() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 0, 1), frame(1, 1, 1)])],
    )
    .unwrap();
    for first in (2..32).step_by(2) {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![frame(1, first, 1), frame(1, first + 1, 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let mut commits = Vec::new();
    // The hourly walk reports its position as it reads; how many blocks one
    // read covers depends on how fast the walk runs, so only its end is fixed.
    let mut walked = std::collections::BTreeSet::new();
    let result = independent::with_test_hourly(|| {
        run_checked(
            &source,
            None,
            &flag,
            |progress| {
                if let Some(map) = progress
                    .sections
                    .iter()
                    .find(|section| section.id == "map" && section.state == State::Preparing)
                {
                    walked.insert(map.done.parse::<usize>().unwrap());
                }
            },
            |cursor| {
                commits.push(cursor);
                Ok(())
            },
        )
    })
    .unwrap();
    assert_eq!(
        commits.first(),
        Some(&1),
        "the first block publishes a view"
    );
    assert_eq!(commits.last(), Some(&replay.index.len()));
    assert!(commits.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(commits.iter().all(|&cursor| cursor <= replay.index.len()));
    assert_eq!(walked.last(), Some(&replay.index.len()), "{walked:?}");
    assert_eq!(result.new_blocks, replay.index.len());
    assert!(
        commits.len() < replay.index.len(),
        "publication still follows each replay block"
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    let saved = checkpoint(&store, &key, None).unwrap().unwrap().0;
    assert_eq!(saved.cursor.next_block, replay.index.len());
}

#[test]
fn prepared_groups_replace_one_state_across_commits_cancel_resume_and_append_at_scale() {
    use crate::products::{
        LoadContext, Product,
        country_groups::{CountryGroupParams, CountryGroups},
    };
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let middle = old
        .append(&[continuation_records(
            &old,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("2");
    assert!(
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            }
        )
        .is_err()
    );
    flag.store(false, Ordering::Release);
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let compare = |source: &Source, branch_id: &str| {
        let params = serde_json::json!({"branchId":branch_id});
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "country-groups".into(),
            params,
        };
        let reply: serde_json::Value = serde_json::from_str(
            &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let expected = CountryGroups::load(
            source,
            &CountryGroupParams {
                branch_id: branch_id.into(),
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        assert_eq!(reply["data"], serde_json::to_value(expected).unwrap());
        let (store, key) = super::super::files(source, None).unwrap();
        assert_eq!(
            store
                .range_count(&key, &format!("country-groups-{branch_id}"))
                .unwrap(),
            Some(1)
        );
    };
    compare(&source, "1");
    let grown = middle
        .append(&[continuation_records(&middle, 1, vec![frame(1, 15, 24)])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("3");
    run(&appended, None, &flag, |_| {}).unwrap();
    compare(&appended, "1");
}

#[test]
fn prepared_groups_sample_when_other_prepared_sections_exist_without_group_state() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("groups-pending");
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut writer = store.replace(key).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 11,
            complete: true,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "country-groups".into(),
        params: serde_json::json!({"branchId":"1"}),
    };
    let reply: serde_json::Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    let data = &reply["data"];
    assert!(data.is_object());
    assert_eq!(data["coverage"]["firstHour"], 10);
    assert_eq!(data["coverage"]["lastHour"], 9);
    assert_eq!(data["coverage"]["complete"], false);
    assert_eq!(data["pending"], serde_json::json!([]));
    assert_eq!(
        data["approximate"],
        serde_json::json!([{"firstHour": 10, "lastHour": 11}])
    );
    assert_eq!(reply["gate"], serde_json::json!(["groups"]));
}

#[test]
fn prepared_feed_matches_after_discarded_batch_cancel_resume_and_append() {
    // The hourly walk's own batches; prepared on its own, the feed commits
    // before the walk (`independent_feed`).
    independent::with_test_hourly(feed_discarded_batch_cancel_resume_and_append);
}

fn feed_discarded_batch_cancel_resume_and_append() {
    use crate::products::event_feed::prepared::tests::{assert_store_pages, frame};
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(100, 3), frame(101, 3)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("feed-first");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_store_pages(&source, &store, &key);
    let first_count = store.range_count(&key, "event-feed-facts-1").unwrap();
    let before_cancel = old
        .append(&[continuation_records(&old, 1, vec![frame(102, 3)])])
        .unwrap();
    // Only block 2 is present for this run, so its tail publication cannot
    // depend on the wall-clock batching budget or include block 3.
    fixture.replace(&before_cancel);
    let source = fixture.source("feed-cancel");
    let mut discarded = Vec::new();
    let error = run_checked(
        &source,
        None,
        &flag,
        |_| {},
        |next| {
            assert_eq!(next, 2);
            discarded.push(next);
            flag.store(true, Ordering::Release);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(discarded, [2]);
    flag.store(false, Ordering::Release);
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_eq!(
        checkpoint(&store, &key, None)
            .unwrap()
            .unwrap()
            .0
            .cursor
            .next_block,
        1,
        "discarding block 2 also preserves the committed decoder cursor"
    );
    assert_eq!(
        store.range_count(&key, "event-feed-facts-1").unwrap(),
        first_count
    );
    let mut facts =
        crate::prepared::series::Reader::new(&store, &key, "event-feed-facts-1".into(), &flag)
            .unwrap();
    assert!(
        facts.latest_event(1 | (1 << 63), 105).unwrap().is_none(),
        "the cancelled supplying read cannot publish its declaration"
    );
    let middle = before_cancel
        .append(&[continuation_records(
            &before_cancel,
            1,
            vec![frame(104, 3), frame(105, 3)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("feed-resume");
    let mut visited = Vec::new();
    let resumed = run(&source, None, &flag, |progress| {
        // Per-block progress reports each reconstructed block. Commit cursors
        // report batch endpoints and may legitimately contain only 3 here.
        let done = progress
            .sections
            .iter()
            .find(|section| section.id == "event-feed")
            .unwrap()
            .done
            .parse::<usize>()
            .unwrap();
        if done > 1 && visited.last().copied() != Some(done) {
            visited.push(done);
        }
    })
    .unwrap();
    assert_eq!(visited, [2, 3]);
    assert_eq!(resumed.new_blocks, 2);
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_store_pages(&source, &store, &key);
    let grown = middle
        .append(&[continuation_records(
            &middle,
            1,
            vec![frame(106, 3), frame(107, 3)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let source = fixture.source("feed-append");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_store_pages(&source, &store, &key);
}

#[test]
fn air_superiority_unprepared_regional_spans_remain_pending_without_a_frame_sampler() {
    let scored = |hour| {
        let mut frame = frame(1, hour, 2);
        *frame.tables[TableId::RaidAirScores.index()] =
            TableView::Complete(TableRows::RaidAirScores(vec![
                replay_data::rows::RaidAirScoreRow {
                    country: 100_000,
                    region: 7000,
                    friendly: i64::from(hour),
                    enemy: 1,
                    impact: Default::default(),
                    friendly_missions: 0,
                    enemy_missions: 0,
                    air_spotting_scale: Some(0),
                    menu: replay_data::rows::Recorded::Value(None),
                },
            ]));
        frame
    };
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scored(10), scored(11)])]).unwrap();
    let replay = first
        .append(&[continuation_records(
            &first,
            1,
            vec![scored(12), scored(13)],
        )])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("air-progressive-pending");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    // Readable keyframes can supply marked first views; an unreadable tail
    // must remain pending rather than inventing comparison scores.
    fixture.corrupt_block_preserving_stamp(&replay.index[1]);
    let query = |params: serde_json::Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "air-superiority".into(),
            params,
        };
        let reply: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let assert_pending = || {
        for (view, first, last) in [
            ("choices", 10, 13),
            ("timeline", 10, 13),
            ("moment", 12, 13),
            ("timeline", 12, 12),
            ("moment", 12, 12),
            // Historical side anchors remain approximate before completion.
            ("timeline", 10, 11),
        ] {
            let reply = query(serde_json::json!({"branchId":"1", "view":view,
                "firstHour":first, "lastHour":last, "regions":[7000]}));
            assert!(!reply.is_null(), "{view} {first}..{last}: {reply}");
            assert_eq!(
                reply["coverage"],
                serde_json::json!({
                    "schema":SCHEMA,"firstHour":10,"lastHour":9,"complete":false
                })
            );
            if last >= 12 {
                assert!(
                    reply["pending"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|span| span["firstHour"].as_i64().unwrap()
                            <= i64::from(12.max(first))
                            && span["lastHour"].as_i64().unwrap() >= i64::from(last)),
                    "the unreadable tail remains pending: {reply}"
                );
            } else {
                assert_eq!(reply["pending"], serde_json::json!([]));
                assert_eq!(
                    reply["approximate"],
                    serde_json::json!([{"firstHour":10,"lastHour":11}])
                );
            }
        }
    };
    assert_pending();
    assert!(store.saved_key().unwrap().is_none());
    let mut writer = store.replace(key.clone()).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 11,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    assert_pending();
    let windows = query(serde_json::json!({"branchId":"1", "view":"choices", "windowsOnly":true}));
    assert!(!windows["windows"].as_array().unwrap().is_empty());
    assert_eq!(
        store.range_count(&key, "air-superiority-1-values").unwrap(),
        None,
        "pending reads and index-only discovery do not prepare scores"
    );
}

#[test]
fn unprepared_blocks_detects_same_size_partial_work_and_appended_tail() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("detection");
    assert_eq!(unprepared_blocks(&source, None).unwrap(), None);
    let flag = AtomicBool::new(false);
    assert_eq!(
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |next| {
                if next == 2 {
                    flag.store(true, Ordering::Release);
                }
                Ok(())
            }
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    // The saved key already has this recording's full size; its cursor is partial.
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_eq!(store.saved_key().unwrap().unwrap(), key);
    assert_eq!(unprepared_blocks(&source, None).unwrap(), Some(1));
    flag.store(false, Ordering::Release);
    let resumed = fixture.source("detection-resume");
    assert_eq!(unprepared_blocks(&resumed, None).unwrap(), Some(1));
    assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 1);
    assert_eq!(unprepared_blocks(&resumed, None).unwrap(), Some(0));
    let appended = replay.append(&[records(3, vec![frame(3, 30, 2)])]).unwrap();
    fixture.replace(&appended);
    let grown = fixture.source("detection-grown");
    assert_eq!(unprepared_blocks(&grown, None).unwrap(), Some(1));
    assert_eq!(run(&grown, None, &flag, |_| {}).unwrap().new_blocks, 1);
    assert_eq!(unprepared_blocks(&grown, None).unwrap(), Some(0));
}

#[test]
fn unprepared_blocks_reads_metadata_without_decoding_recorded_blocks() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("metadata-only-detection");
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    fixture.corrupt_block_preserving_stamp(&replay.index[0]);
    // Size and mtime are the store's reuse contract. This fixture deliberately
    // keeps both while making any attempted frame decode fail.
    assert!(
        source
            .replay_ref()
            .read_block(&source.replay_ref().index[0])
            .is_err()
    );
    assert_eq!(unprepared_blocks(&source, None).unwrap(), Some(0));
}

#[test]
fn a_pause_discards_block_and_library_resume_rejects_changed_accepted_bytes() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let service = crate::Service::new(Arc::new(|_| true)).unwrap();
    let source = service.open_map(&fixture.path, None).unwrap();
    fixture
        .stores
        .push(super::super::files(&source, None).unwrap().0);
    let progress = source.state::<super::super::Progress>();
    let flag = {
        let mut progress = progress.lock().unwrap();
        progress.running = true;
        progress.automatic = true;
        Arc::clone(&progress.cancelled)
    };
    // The families prepared on their own commit beside the walk, so one can
    // still be folding the first block when the walk reaches the second. Such
    // a family's accepted prefix is empty and its resume rightly reads the
    // first block again. The pause arrives only once every family has
    // committed the first block, so the walk's second block is the whole tail.
    let families = independent::names();
    let committed = Arc::new((
        std::sync::Mutex::new(std::collections::BTreeSet::new()),
        std::sync::Condvar::new(),
    ));
    let published = Arc::clone(&committed);
    assert_eq!(
        independent::with_test_on_publish(
            move |family, next| {
                if next > 0 {
                    published.0.lock().unwrap().insert(family.to_owned());
                    published.1.notify_all();
                }
            },
            || run_checked(
                &source,
                None,
                &flag,
                |_| {},
                |next| {
                    if next == 2 {
                        let (accepted, _) = committed
                            .1
                            .wait_timeout_while(
                                committed.0.lock().unwrap(),
                                std::time::Duration::from_secs(120),
                                |accepted| accepted.len() < families.len(),
                            )
                            .unwrap();
                        assert_eq!(
                            accepted.iter().map(String::as_str).collect::<Vec<_>>(),
                            families,
                            "every family commits the first block beside the walk"
                        );
                        drop(accepted);
                        super::super::pause(&source, super::super::PauseReason::NewerRecording);
                    }
                    Ok(())
                }
            )
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    {
        let mut progress = progress.lock().unwrap();
        progress.running = false;
        progress.state = State::Cancelled;
    }
    assert_eq!(unprepared_blocks(&source, None).unwrap(), Some(1));
    // Same-stamp source corruption invalidates the fresh snow witness, even on an accepted prefix.
    fixture.corrupt_diagnostics_preserving_stamp(&replay, &replay.index[0]);
    assert!(
        source
            .replay_ref()
            .read_block(&source.replay_ref().index[0])
            .is_err()
    );
    source
        .replay_ref()
        .read_block(&source.replay_ref().index[1])
        .expect("synthetic tail must retain its native anchor dependencies");
    service
        .prepare_library(std::slice::from_ref(&fixture.path))
        .unwrap();
    let finished = Arc::clone(&progress.lock().unwrap().finished);
    let (done, _) = finished
        .1
        .wait_timeout_while(
            finished.0.lock().unwrap(),
            std::time::Duration::from_secs(120),
            |done| !*done,
        )
        .unwrap();
    assert!(*done);
    let status = super::super::status(&source, "resume-test", "resume-status");
    assert_eq!(
        status.state,
        State::Failed,
        "changed source bytes must not reuse completed exports: {:?}",
        status.error
    );
    assert_eq!(
        unprepared_blocks(&source, None).unwrap(),
        Some(replay.index.len())
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, store.saved_key().unwrap().as_ref())
        .unwrap()
        .unwrap();
    assert_eq!(state.cursor.next_block, 0);
    assert!(
        source
            .replay_ref()
            .read_block(&source.replay_ref().index[0])
            .is_err()
    );
    service.shutdown();
}

#[test]
fn prepared_leaders_pipeline_resumes_appended_outcomes_under_the_current_key() {
    use crate::products::{Product, unit_leaders::*};
    let frames = crate::products::unit_leaders::prepared::test_fixtures::outcome_frames(true);
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    let context = crate::products::LoadContext { content: None };
    for end in 2..=4 {
        if end > 2 {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    vec![frames[end - 1].clone()],
                )])
                .unwrap();
            fixture.replace(&replay);
        }
        let source = fixture.source(&format!("leader-append-{end}"));
        if end == 3 {
            assert!(
                run_checked(
                    &source,
                    None,
                    &flag,
                    |_| {},
                    |_| {
                        flag.store(true, Ordering::Release);
                        Ok(())
                    },
                )
                .is_err()
            );
            flag.store(false, Ordering::Release);
        }
        assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
        assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 0);
        let (store, key) = super::super::files(&source, None).unwrap();
        assert_eq!(store.range_count(&key, "unit-leaders-1").unwrap(), Some(1));
        assert_eq!(
            store.range_count(&key, "unit-leaders-events-1").unwrap(),
            Some((end - 1) as u64),
        );
        for (selection, view) in [
            (LeaderSelection::All, LeaderView::Leaderboard),
            (LeaderSelection::All, LeaderView::Choices),
            (
                LeaderSelection::Countries {
                    tags: vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()],
                },
                LeaderView::Choices,
            ),
            (
                LeaderSelection::Leader {
                    reference: ((1_u64 << 32) | 4713).to_string(),
                },
                LeaderView::Career,
            ),
        ] {
            let params = UnitLeadersParams {
                branch_id: "01".into(),
                selection,
                view,
                leaders: LeaderKinds::All,
            };
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "unit-leaders".into(),
                params: serde_json::to_value(&params).unwrap(),
            };
            let reply: serde_json::Value = serde_json::from_str(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            let oracle = if view == LeaderView::Choices {
                serde_json::to_value(load_choices(&source, &params, &context, &flag).unwrap())
            } else {
                serde_json::to_value(UnitLeaders::load(&source, &params, &context, &flag).unwrap())
            }
            .unwrap();
            let mut data = reply["data"].clone();
            assert_eq!(data["coverage"]["complete"], true);
            assert_eq!(data["pending"], serde_json::json!([]));
            for field in ["coverage", "pending"] {
                data.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(data, oracle, "append={end} view={view:?}");
        }
    }
}

#[test]
fn preparation_branch_state_has_a_small_heap_owned_layout() {
    assert!(
        std::mem::size_of::<BranchState>() <= 1024,
        "BranchState must hold heap-owned observers, got {} bytes",
        std::mem::size_of::<BranchState>()
    );
}

#[test]
fn production_stack_prepare_resume_append_and_query_roundtrips() {
    let worker = super::super::runner::worker_thread("preparation stack regression")
        .spawn(|| {
            // Fail before entering a large deserializer on the old representation.
            assert!(std::mem::size_of::<BranchState>() <= 1024);
            assert_eq!(super::super::runner::WORKER_STACK_BYTES, 2 * 1024 * 1024);
            let replay = recording();
            let mut fixture = Fixture::new(&replay);
            let flag = AtomicBool::new(false);
            let mut queries = 0;
            let mut check_reads = |source: &Source| {
                assert!(restore(source, None, &flag).unwrap().is_some());
                let (store, key) = super::super::files(source, None).unwrap();
                for branch in &source.replay_ref().branches {
                    let index = source
                        .replay_ref()
                        .index
                        .iter()
                        .find(|index| index.branch == branch.id)
                        .unwrap();
                    let saved = load_branch(
                        &store,
                        &key,
                        Some(&key),
                        source.replay_ref(),
                        branch.id,
                        index.first_hour,
                        &Refused::default(),
                    )
                    .unwrap();
                    let bytes = serde_json::to_vec(&saved).unwrap();
                    let decoded: Box<BranchState> =
                        serde_json::from_reader(bytes.as_slice()).unwrap();
                    assert_eq!(
                        serde_json::to_value(&decoded).unwrap(),
                        serde_json::to_value(&saved).unwrap()
                    );
                    for product in ["country-groups", "construction-log-manifest"] {
                        let request = crate::transport::ProductRequest {
                            source_id: source.key().source_id.clone(),
                            revision: source.key().revision.clone(),
                            product: product.into(),
                            params: serde_json::json!({"branchId":branch.id.to_string()}),
                        };
                        let reply: serde_json::Value = serde_json::from_str(
                            &super::super::query::query(source, &request, None, &flag).unwrap(),
                        )
                        .unwrap();
                        assert!(!reply["data"].is_null());
                        queries += 1;
                    }
                }
            };
            let full = fixture.source("stack-full");
            assert_eq!(run(&full, None, &flag, |_| {}).unwrap().new_blocks, 2);
            check_reads(&full);
            super::super::files(&full, None).unwrap().0.clear().unwrap();
            let partial = fixture.source("stack-partial");
            assert_eq!(
                run_checked(
                    &partial,
                    None,
                    &flag,
                    |_| {},
                    |next| {
                        if next == 2 {
                            flag.store(true, Ordering::Release);
                        }
                        Ok(())
                    }
                )
                .unwrap_err()
                .kind(),
                io::ErrorKind::Interrupted
            );
            flag.store(false, Ordering::Release);
            assert_eq!(unprepared_blocks(&partial, None).unwrap(), Some(1));
            let resumed = fixture.source("stack-resumed");
            assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 1);
            check_reads(&resumed);
            let appended = replay
                .append(&[records(3, vec![frame(3, 30, 2), frame(3, 31, 2)])])
                .unwrap();
            fixture.replace(&appended);
            let grown = fixture.source("stack-appended");
            assert_eq!(run(&grown, None, &flag, |_| {}).unwrap().new_blocks, 1);
            check_reads(&grown);
            assert_eq!(
                queries, 14,
                "three finite phases must issue exactly their branch queries"
            );
        })
        .unwrap();
    worker
        .join()
        .expect("preparation and queries must fit the production worker stack");
}
