use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
use replay::{
    frame::TableView,
    publication::replace_file,
    rows::{CompletedFocusRow, FocusRow, TableRows},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod casualty_pending;
mod catalogue_refusals;
mod equipment_loss_pending;
mod family_lanes;
mod family_refusal;
mod independent_families;
mod independent_groups;
mod parallel_flush;
mod prepass_parity;
mod serialization_order;
mod stability_progressive;

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    let (_, timing) = run_measured(
        &source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        |_| {},
    )
    .unwrap();
    let families = timing.families.expect("measured call sites fill the table");
    assert_eq!(families["focus.write_frame"].calls, 2);
    assert_eq!(families["naval.observe"].calls, 2);
    assert_eq!(families["naval.observe_details"].calls, 2);
    assert_eq!(families["store_commit"].calls, 1);
    assert!(families.values().all(|family| family.calls > 0));
    for name in [
        "politics_ledger",
        "stability_parts",
        "setup.files",
        "setup.state",
        "setup.country",
        "setup.ordinals",
        "setup.overhead",
        "content.land_names",
        "content.production",
        "context.construct",
        "construct.decoder",
        "focus.construct",
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

fn forces_frame(hour: i32) -> DecodedFrame {
    let mut frame = crate::products::army_size::test_fixtures::frame(hour);
    let divisions = frame.complete::<replay::rows::tables::Divisions>().unwrap();
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            divisions
                .iter()
                .map(|row| replay::rows::DivisionStatusRow {
                    division: row.id,
                    supply_ratio: if hour % 2 == 0 { 0 } else { 100_000 },
                    supply_grace_maximum: replay::rows::Recorded::Value(1_000_000),
                    supply_grace_remaining: replay::rows::Recorded::Value(0),
                    ..Default::default()
                })
                .collect(),
        ));
    let divisions = frame.complete::<replay::rows::tables::Divisions>().unwrap();
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(
            divisions
                .iter()
                .map(|row| replay::rows::DivisionLandInputsRow {
                    division: row.id,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

fn assert_forces(source: &Source, context: &crate::products::LoadContext) {
    use crate::products::{Product, army_size::*, supply_health::*};
    let flag = AtomicBool::new(false);
    for measure in [
        ArmyMeasure::Divisions,
        ArmyMeasure::Strength,
        ArmyMeasure::Battalions,
        ArmyMeasure::Manpower,
    ] {
        for count_for in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            for top in [None, Some(1), Some(5000)] {
                for window in 0..source
                    .replay_ref()
                    .index
                    .iter()
                    .filter(|block| block.branch == 1)
                    .count()
                {
                    let params = ArmySizeParams {
                        branch_id: "1".into(),
                        tags: if top.is_some() {
                            Vec::new()
                        } else {
                            vec!["origin".into()]
                        },
                        top,
                        measure,
                        count_for,
                        by_type: measure == ArmyMeasure::Battalions,
                        window: Some(window),
                    };
                    let old = ArmySize::load(source, &params, context, &flag).unwrap();
                    let new = crate::products::army_size::prepared::query(
                        source, &params, context, &flag,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(
                        serde_json::to_value(&new).unwrap(),
                        serde_json::to_value(&old).unwrap(),
                        "{measure:?} {count_for:?} top={top:?} window={window}"
                    );
                }
            }
        }
    }
    let params = SupplyHealthParams {
        branch_id: "1".into(),
        tags: vec!["origin".into()],
    };
    let old = SupplyHealth::load(source, &params, context, &flag).unwrap();
    let new = crate::products::supply_health::prepared::query(
        source,
        &serde_json::json!({"branchId": params.branch_id, "tags": params.tags, "from":100, "to":1000}),
        context, &flag,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::to_value(&new).unwrap(),
        serde_json::to_value(&old).unwrap()
    );
    let last = source.block_windows(1).last().unwrap().last_hour;
    let dense = crate::products::supply_health::prepared::query(
        source,
        &serde_json::json!({"branchId": params.branch_id, "tags": params.tags, "from":100, "to":last, "width":1}),
        context,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert_eq!(dense.summaries.as_ref().unwrap().len(), dense.series.len());
    for series in &dense.series {
        for run in &series.runs {
            let supplying = old
                .series
                .iter()
                .find(|old| old.tag == series.tag)
                .unwrap()
                .runs
                .iter()
                .find(|old| old.first_hour <= run.first_hour && old.last_hour >= run.last_hour)
                .unwrap();
            assert_eq!(
                (run.no_supply, run.strained, run.in_supply),
                (supplying.no_supply, supplying.strained, supplying.in_supply)
            );
        }
    }
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "series-next-change".into(),
        params: serde_json::json!({"branchId":"1","family":{"kind":"supply-health"},"tags":["origin"],"key":"ArrowRight","from":100,"firstHour":100,"lastHour":last}),
    };
    let value: serde_json::Value = serde_json::from_str(
        &super::super::query::query(source, &request, context.content.as_ref(), &flag).unwrap(),
    )
    .unwrap();
    assert_eq!(value["data"]["hour"].as_f64(), Some(101.0));
    for family in [
        serde_json::json!({"kind":"supply-health"}),
        serde_json::json!({"kind":"army-size","measure":"divisions","countFor":"owner"}),
        serde_json::json!({"kind":"army-size","measure":"divisions","countFor":"owner","top":1}),
    ] {
        let tags = if family["top"].is_null() {
            vec!["origin"]
        } else {
            vec![]
        };
        for (key, from, expected) in [
            ("Home", None, 100.5),
            ("ArrowRight", Some(last as f64), last as f64),
        ] {
            let request = crate::transport::ProductRequest {
                params: serde_json::json!({"branchId":"1","family":family,"tags":tags,
                    "key":key,"from":from,"firstHour":100.5,"lastHour":last}),
                ..request.clone()
            };
            let value: serde_json::Value = serde_json::from_str(
                &super::super::query::query(source, &request, context.content.as_ref(), &flag)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(value["data"]["hour"], expected, "{family} {key}");
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_forces_match_oracles_across_batches_resume_append_and_aliases() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for composition parity");
    let mut first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![forces_frame(100), forces_frame(101)])],
    )
    .unwrap();
    let mut replay = first
        .append(&[continuation_records(&first, 1, vec![forces_frame(102)])])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let game = fixture.root.join("content");
    std::fs::create_dir_all(game.join("common/defines")).unwrap();
    std::fs::create_dir_all(game.join("user")).unwrap();
    std::fs::write(game.join("hoi4.exe"), "test").unwrap();
    std::fs::write(
        game.join("user/dlc_load.json"),
        r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    std::fs::write(
        game.join("user/settings.txt"),
        "language=l_english force_pow2_textures=no",
    )
    .unwrap();
    std::fs::write(game.join("common/defines/synthetic.lua"), "NDefines={NMilitary={SUPPLY_GRACE=10,LOW_SUPPLY=.6},NGraphics={SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.15,SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.4}}").unwrap();
    let (options, identity) = crate::test_fixtures::content_identity(&game);
    let mut wire = records(1, vec![forces_frame(100), forces_frame(101)]);
    if let replay::spool::Record::SessionStart(session) = &mut wire[0] {
        session.identity = identity;
    }
    first = Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap();
    replay = first
        .append(&[continuation_records(&first, 1, vec![forces_frame(102)])])
        .unwrap();
    let context = crate::products::LoadContext {
        content: Some(options),
    };
    fixture.replace(&first);
    let source = fixture.source("forces");
    let flag = AtomicBool::new(false);
    run(&source, context.content.as_ref(), &flag, |_| {}).unwrap();
    assert_forces(&source, &context);
    fixture.replace(&replay);
    let source = fixture.source("forces-tail");
    assert_eq!(
        run_checked(
            &source,
            context.content.as_ref(),
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
    let resumed = fixture.source("forces-resume");
    let mut visited = Vec::new();
    run_checked(
        &resumed,
        context.content.as_ref(),
        &flag,
        |_| {},
        |next| {
            visited.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(visited, [2]);
    assert_forces(&resumed, &context);
    let mut renamed = forces_frame(103);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"later alias".to_vec();
    }
    let grown = replay
        .append(&[continuation_records(&replay, 1, vec![renamed])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("forces-append");
    visited.clear();
    run_checked(
        &appended,
        context.content.as_ref(),
        &flag,
        |_| {},
        |next| {
            visited.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(visited, [3]);
    assert_forces(&appended, &context);
}

#[test]
fn prepared_pipeline_keeps_ledger_and_other_families_when_advisor_define_is_missing() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for ledger content parity");
    use crate::products::{
        Product,
        country_measure::{CountryMeasure, Measure, MeasureParams},
        politics_ledger::{LedgerView, PoliticsLedger, PoliticsLedgerParams, PoliticsLedgerReply},
    };

    let frames: Vec<_> = (100..=104)
        .map(|hour| crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000)))
        .collect();
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let game = fixture.root.join("ledger-content-without-advisor-define");
    std::fs::create_dir_all(game.join("common/defines")).unwrap();
    std::fs::write(
        game.join("common/defines/synthetic.lua"),
        "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
    )
    .unwrap();
    let (with_content, context) =
        crate::test_fixtures::source_with_content(frames[..2].to_vec(), &game);
    let first = with_content.replay_ref();
    let second = first
        .append(&[continuation_records(first, 1, frames[2..4].to_vec())])
        .unwrap();
    let replay = second
        .append(&[continuation_records(&second, 1, frames[4..].to_vec())])
        .unwrap();
    fixture.replace(first);
    let source = fixture.source("ledger-missing-advisor-prefix");
    let flag = AtomicBool::new(false);
    let params = PoliticsLedgerParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        view: LedgerView::Raw,
    };
    let refusal = PoliticsLedger::load(&source, &params, &context, &flag)
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU"),
        "{refusal}"
    );
    let check = |source: &Source, hour: i32| {
        let (store, key) = super::super::files(source, context.content.as_ref()).unwrap();
        let reply: PoliticsLedgerReply = serde_json::from_value(
            crate::products::politics_ledger::prepared::query(
                &serde_json::json!({"branchId":"1", "tag":"GER", "view":"raw", "hour":hour}),
                &store,
                &key,
                &flag,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            reply
                .warnings
                .iter()
                .filter(|warning| warning.contains(&refusal))
                .collect::<Vec<_>>(),
            [&format!("{refusal} (count: 1)")]
        );
        assert!(reply.mutations.is_empty());
        let expected = CountryMeasure::load(
            source,
            &MeasureParams {
                branch_id: "1".into(),
                measure: Measure::PoliticalPower,
                tags: vec!["GER".into()],
            },
            &context,
            &flag,
        )
        .unwrap();
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "series-state".into(),
            params: serde_json::json!({"branchId":"1", "family":{"kind":"country-measure", "measure":"politicalPower"}, "reads":[{"tag":"GER", "hour":hour}]}),
        };
        let actual: serde_json::Value = serde_json::from_str(
            &super::super::query::query(source, &request, context.content.as_ref(), &flag).unwrap(),
        )
        .unwrap();
        let expected = expected.series[0]
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap();
        assert_eq!(
            actual["data"]["states"][0]["run"]["valueRaw"],
            expected.value_raw
        );
    };
    run(&source, context.content.as_ref(), &flag, |_| {}).unwrap();
    check(&source, 101);
    fixture.replace(&second);
    let source = fixture.source("ledger-missing-advisor-tail");
    assert_eq!(
        run_checked(
            &source,
            context.content.as_ref(),
            &flag,
            |_| {},
            |next| {
                if next == 2 {
                    flag.store(true, Ordering::Release);
                }
                Ok(())
            },
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    flag.store(false, Ordering::Release);
    check(&source, 101);
    fixture.replace(&replay);
    let source = fixture.source("ledger-missing-advisor-resume");
    let accepted = run(&source, context.content.as_ref(), &flag, |_| {}).unwrap();
    assert_eq!(accepted.new_blocks, 2);
    assert!(accepted.coverage["1"].complete);
    check(&source, 104);
}

#[test]
fn prepared_pipeline_keeps_other_families_when_raid_buildings_are_missing() {
    use crate::products::{
        Product,
        country_groups::{CountryGroupParams, CountryGroups},
        country_measure::{CountryMeasure, Measure, MeasureParams},
        raids::{Raids, RaidsParams, RaidsView},
    };
    use engine::craid_instance as native;
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for raid content metadata");
    let frames: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000));
            frame.metrics.table_ns[TableId::Raids.index()] = 1;
            *frame.tables[TableId::RaidCountries.index()] =
                TableView::Complete(TableRows::RaidCountries(vec![
                    replay::rows::RaidCountryRow {
                        canonical_slot: 7,
                        country: 11,
                        eligible_types: vec![50],
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::Raids.index()] =
                TableView::Complete(TableRows::Raids(vec![replay::rows::RaidRow {
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
    let balances = CountryMeasure::load(
        &source,
        &MeasureParams {
            branch_id: "1".into(),
            measure: Measure::PoliticalPower,
            tags: vec!["GER".into()],
        },
        &context,
        &flag,
    )
    .unwrap();
    for hour in [100, 101] {
        let state: serde_json::Value = serde_json::from_str(&query(
            "series-state",
            serde_json::json!({"branchId":"1", "family":{"kind":"country-measure", "measure":"politicalPower"}, "reads":[{"tag":"GER", "hour":hour}]}),
        ).unwrap()).unwrap();
        let expected = balances.series[0]
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap();
        assert_eq!(
            state["data"]["states"][0]["run"]["valueRaw"],
            expected.value_raw
        );
    }
    // The legacy content error still belongs to raids; it does not become an empty reply.
    assert_eq!(
        query("raids", serde_json::to_value(&raids).unwrap())
            .unwrap_err()
            .to_string(),
        refusal
    );
    let without_content =
        Raids::load(&source, &raids, &LoadContext { content: None }, &flag).unwrap();
    let expected = without_content.series[0].runs.last().unwrap();
    assert_eq!((expected.launched, expected.succeeded), (1, 1));
    let (store, key) = super::super::files(&source, context.content.as_ref()).unwrap();
    let (keys, ()) = super::super::series::read_catalogue::<()>(&store, &key, "raids", 1)
        .unwrap()
        .unwrap();
    let mut tallies =
        super::super::series::Reader::new(&store, &key, "raids-1-tallies".into(), &flag).unwrap();
    let value = tallies.value_at(keys["GER"], 101).unwrap().unwrap().value;
    assert_eq!(
        ((value >> 32) as u32, (value & i128::from(u32::MAX)) as u32),
        (expected.launched, expected.succeeded)
    );
}

#[test]
fn prepared_pipeline_keeps_project_completions_when_advisor_define_is_missing() {
    use super::super::series::{self, Reader};
    use crate::products::special_projects::{completion_events, prepared::completion_stream};
    use replay::rows::{LexicalTokenRow, ProjectRow, ProjectState};

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
    let (store, key) = super::super::files(&source, context.content.as_ref()).unwrap();
    let (_, metadata) = series::read_catalogue::<completion_stream::Metadata>(
        &store,
        &key,
        completion_stream::FAMILY,
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(metadata.content_error.as_deref(), Some(reason));
    let mut reader = Reader::new(&store, &key, completion_stream::section(1), &flag).unwrap();
    let page = reader
        .events(&[completion_stream::KEY], 100, 101, 0, 2)
        .unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(page.cursor.is_none());
    let changes: Vec<completion_stream::Change> = page
        .rows
        .into_iter()
        .map(|row| serde_json::from_value(row.row).unwrap())
        .collect();
    let events: Vec<_> = changes
        .into_iter()
        .filter_map(|row| row.completion)
        .collect();
    assert_eq!(events, oracle.events);
    assert!(events.iter().all(|event| event.name.is_none()
        && event.icon.is_none()
        && event.icon_missing.as_deref() == Some("Content is unavailable.")));
    let reasons: Vec<_> = metadata.refusal_keys.keys().cloned().collect();
    let reason_keys: Vec<_> = metadata.refusal_keys.values().copied().collect();
    for live in [100, 101] {
        let counts = reader.values_at(&reason_keys, live).unwrap();
        let refusals: Vec<_> = reasons
            .iter()
            .cloned()
            .zip(counts)
            .filter_map(|(reason, count)| {
                count
                    .filter(|count| *count > 0)
                    .map(|count| (reason, count as usize))
            })
            .collect();
        assert_eq!(refusals, oracle.refusals, "Live {live}");
    }
    let state: serde_json::Value = serde_json::from_str(
        &super::super::query::query(
            &source,
            &crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "series-state".into(),
                params: serde_json::json!({"branchId":"1", "family":{"kind":"country-measure", "measure":"politicalPower"}, "reads":[{"tag":"GER", "hour":101}]}),
            },
            context.content.as_ref(),
            &flag,
        ).unwrap(),
    ).unwrap();
    assert_eq!(state["data"]["states"][0]["run"]["valueRaw"], "101000");
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
        run_checked(
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
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(commits, [1, 2, 3]);
    flag.store(false, Ordering::Release);
    assert!(
        crate::products::country_history::prepared_branch_histories(&source, 3, None, &flag,)
            .unwrap()
            .is_none()
    );
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_country_identities_keep_all_5000_countries() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![scale_frame(1, 10, 5000), scale_frame(1, 11, 5000)],
        )],
    )
    .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("scale");
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    assert_prepared_identities(&source, 1);
    let history = crate::products::country_history::prepared_branch_histories(
        &source,
        1,
        None,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(history.countries.len(), 5000);
    assert!(
        history
            .countries
            .values()
            .all(|country| country.identities.len() == 1)
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_army_top_first_page_reads_prepared_ranks_at_5000_countries() {
    use crate::products::army_size::*;
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 5000);
        *frame.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(
                (0..5000)
                    .map(|index| replay::rows::DivisionCensusRow {
                        country: 100_000 + index,
                        grouped: index % 101,
                        ungrouped: 0,
                    })
                    .collect(),
            ));
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("rank-scale");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let params = ArmySizeParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        top: Some(10),
        measure: ArmyMeasure::Divisions,
        count_for: ArmyAttribution::Owner,
        by_type: false,
        window: Some(0),
    };
    let context = crate::products::LoadContext { content: None };
    let started = Instant::now();
    let reply = crate::products::army_size::prepared::query(&source, &params, &context, &flag)
        .unwrap()
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(reply.series.len(), 10);
    assert!(
        reply
            .series
            .iter()
            .all(|series| series.runs.iter().all(|run| run.raw == "100"))
    );
    use crate::products::Product;
    let oracle = ArmySize::load(&source, &params, &context, &flag).unwrap();
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::to_value(&oracle).unwrap()
    );
    eprintln!(
        "prepared army top-10 first page: 5,000 countries, {:?}, {} bytes (debug test profile)",
        elapsed,
        serde_json::to_vec(&reply).unwrap().len()
    );
}

#[test]
fn prepared_country_identity_state_grows_with_changes_not_hours() {
    let mut scan = crate::products::country_history::HistoryScan::new(false, &[]);
    let mut frame = scale_frame(1, 0, 32);
    scan.observe(&frame);
    let first = serde_json::to_vec(&scan).unwrap().len();
    for hour in 1..17_520 {
        frame.hour = hour;
        scan.observe(&frame);
    }
    let bytes = serde_json::to_vec(&scan).unwrap().len();
    let history = scan.finish();
    assert!(
        history
            .countries
            .values()
            .all(|country| country.identities.len() == 1)
    );
    assert!(bytes < first + 1000);
    eprintln!(
        "prepared identities: 32 countries, 17,520 hours, {bytes} bytes (first hour {first})"
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    let published: Vec<_> = published
        .into_iter()
        .filter(|accepted| {
            accepted
                .sections
                .iter()
                .any(|section| section.state == State::Ready)
        })
        .collect();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].coverage["1"].first_hour, 10);
    assert_eq!(published[0].coverage["1"].last_hour, 11);
    assert!(published[0].coverage["1"].complete);
    assert!(
        !published[0].coverage.contains_key("2"),
        "uncommitted decoded blocks never reach status coverage"
    );

    assert_eq!(
        published[0]
            .sections
            .iter()
            .find(|section| section.id == "focus")
            .unwrap()
            .state,
        State::Ready
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
    let mut old_focus = store.read(&key, "focus-1", 0).unwrap().unwrap();
    let mut original = Vec::new();
    old_focus.read_to_end(&mut original).unwrap();
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
            .find(|section| section.id == "focus")
            .unwrap()
            .state,
        State::Ready
    );
    assert_eq!(
        completed
            .sections
            .iter()
            .find(|section| section.id == "focus-display")
            .unwrap()
            .state,
        State::Failed
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
        .read(&key, "focus-1", 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut current)
        .unwrap();
    assert_eq!(current, original);
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
            let request = crate::transport::ProductRequest { source_id: next.key().source_id.clone(), revision: next.key().revision.clone(), product: "series-state".into(),
                params: serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"reads":[{"tag":String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap(),"hour":13}]}) };
            let reply: serde_json::Value = serde_json::from_str(&super::super::query::query(&next, &request, None, &AtomicBool::new(false))?)?;
            assert_eq!(reply["data"]["coverage"]["lastHour"], 11);
            assert_eq!(reply["data"]["states"][0]["run"]["valueRaw"], "13000");
            assert_eq!(reply["data"]["states"][0]["approximate"], false);

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
    assert_eq!(branch.measure.next_hour(), 14);
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
    // Cancels before the hourly walk's first commit.
    let cancelled = |source: &Source, flag: &AtomicBool| {
        run_checked(
            source,
            None,
            flag,
            |accepted| {
                assert!(
                    accepted
                        .sections
                        .iter()
                        .all(|section| section.state == State::Preparing)
                )
            },
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
        .is_err()
    };
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    independent::with_test_hourly(|| assert!(cancelled(&source, &flag)));
    let (store, key) = super::super::files(&source, None).unwrap();
    assert!(!store.is_reusable(&key).unwrap());
    assert!(store.saved_key().unwrap().is_none());
    flag.store(false, Ordering::Release);
    assert!(run(&source, None, &flag, |_| {}).is_ok());

    // Families prepared on their own commit before the hourly walk, so only
    // their progress is kept: the walk accepted nothing.
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    assert!(cancelled(&source, &flag));
    let (store, key) = super::super::files(&source, None).unwrap();
    let saved = checkpoint(&store, &key, None).unwrap().unwrap().0;
    assert_eq!(saved.cursor.next_block, 0);
    assert!(
        saved
            .independent
            .values()
            .all(|family| family.next_block == replay.index.len())
    );
    flag.store(false, Ordering::Release);
    assert!(run(&source, None, &flag, |_| {}).is_ok());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    let old: Coverage = serde_json::from_reader(held).unwrap();
    assert_eq!(old.last_hour, 11);
    assert_eq!(
        super::super::status(&one, "one", "request").state,
        State::Idle
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn unknown_checkpoint_schema_rebuilds_without_refusing_recording() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let file = store.read(&key, CHECKPOINT, 0).unwrap().unwrap();
    let mut value: serde_json::Value = serde_json::from_reader(file).unwrap();
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    value["focus"]["schema"] = 999.into();
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
fn five_thousand_country_accumulators_retain_current_inputs_not_campaign_frames() {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(1, 10, 1)])]).unwrap();
    let mut state = BranchState::new(&replay, 1, 10);
    let flag = AtomicBool::new(false);
    for hour in 10..14 {
        let decoded = frame(1, hour, 5_000);
        let mut countries = 0;
        state
            .focus
            .observe(&decoded, &flag, |_| {
                countries += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(countries, 5_000);
        state
            .production
            .ingest(
                replay.format,
                &decoded,
                crate::products::equipment::production::Named {
                    units: None,
                    texts: None,
                },
                &flag,
            )
            .unwrap();
        state.production.flush();
        state.measure.observe(&decoded).unwrap();
    }
    state.measure.validate_checkpoint().unwrap();
    let json = serde_json::to_value(&state).unwrap();
    assert!(json.get("frames").is_none());
    let decoded: Box<BranchState> = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.measure.next_hour(), 14);
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
    super::super::start(&runner, Arc::clone(&source), None, &AtomicBool::new(false)).unwrap();
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn completed_cache_restores_before_queue_admission_and_old_schema_rebuilds() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    let expected = run(&source, None, &flag, |_| {}).unwrap();
    let runner = super::super::runner::Runner::new().unwrap();
    super::super::start(&runner, Arc::clone(&source), None, &flag).unwrap();
    let status = super::super::status(&source, "source", "q");
    assert_eq!(status.state, State::Ready);
    assert_eq!(status.bytes, expected.bytes.to_string());
    let (store, key) = super::super::files(&source, None).unwrap();
    let file = store.read(&key, CHECKPOINT, 0).unwrap().unwrap();
    let mut value: serde_json::Value = serde_json::from_reader(file).unwrap();
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
    super::super::start(&runner, Arc::clone(&source), None, &flag).unwrap();
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
    source.name_countries(None);
    let expected = source.metadata();
    run(&source, None, &flag, |_| {}).unwrap();
    let reopened = fixture.source("2");
    assert_eq!(run(&reopened, None, &flag, |_| {}).unwrap().new_blocks, 0);
    reopened.name_countries(None);
    assert_eq!(reopened.metadata(), expected);
    let (store, key) = super::super::files(&reopened, None).unwrap();
    let file = store.read(&key, "source-metadata-v1", 0).unwrap().unwrap();
    let mut value: serde_json::Value = serde_json::from_reader(file).unwrap();
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
        fallback.metadata().countries.len(),
        expected.countries.len()
    );
}

#[test]
fn lightweight_map_binding_reads_the_published_map_page_before_whole_source_completion() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run_checked(
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn one_branch_batches_many_replay_blocks_into_shared_measure_files() {
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
    let mut commits = 0;
    let mut blocks = 0;
    // The hourly walk's own reports, one per replay block.
    let result = independent::with_test_hourly(|| {
        run_checked(
            &source,
            None,
            &flag,
            |progress| {
                if progress
                    .sections
                    .iter()
                    .all(|section| section.state == State::Preparing)
                {
                    blocks += 1;
                }
            },
            |_| {
                commits += 1;
                Ok(())
            },
        )
    })
    .unwrap();
    assert_eq!(blocks, replay.index.len());
    assert_eq!(result.new_blocks, replay.index.len());
    assert!(
        commits < replay.index.len(),
        "publication still follows each replay block"
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    for section in ["measure-pp-1", "measure-cp-1"] {
        assert_eq!(
            store.range_count(&key, section).unwrap(),
            Some(commits as u64)
        );
    }
    // A catalogue is its base plus a short chain of patches.
    assert!(
        store
            .range_count(&key, "measure-1-catalogue")
            .unwrap()
            .is_some_and(|count| (1..=4).contains(&count))
    );
    let saved = checkpoint(&store, &key, None).unwrap().unwrap().0;
    assert_eq!(saved.cursor.next_block, replay.index.len());
}

#[test]
fn accepted_frame_without_focus_observations_publishes_exact_table_refusal() {
    let mut missing = frame(1, 10, 2);
    *missing.tables[TableId::Focus.index()] = TableView::Unavailable {
        reason: "Synthetic focus supplying-read refusal".into(),
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![missing])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("1");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let refusal = crate::prepared_focus::query_unavailable(&store, &key, 1, 10, &flag)
        .unwrap()
        .unwrap();
    assert!(refusal.reasons.iter().any(|reason| reason.table == "Focus"
        && reason.reason.as_deref() == Some("Synthetic focus supplying-read refusal")));
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
        .append(&[continuation_records(&middle, 1, vec![frame(1, 15, 5000)])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("3");
    run(&appended, None, &flag, |_| {}).unwrap();
    compare(&appended, "1");
}

#[test]
fn prepared_groups_wait_when_other_prepared_sections_exist_without_group_state() {
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
    assert!(reply["data"].is_null());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_combat_queries_wait_for_coverage_and_resume_only_the_appended_tail() {
    use crate::products::{
        BranchParams, LoadContext, Product,
        division_log::DivisionLogManifest,
        land_battles::{LandBattlesManifest, LandBattlesManifestParams},
        naval_battles::{NavalBattlesManifest, NavalBattlesManifestParams},
    };
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("combat-prefix");
    let flag = AtomicBool::new(false);
    assert!(
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
        .is_err()
    );
    flag.store(false, Ordering::Release);
    let query = |source: &Source, branch: &str, product: &str| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params: serde_json::json!({"branchId": branch}),
        };
        serde_json::from_str::<serde_json::Value>(
            &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
        )
        .unwrap()["data"]
            .clone()
    };
    for product in [
        "land-battles-manifest",
        "naval-battles-manifest",
        "division-log-manifest",
    ] {
        assert_eq!(query(&source, "2", product), serde_json::Value::Null);
    }
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let context = LoadContext { content: None };
    let compare = |source: &Source, branch: &str| {
        assert_eq!(
            query(source, branch, "land-battles-manifest"),
            serde_json::to_value(
                LandBattlesManifest::load(
                    source,
                    &LandBattlesManifestParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap()
        );
        assert_eq!(
            query(source, branch, "naval-battles-manifest"),
            serde_json::to_value(
                NavalBattlesManifest::load(
                    source,
                    &NavalBattlesManifestParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap()
        );
        assert_eq!(
            query(source, branch, "division-log-manifest"),
            serde_json::to_value(
                DivisionLogManifest::load(
                    source,
                    &BranchParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap()
        );
    };
    compare(&source, "1");
    compare(&source, "2");
    let appended = replay
        .append(&[continuation_records(
            &replay,
            2,
            vec![frame(2, 22, 2), frame(2, 23, 2)],
        )])
        .unwrap();
    fixture.replace(&appended);
    let grown = fixture.source("combat-tail");
    assert_eq!(run(&grown, None, &flag, |_| {}).unwrap().new_blocks, 1);
    assert_eq!(run(&grown, None, &flag, |_| {}).unwrap().new_blocks, 0);
    compare(&grown, "1");
    compare(&grown, "2");
}

#[test]
fn prepared_ideas_replace_one_range_and_reload_it_after_append() {
    use crate::products::idea_history;
    let make = |hour| {
        let mut frame = frame(1, hour, 3);
        *frame.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(vec![replay::rows::ActiveIdeaRow {
                country: 100_000,
                ordinal: 0,
                token: b"mod_idea".to_vec(),
            }]));
        *frame.tables[TableId::HiredAdvisors.index()] =
            TableView::Complete(TableRows::HiredAdvisors(vec![]));
        frame
    };
    let old =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(10), make(11)])]).unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("idea-first");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let compare = |source: &Source| {
        let (store, key) = super::super::files(source, None).unwrap();
        assert_eq!(store.range_count(&key, "idea-history-1").unwrap(), Some(1));
        let loaded = load_branch(
            &store,
            &key,
            Some(&key),
            source.replay_ref(),
            1,
            10,
            &Refused::default(),
        )
        .unwrap();
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        for tag in [tag.as_str(), "missing"] {
            assert_eq!(
                serde_json::to_value(loaded.ideas.country(source.replay_ref(), 1, tag).unwrap())
                    .unwrap(),
                serde_json::to_value(
                    idea_history::scan(source.replay_ref(), 1, tag, &flag).unwrap()
                )
                .unwrap()
            );
        }
    };
    compare(&source);
    let grown = old
        .append(&[continuation_records(&old, 1, vec![make(12), make(14)])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("idea-append");
    assert_eq!(run(&appended, None, &flag, |_| {}).unwrap().new_blocks, 1);
    compare(&appended);
}

#[test]
fn prepared_idea_query_waits_for_its_state_even_with_complete_other_sections() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("ideas-pending");
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
        product: "idea-history".into(),
        params: serde_json::json!({"branchId":"1","tag":"opaque"}),
    };
    let reply: serde_json::Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    assert!(reply["data"].is_null());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_subjects_keep_changes_across_commits_cancel_resume_append_and_5000_countries() {
    use crate::products::{
        LoadContext, Product,
        subjects::{SubjectParams, Subjects},
    };
    fn subjects(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = frame(1, hour, count);
        frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            (1..count)
                .map(|index| replay::rows::SubjectRow {
                    overlord: 100_000,
                    subject: 100_000 + index,
                    start_hour: 9,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            (1..count)
                .map(|index| replay::rows::AutonomyRow {
                    subject: 100_000 + index,
                    state: if hour < 12 {
                        b"mod/low".to_vec()
                    } else {
                        b"mod/high".to_vec()
                    },
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame
    }
    let flag = AtomicBool::new(false);
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![subjects(10, 2), subjects(11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("subjects-1");
    run(&source, None, &flag, |_| {}).unwrap();
    let compare = |source: &Source| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "subjects".into(),
            params: serde_json::json!({"branchId":"1"}),
        };
        let actual: serde_json::Value = serde_json::from_str(
            &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let expected = Subjects::load(
            source,
            &SubjectParams {
                branch_id: "1".into(),
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        assert_eq!(actual["data"], serde_json::to_value(&expected).unwrap());
        let (store, key) = super::super::files(source, None).unwrap();
        assert_eq!(store.range_count(&key, "subjects-1").unwrap(), Some(1));
        expected
    };
    assert_eq!(compare(&source).runs.len(), 1);
    let middle = old
        .append(&[continuation_records(
            &old,
            1,
            vec![subjects(12, 2), subjects(13, 2)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("subjects-2");
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
    assert_eq!(compare(&source).runs[0].steps.len(), 2);
    let grown = middle
        .append(&[continuation_records(
            &middle,
            1,
            vec![subjects(15, 5000), subjects(16, 5000)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let source = fixture.source("subjects-3");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let reply = compare(&source);
    assert_eq!(reply.runs.len(), 5000);
    assert!(
        reply
            .unobserved
            .iter()
            .any(|span| span.first_hour == 14 && span.last_hour == 14)
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_autonomy_matches_oracle_after_commits_cancel_resume_append_and_5000_countries() {
    use crate::products::{
        LoadContext, Product,
        autonomy::{Autonomy, AutonomyParams},
    };
    fn autonomy(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = frame(1, hour, count);
        frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            (1..count)
                .map(|index| replay::rows::SubjectRow {
                    overlord: 100_000,
                    subject: 100_000 + index,
                    autonomy_ratio: i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            (1..count)
                .map(|index| replay::rows::AutonomyRow {
                    subject: 100_000 + index,
                    state: if hour < 12 {
                        b"mod/low".to_vec()
                    } else {
                        b"mod/high".to_vec()
                    },
                    score: i64::from(hour) * 100_000 + i64::from(index),
                    lower_score: 0,
                    upper_score: 10_000_000,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame
    }
    let flag = AtomicBool::new(false);
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![autonomy(10, 2), autonomy(11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("autonomy-1");
    run(&source, None, &flag, |_| {}).unwrap();
    let compare = |source: &Source| {
        let params = AutonomyParams {
            branch_id: "1".into(),
            subjects: vec![],
            overlords: vec![],
        };
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "series-state".into(),
            params: serde_json::json!({"branchId":"1","family":{"kind":"autonomy"},"subjects":[],"overlords":[],"reads":([10,11,12,13,14,15,16].map(|hour|serde_json::json!({"hour":hour})))}),
        };
        let actual: serde_json::Value = serde_json::from_str(
            &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let mut data = actual["data"].clone();
        for field in [
            "bands",
            "summaries",
            "exactHours",
            "navigation",
            "identities",
            "coverage",
        ] {
            data.as_object_mut().unwrap().remove(field);
        }
        let expected =
            Autonomy::load(source, &params, &LoadContext { content: None }, &flag).unwrap();
        assert_eq!(data, serde_json::to_value(&expected).unwrap());
        expected
    };
    assert_eq!(compare(&source).subjects.len(), 1);
    let middle = old
        .append(&[continuation_records(
            &old,
            1,
            vec![autonomy(12, 2), autonomy(13, 2)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("autonomy-2");
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
    assert_eq!(compare(&source).subjects[0].runs.len(), 4);
    let grown = middle
        .append(&[continuation_records(
            &middle,
            1,
            vec![autonomy(15, 5000), autonomy(16, 5000)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let source = fixture.source("autonomy-3");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let reply = compare(&source);
    assert_eq!(reply.subjects.len(), 4999);
    assert_eq!(
        reply
            .recorded
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(10, 13), (15, 16)]
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    // A catalogue is its base plus a short chain of patches.
    assert!(
        store
            .range_count(&key, "autonomy-1-catalogue")
            .unwrap()
            .is_some_and(|count| (1..=4).contains(&count))
    );
    assert_eq!(
        store.range_count(&key, "autonomy-series-1").unwrap(),
        Some(3)
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_tension_matches_oracle_after_commits_cancel_resume_append_and_5000_countries() {
    use crate::products::{BranchParams, LoadContext, Product, world_tension::WorldTension};
    fn tension(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = frame(1, hour, count);
        let amount = |index: u32| {
            if index.is_multiple_of(2) {
                -100_000 + i64::from(hour) * 1000
            } else {
                200_000 + i64::from(hour) * 1000
            }
        };
        let sum: i64 = (0..count).map(amount).sum();
        frame.tables[TableId::WorldTension.index()] =
            TableView::Complete(TableRows::WorldTension(vec![
                replay::rows::WorldTensionRow {
                    total_raw: sum.clamp(0, engine::cworld_threat::CLAMP_CEILING)
                        / engine::cworld_threat::SCALAR_DIVISOR,
                    source_count: count as i32,
                    country_aggregate_count: count as i32,
                },
            ]))
            .into();
        frame.tables[TableId::ThreatCountryAggregates.index()] =
            TableView::Complete(TableRows::ThreatCountryAggregates(
                (0..count)
                    .map(|index| replay::rows::ThreatCountryAggregateRow {
                        country_slot: index,
                        total_raw: amount(index),
                    })
                    .collect(),
            ))
            .into();
        frame.tables[TableId::ThreatSources.index()] =
            TableView::Complete(TableRows::ThreatSources(
                (0..count)
                    .map(|index| replay::rows::ThreatSourceRow {
                        ordinal: index,
                        country: 100_000 + index,
                        date: 9,
                        reason: format!("Synthetic threat {index}").into_bytes(),
                        current_raw: amount(index),
                        final_raw: amount(index) + 1000,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        frame.metrics.table_ns[TableId::WorldTension.index()] = 1;
        frame
    }
    let flag = AtomicBool::new(false);
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![tension(10, 2), tension(11, 2)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("tension-1");
    run(&source, None, &flag, |_| {}).unwrap();
    let compare = |source: &Source| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "series-state".into(),
            params: serde_json::json!({"branchId":"1","family":{"kind":"world-tension"},"reads":([10,11,12,13,14,15,16].map(|hour|serde_json::json!({"hour":hour})))}),
        };
        let actual: serde_json::Value = serde_json::from_str(
            &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let mut data = actual["data"].clone();
        for field in [
            "catalogue",
            "summaries",
            "exactHours",
            "navigation",
            "available",
            "coverage",
        ] {
            data.as_object_mut().unwrap().remove(field);
        }
        let expected = WorldTension::load(
            source,
            &BranchParams {
                branch_id: "1".into(),
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        assert_eq!(data, serde_json::to_value(&expected).unwrap());
        expected
    };
    assert_eq!(compare(&source).countries.len(), 2);
    let middle = old
        .append(&[continuation_records(
            &old,
            1,
            vec![tension(12, 2), tension(13, 2)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("tension-2");
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
    assert_eq!(compare(&source).reads.len(), 4);
    let grown = middle
        .append(&[continuation_records(
            &middle,
            1,
            vec![tension(15, 5000), tension(16, 5000)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let source = fixture.source("tension-3");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    let reply = compare(&source);
    assert_eq!(reply.countries.len(), 5000);
    assert_eq!(
        reply
            .reads
            .iter()
            .map(|read| (read.first_hour, read.starts_segment))
            .collect::<Vec<_>>(),
        [
            (10, true),
            (11, false),
            (12, false),
            (13, false),
            (15, true),
            (16, false)
        ]
    );
    let (store, key) = super::super::files(&source, None).unwrap();
    // A catalogue is its base plus a short chain of patches.
    assert!(
        store
            .range_count(&key, "world-tension-1-catalogue")
            .unwrap()
            .is_some_and(|count| (1..=4).contains(&count))
    );
    assert_eq!(
        store.range_count(&key, "tension-series-1").unwrap(),
        Some(3)
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_wars_match_oracle_after_commits_cancel_resume_append_and_5000_countries() {
    use crate::products::{
        LoadContext, Product,
        wars::{Wars, WarsParams, test_fixtures::scaled_recorded},
    };
    let flag = AtomicBool::new(false);
    let old = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![scaled_recorded(100, 3), scaled_recorded(101, 3)],
        )],
    )
    .unwrap();
    let mut fixture = Fixture::new(&old);
    let source = fixture.source("wars-1");
    run(&source, None, &flag, |_| {}).unwrap();
    let compare = |source: &Source| {
        for viewpoint_tag in [None, Some("mod-1"), Some("mod-2")] {
            let params = WarsParams {
                branch_id: "1".into(),
                viewpoint_tag: viewpoint_tag.map(str::to_owned),
            };
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "wars".into(),
                params: serde_json::json!({"branchId":"1","viewpointTag":viewpoint_tag,"reads":([100,101,102,103,104,105,106].map(|hour|serde_json::json!({"hour":hour})))}),
            };
            let actual: serde_json::Value = serde_json::from_str(
                &crate::prepared::query::query(source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            let expected =
                Wars::load(source, &params, &LoadContext { content: None }, &flag).unwrap();
            assert_eq!(
                actual["data"],
                serde_json::to_value(expected).unwrap(),
                "view {viewpoint_tag:?}"
            );
        }
    };
    compare(&source);
    let middle = old
        .append(&[continuation_records(
            &old,
            1,
            vec![scaled_recorded(102, 3), scaled_recorded(103, 3)],
        )])
        .unwrap();
    fixture.replace(&middle);
    let source = fixture.source("wars-2");
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
    compare(&source);
    let grown = middle
        .append(&[continuation_records(
            &middle,
            1,
            vec![scaled_recorded(105, 5000), scaled_recorded(106, 5000)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let source = fixture.source("wars-3");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    compare(&source);
    let (store, key) = super::super::files(&source, None).unwrap();
    // A catalogue is its base plus a short chain of patches.
    assert!(
        store
            .range_count(&key, "wars-1-catalogue")
            .unwrap()
            .is_some_and(|count| (1..=4).contains(&count))
    );
    assert_eq!(store.range_count(&key, "wars-series-1").unwrap(), Some(3));
    let mut reader =
        crate::prepared::series::Reader::new(&store, &key, "wars-series-1".into(), &flag).unwrap();
    assert_eq!(
        reader.event_count_through(0, i32::MAX).unwrap(),
        2,
        "counter-only appends do not publish another topology input"
    );
    let metadata = crate::products::wars::prepared::query(
        &source,
        &serde_json::json!({"branchId":"1","viewpointTag":"mod-1"}),
        &store,
        &key,
        &LoadContext { content: None },
        &flag,
    )
    .unwrap();
    assert!(
        metadata["wars"]
            .as_array()
            .unwrap()
            .iter()
            .all(|war| war["casualties"].as_array().unwrap().is_empty()),
        "structural requests never load hourly casualty history"
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_feed_matches_after_discarded_batch_cancel_resume_and_append() {
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn progressive_autonomy_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    use std::cell::Cell;
    fn sampled(hour: i32) -> DecodedFrame {
        let mut frame = crate::test_fixtures::world_frame(hour, hour >= 2);
        let mut subjects = if hour >= 2 {
            vec![9001, 31]
        } else {
            vec![9001]
        };
        subjects.sort_unstable();
        *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(
            subjects
                .iter()
                .map(|&subject| replay::rows::AutonomyRow {
                    subject,
                    state: if hour >= 7 {
                        b"recorded/mod-next".to_vec()
                    } else {
                        b"recorded/mod-level".to_vec()
                    },
                    score: i64::from(if hour == 7 {
                        6
                    } else if hour % 2 == 0 {
                        hour
                    } else {
                        99
                    }) * 100_000,
                    lower_score: 0,
                    upper_score: 10_000_000,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            subjects
                .iter()
                .map(|&subject| replay::rows::SubjectRow {
                    subject,
                    overlord: 21,
                    autonomy_ratio: 23_456,
                    ..Default::default()
                })
                .collect(),
        ));
        frame
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sampled(0), sampled(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![sampled(block * 2), sampled(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive-autonomy");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let requests = Cell::new(0);
    let query = |product: &str, params: serde_json::Value| {
        requests.set(requests.get() + 1);
        assert!(requests.get() <= 6, "progressive views never request-loop");
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let value: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        value["data"].clone()
    };
    let summary = |from| {
        serde_json::json!({"branchId":"1","family":{"kind":"autonomy"},"subjects":[],"overlords":[],"from":from,"to":29,"width":30,
        "within":{"holder":[["GER",0,29,true]],"tail-only":[["D01 mod",2,29,true]]}})
    };
    let bootstrap = query(
        "series-summary",
        serde_json::json!({"branchId":"1", "family":{"kind":"autonomy"}, "from":0,"to":29,"width":1,"within":{}}),
    );
    assert!(
        bootstrap["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|subject| subject["runs"].as_array().unwrap().is_empty()),
        "bootstrap catalogue retains no numeric history"
    );
    assert_eq!(
        bootstrap["bands"][0]["runs"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["lastHour"],
        29
    );
    let cold = query("series-summary", summary(0));
    assert!(cold["coverage"].is_null());
    assert!(
        !cold["summaries"]["holder"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        cold["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|subject| subject["tag"] == "D01 mod")
    );
    assert!(
        cold["identities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|country| country["tag"] == "ITA"),
        "overlord identity is available without groups"
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "sampling never creates prepared files"
    );
    let state = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"autonomy"},"reads":[{"hour":29}]}),
    );
    assert_eq!(state["exactHours"], serde_json::json!([29]));
    assert_eq!(state["subjects"][0]["runs"][0]["scoreRaw"], "9900000");
    let mut observer = crate::products::autonomy::ScannedAutonomy::default();
    let mut history = crate::products::country_history::HistoryScan::new(false, &[]);
    for frame in [sampled(0), sampled(1)] {
        observer.prepare(&frame).unwrap();
        history.observe(&frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    observer.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &history).unwrap();
    publish_coverage(&mut writer, 1, 0, 1, false).unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "autonomy-series-1").unwrap();
    let rough = query("series-summary", summary(0));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    assert_eq!(
        rough["summaries"]["holder"]["approximate"],
        serde_json::json!([{"firstHour":2,"lastHour":29}])
    );
    let exact = query("series-summary", summary(4));
    assert!(
        exact["summaries"]["holder"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let runs = exact["summaries"]["holder"]["runs"].as_array().unwrap();
    for hour in 4..30 {
        let reached = runs
            .iter()
            .find(|run| {
                run["firstHour"].as_i64().unwrap() <= hour
                    && hour <= run["lastHour"].as_i64().unwrap()
            })
            .unwrap();
        let expected = if hour == 7 {
            60_000_000
        } else if hour % 2 == 0 {
            hour * 10_000_000
        } else {
            990_000_000
        };
        assert_eq!(
            reached["value"].as_f64().unwrap(),
            expected as f64,
            "hour {hour}"
        );
    }
    let change = query(
        "series-next-change",
        serde_json::json!({"branchId":"1", "family":{"kind":"autonomy"}, "firstHour":4, "lastHour":29, "from":6, "key":"ArrowRight", "tag":"holder", "within":{"holder":[["GER",0,29,true]]}}),
    );
    assert_eq!(
        change["hour"], 7.0,
        "categorical changes remain exact stops when numeric operands are equal"
    );
    assert_eq!(
        store.range_count(&key, "autonomy-series-1").unwrap(),
        ranges
    );
    assert_eq!(requests.get(), 6);
}

#[test]
fn air_superiority_unprepared_regional_spans_remain_pending_without_a_frame_sampler() {
    let scored = |hour| {
        let mut frame = frame(1, hour, 2);
        *frame.tables[TableId::RaidAirScores.index()] =
            TableView::Complete(TableRows::RaidAirScores(vec![
                replay::rows::RaidAirScoreRow {
                    country: 100_000,
                    region: 7000,
                    friendly: i64::from(hour),
                    enemy: 1,
                    impact: Default::default(),
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
    // A pending comparison must not try to decode the unprepared tail.
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
            // Even a prepared prefix needs the final historical side anchors.
            ("timeline", 10, 11),
        ] {
            assert!(
                query(serde_json::json!({"branchId":"1", "view":view,
                    "firstHour":first, "lastHour":last, "regions":[7000]}))
                .is_null(),
                "{view} {first}..{last} must remain pending"
            );
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
fn naval_losses_unprepared_spans_are_pending_not_zero() {
    fn sinking_frame(hour: i32) -> DecodedFrame {
        let mut frame = crate::test_fixtures::source(1, hour, Some(0));
        let entries = if hour >= 2 {
            vec![replay::rows::SunkShipEntry {
                ship_name: b"synthetic retained sinking".to_vec(),
                victim_country: 9001,
                attacker_country: 9001,
                sink_hour: 2,
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![replay::rows::SunkShipsRow {
                entries,
            }]))
            .into();
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        frame
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![sinking_frame(0), sinking_frame(1)])],
    )
    .unwrap();
    let replay = replay
        .append(&[continuation_records(
            &replay,
            1,
            vec![sinking_frame(2), sinking_frame(3)],
        )])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("naval-progressive-pending");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let selection = serde_json::json!({"branchId":"1","kind":"ships","attribution":"lost",
        "period":"running","tags":["GER"],"top":null,"hour":3});
    let family = serde_json::json!({"kind":"naval-losses","navalKind":"ships","attribution":"lost",
        "period":"running","top":null});
    let queries = [
        ("naval-losses", selection),
        (
            "series-summary",
            serde_json::json!({"branchId":"1","family":family.clone(),"tags":["GER"],
            "from":2,"to":3,"width":1}),
        ),
        (
            "series-next-change",
            serde_json::json!({"branchId":"1","family":family,"tags":["GER"],
            "firstHour":2,"lastHour":3,"key":"Home","from":null}),
        ),
    ];
    for accepted_prefix in [false, true] {
        if accepted_prefix {
            let mut writer = store.replace(key.clone()).unwrap();
            json(
                &mut writer,
                &coverage_section(1),
                &Coverage {
                    schema: SCHEMA,
                    first_hour: 0,
                    last_hour: 1,
                    complete: false,
                },
            )
            .unwrap();
            writer.commit().unwrap();
        }
        for (product, params) in &queries {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: (*product).into(),
                params: params.clone(),
            };
            let reply: serde_json::Value = serde_json::from_str(
                &super::super::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap();
            assert!(
                reply["data"].is_null(),
                "{product}, prefix={accepted_prefix}: an unprepared sinking tally must be pending, never a zero or retained-vector count"
            );
        }
        if !accepted_prefix {
            assert!(
                store.saved_key().unwrap().is_none(),
                "pending reads never prepare data"
            );
        } else {
            assert_eq!(
                store.range_count(&key, &coverage_section(1)).unwrap(),
                Some(1)
            );
            assert!(
                store
                    .range_count(&key, "naval-loss-values-1-0")
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    fn sampled(hour: i32) -> DecodedFrame {
        let mut frame = crate::test_fixtures::source(
            1,
            hour,
            Some(if hour % 2 == 0 { i64::from(hour) } else { 99 }),
        );
        if hour >= 2 {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows.push(replay::rows::CountryRow {
                    id: 333,
                    canonical_slot: 8,
                    tag: b"tail only".to_vec(),
                    political_power_raw: Some(i64::from(hour)),
                    ..Default::default()
                });
                rows.sort_by_key(|row| row.id);
                rows.dedup_by_key(|row| row.id);
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(replay::rows::CountryLinkRow {
                    id: 333,
                    canonical_slot: 8,
                });
                rows.sort_by_key(|row| row.id);
                rows.dedup_by_key(|row| row.id);
            }
        }
        frame
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sampled(0), sampled(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![sampled(block * 2), sampled(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let query = |product: &str, params: serde_json::Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let value: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        value["data"].clone()
    };
    let summary = |from: i32, tags: &[&str]| serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"tags":tags,"from":from,"to":29,"width":30});
    // No cache still gives a rough line and tail-only native states.
    let cold = query("series-summary", summary(0, &["GER", "tail only"]));
    assert!(cold["coverage"].is_null());
    assert_eq!(
        cold["series"][0]["summary"]["approximate"][0]["firstHour"],
        0
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "direct reads never create prepared files"
    );
    let cold_state = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"reads":[{"tag":"tail only","hour":29}]}),
    );
    assert_eq!(cold_state["states"][0]["run"]["valueRaw"], "29");
    assert_eq!(cold_state["states"][0]["run"]["countryId"], 333);
    assert_eq!(cold_state["states"][0]["approximate"], false);
    // Publish only the first block, using the production observer and existing coverage page.
    let mut accumulator = MeasureAccumulator::new(0);
    accumulator.observe(&sampled(0)).unwrap();
    accumulator.observe(&sampled(1)).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    accumulator.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "measure-pp-1").unwrap();
    let rough = query("series-summary", summary(0, &["GER", "tail only"]));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    assert_eq!(
        rough["series"][0]["summary"]["approximate"],
        serde_json::json!([{"firstHour":2,"lastHour":29}])
    );
    let runs = rough["series"][0]["summary"]["runs"].as_array().unwrap();
    assert_eq!(runs.iter().filter(|run| run["firstHour"] == 0).count(), 1);
    assert!(
        rough["series"][1]["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 2)
    );
    let mut member_params = summary(0, &["tail only"]);
    member_params["within"] = serde_json::json!({"tail only":[{"firstHour":6,"lastHour":9}]});
    let member = query("series-summary", member_params);
    assert_eq!(
        member["series"][0]["summary"]["approximate"],
        serde_json::json!([{"firstHour":6,"lastHour":9}])
    );
    assert!(
        member["series"][0]["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["firstHour"].as_i64().unwrap() >= 6
                && run["lastHour"].as_i64().unwrap() <= 9)
    );
    let exact = query("series-summary", summary(4, &["GER", "tail only"]));
    assert_eq!(
        exact["series"][0]["summary"]["approximate"],
        serde_json::json!([])
    );
    let exact_runs = exact["series"][0]["summary"]["runs"].as_array().unwrap();
    assert!(
        exact_runs
            .iter()
            .any(|run| run["firstHour"] == 5 && run["value"] == 99.0 / 100_000.0)
    );
    let event_page = query(
        "events-page",
        serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"tags":["GER"],"to":29,"pageSize":20}),
    );
    assert_eq!(event_page["coverage"]["lastHour"], 1);
    assert_eq!(event_page["total"], "0");
    let navigation = query(
        "series-next-change",
        serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"tags":["GER"],"from":4,"firstHour":4,"lastHour":29,"key":"ArrowRight"}),
    );
    assert_eq!(navigation["hour"].as_f64(), Some(5.0));
    assert_eq!(navigation["approximate"], false);
    let rough_navigation = query(
        "series-next-change",
        serde_json::json!({"branchId":"1","family":{"kind":"country-measure","measure":"politicalPower"},"tags":["GER"],"from":2,"firstHour":0,"lastHour":29,"key":"ArrowRight"}),
    );
    assert_eq!(rough_navigation["hour"].as_f64(), Some(4.0));
    assert_eq!(rough_navigation["approximate"], true);
    assert_eq!(store.range_count(&key, "measure-pp-1").unwrap(), ranges);
    // An invalidated schema ignores stale prepared pages and uses the same direct path.
    let mut writer = store.extend(&key).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA + 1,
            first_hour: 0,
            last_hour: 29,
            complete: true,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let invalidated = query("series-summary", summary(0, &["GER"]));
    assert!(invalidated["coverage"].is_null());
    assert_eq!(
        invalidated["series"][0]["summary"]["approximate"][0]["firstHour"],
        0
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn progressive_stability_parts_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    use crate::test_fixtures::{stability_frame, stability_inputs};
    fn sampled(hour: i32) -> DecodedFrame {
        let mut inputs = stability_inputs();
        inputs.stability_raw = if hour % 2 == 0 {
            50_000 + i64::from(hour)
        } else {
            60_000
        };
        let mut frame = stability_frame(
            hour,
            inputs.clone(),
            hour,
            if hour >= 2 {
                vec![replay::rows::CountryStabilityLineRow {
                    country: 9001,
                    key: 0x8d,
                    kind: replay::rows::CountryStabilityLineRow::IDEA,
                    identity: b"mod/tail".to_vec(),
                    value: 10_000,
                    ..Default::default()
                }]
            } else {
                vec![]
            },
        );
        if hour >= 2 {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows.push(replay::rows::CountryRow {
                    id: 333,
                    canonical_slot: 8,
                    tag: b"tail only".to_vec(),
                    ..Default::default()
                });
                rows.sort_by_key(|row| row.id);
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(replay::rows::CountryLinkRow {
                    id: 333,
                    canonical_slot: 8,
                });
                rows.sort_by_key(|row| row.id);
            }
            inputs.country = 333;
            if let TableView::Complete(TableRows::CountryStabilityInputs(rows)) =
                &mut *frame.tables[TableId::CountryStabilityInputs.index()]
            {
                rows.push(inputs);
                rows.sort_by_key(|row| row.country);
            }
            if let TableView::Complete(TableRows::SliceReads(rows)) =
                &mut *frame.tables[TableId::SliceReads.index()]
            {
                rows.push(replay::rows::SliceReadRow {
                    owner: 333,
                    table: TableId::CountryStabilityInputs as u32,
                    sample_hour: hour,
                });
                rows.sort_by_key(|row| (row.table, row.owner));
            }
        }
        frame
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sampled(0), sampled(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![sampled(block * 2), sampled(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive parts");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let requests = std::cell::Cell::new(0);
    let query = |product: &str, params: serde_json::Value| {
        requests.set(requests.get() + 1);
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let summary = |from: i32, width: u32| serde_json::json!({"branchId":"1","family":{"kind":"stability-parts","measure":"stability"},"tags":["GER","tail only"],"from":from,"to":29,"width":width});
    let child = |reply: &serde_json::Value, tag: &str, id: &str| {
        reply["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["tag"] == serde_json::to_string(&(tag, id)).unwrap())
            .unwrap()["summary"]
            .clone()
    };
    let cold = query("series-summary", summary(0, 30));
    assert!(cold["coverage"].is_null());
    assert_eq!(
        child(&cold, "GER", "term:base")["approximate"],
        serde_json::json!([{"firstHour":0,"lastHour":29}])
    );
    assert!(
        cold["parts"]["tail only"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["id"] == "term:base")
    );
    assert!(
        cold["parts"]["GER"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["id"] == "line:idea:mod/tail:0:0")
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "tail reads must not publish prepared files"
    );
    let mut prepared = crate::products::stability_parts::Prepared::default();
    prepared.observe(&sampled(0)).unwrap();
    prepared.observe(&sampled(1)).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let before = store.range_count(&key, "parts-1").unwrap();
    let rough = query("series-summary", summary(0, 30));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    assert_eq!(
        child(&rough, "GER", "term:base")["approximate"],
        serde_json::json!([{"firstHour":2,"lastHour":29}])
    );
    let mut member = summary(0, 30);
    member["within"] = serde_json::json!({"tail only":[{"firstHour":6,"lastHour":9}]});
    let member = query("series-summary", member);
    assert_eq!(
        child(&member, "tail only", "term:base")["approximate"],
        serde_json::json!([{"firstHour":6,"lastHour":9}])
    );
    let dense = query("series-summary", summary(0, 1));
    assert!(child(&dense, "GER", "term:base")["runs"].is_null());
    assert_eq!(
        child(&dense, "GER", "term:base")["buckets"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let exact = query("series-summary", summary(4, 30));
    let base = child(&exact, "GER", "term:base");
    assert_eq!(base["approximate"], serde_json::json!([]));
    assert!(
        base["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 5 && run["value"] == 0.6)
    );
    let state = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"stability-parts","measure":"stability"},"reads":[{"tag":"tail only","hour":29}]}),
    );
    let base = state["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["tag"] == serde_json::to_string(&("tail only", "term:base")).unwrap())
        .unwrap();
    assert_eq!(base["run"]["valueRaw"], "60000");
    assert_eq!(base["run"]["countryId"], 333);
    assert_eq!(base["approximate"], false);
    let navigation = query(
        "series-next-change",
        serde_json::json!({"branchId":"1","family":{"kind":"stability-parts","measure":"stability"},"tags":["GER"],"from":4,"firstHour":4,"lastHour":29,"key":"ArrowRight"}),
    );
    assert_eq!(navigation["hour"].as_f64(), Some(5.0));
    assert_eq!(navigation["approximate"], false);
    assert_eq!(
        requests.get(),
        7,
        "one bounded request per reached view, date or navigation"
    );
    assert_eq!(store.range_count(&key, "parts-1").unwrap(), before);
    assert_eq!(store.range_count(&key, "parts-1-names").unwrap(), Some(1));
}

#[test]
fn unprepared_research_span_returns_pending_through_the_real_query_path() {
    use crate::products::research::{prepared, test_fixtures::sliced_research_frame};

    let frames = vec![
        sliced_research_frame(100, 100, &[Some(1)]),
        sliced_research_frame(101, 101, &[Some(2)]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("research-pending");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut observer = prepared::Prepared::default();
    observer.observe(&frames[0]).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    observer.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: 100,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let prefix = prepared::history(&store, &key, 1, "GER", &flag)
        .unwrap()
        .unwrap();
    assert_eq!(prefix.0.slots[0].entries[0].token, "tech_1");

    for (product, selection) in [
        (
            "series-state",
            serde_json::json!({"reads":[{"tag":"GER","hour":101}]}),
        ),
        (
            "series-summary",
            serde_json::json!({"tags":["GER"],"from":101,"to":101,"width":10}),
        ),
        ("events-page", serde_json::json!({"tag":"GER","to":101})),
        ("events-page", serde_json::json!({"tag":"GER"})),
    ] {
        let mut params = selection;
        params["branchId"] = serde_json::json!("1");
        params["family"] = serde_json::json!({"kind":"research"});
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let response: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        assert_eq!(response["product"], product);
        assert!(
            response["data"].is_null(),
            "{product} must stay pending beyond research coverage: {response}"
        );
    }
}

#[test]
fn unprepared_peace_deals_span_returns_pending_through_the_real_query_path() {
    use crate::products::peace_deals::{prepared, test_fixtures as tests};

    let frames = vec![
        tests::frame(100, 3, false, &[9]),
        tests::frame(101, 3, true, &[]),
    ];
    let replay = tests::replay(frames.clone());
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("peace-pending");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut observer = prepared::Prepared::new(&replay, 1);
    observer.observe(&frames[0]).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    observer.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: 100,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let (keys, ()) = crate::prepared::series::read_catalogue::<()>(&store, &key, "peace-deals", 1)
        .unwrap()
        .unwrap();
    assert!(keys.contains_key("1"));

    for end in [Some(101), None] {
        let mut params = serde_json::json!({"branchId":"1","family":{"kind":"peace-deals"}});
        if let Some(hour) = end {
            params["to"] = serde_json::json!(hour);
        }
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "events-page".into(),
            params,
        };
        let response: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        assert_eq!(response["product"], "events-page");
        assert!(
            response["data"].is_null(),
            "unprepared peace outcomes must stay pending, not look like an empty or complete reply: {response}"
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn progressive_fuel_factory_and_nukes_queries_join_prefix_and_tail_then_zoom_exactly_without_writing()
 {
    fn sampled(hour: i32) -> DecodedFrame {
        let count = if hour < 2 { 1 } else { 2 };
        let mut frame = crate::test_fixtures::scale_frame(1, hour, count);
        let amount = if hour % 2 == 0 { i64::from(hour) } else { 99 };
        frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
        *frame.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
            (0..count)
                .map(|index| replay::rows::CountryFuelRow {
                    country: 100_000 + index,
                    current_raw: amount + i64::from(index),
                    maximum_raw: 32768,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryFactories.index()] =
            TableView::Complete(TableRows::CountryFactories(
                (0..count)
                    .map(|index| replay::rows::CountryFactoriesRow {
                        country: 100_000 + index,
                        military_total_raw: (amount + i64::from(index)) * 100_000,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
            (0..count)
                .map(|index| replay::rows::NukeRow {
                    country: 100_000 + index,
                    nuke_type: 0,
                    amount_raw: (amount + i64::from(index)) * 100_000_000,
                    ..Default::default()
                })
                .collect(),
        ));
        frame
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sampled(0), sampled(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![sampled(block * 2), sampled(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive-fuel-forces");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let query = |product: &str, params: serde_json::Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let tags: Vec<_> = (0..2)
        .map(|index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap())
        .collect();
    let summary = |kind: &str, from: i32| serde_json::json!({"branchId":"1","family":{"kind":kind},"tags":tags,"from":from,"to":29,"width":30});
    let lines = |data: &serde_json::Value, kind: &str| {
        data[if kind == "country-fuel" {
            "series"
        } else {
            "summaries"
        }]
        .as_array()
        .unwrap()
        .clone()
    };
    for kind in ["country-fuel", "country-factories", "country-nukes"] {
        let cold = query("series-summary", summary(kind, 0));
        assert!(cold["coverage"].is_null());
        assert_eq!(
            lines(&cold, kind)[0]["summary"]["approximate"],
            serde_json::json!([{"firstHour":0,"lastHour":29}])
        );
        assert!(
            lines(&cold, kind).iter().any(|line| line["tag"] == tags[1]),
            "tail-only countries must not require catalogue keys"
        );
        assert!(
            store.saved_key().unwrap().is_none(),
            "progressive reads cannot publish prepared data"
        );
        let absent = query(
            "series-state",
            serde_json::json!({"branchId":"1","family":{"kind":kind},"reads":[{"tag":"absent country","hour":29}]}),
        );
        if kind == "country-fuel" {
            assert!(absent["states"][0]["fuelRaw"].is_null());
        } else {
            assert!(
                absent["series"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|series| series["runs"].as_array().unwrap().is_empty())
            );
        }
    }
    let mut fuel = crate::products::country_fuel::prepared::Prepared::default();
    let mut factories = crate::products::country_factories::prepared::Prepared::default();
    let mut nukes = crate::products::country_nukes::prepared::Prepared::default();
    let mut identities = crate::products::country_history::HistoryScan::new(false, &[]);
    for hour in 0..=1 {
        let frame = sampled(hour);
        fuel.observe(&frame, replay.format, &flag).unwrap();
        factories
            .observe(
                &frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(&frame, replay.format),
            )
            .unwrap();
        nukes
            .observe(
                &frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(&frame, replay.format),
            )
            .unwrap();
        identities.observe(&frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    fuel.flush(&mut writer, 1, replay.format, &flag).unwrap();
    factories.flush(&mut writer, 1, &flag).unwrap();
    nukes.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let range_counts: Vec<_> = ["fuel-1", "factories-values-1-6", "nukes-values-1"]
        .map(|section| store.range_count(&key, section).unwrap())
        .into();
    for kind in ["country-fuel", "country-factories", "country-nukes"] {
        let rough = query("series-summary", summary(kind, 0));
        assert_eq!(rough["coverage"]["lastHour"], 1);
        assert_eq!(
            lines(&rough, kind)[0]["summary"]["approximate"],
            serde_json::json!([{"firstHour":2,"lastHour":29}])
        );
        let mut member = summary(kind, 0);
        member["within"] = serde_json::json!({(tags[1].clone()):[{"firstHour":6,"lastHour":9}]});
        let clipped = query("series-summary", member);
        let tail_only = lines(&clipped, kind)
            .into_iter()
            .find(|line| line["tag"] == tags[1])
            .unwrap();
        assert_eq!(
            tail_only["summary"]["approximate"],
            serde_json::json!([{"firstHour":6,"lastHour":9}])
        );
        let exact = query("series-summary", summary(kind, 4));
        let exact_lines = lines(&exact, kind);
        assert_eq!(
            exact_lines[0]["summary"]["approximate"],
            serde_json::json!([])
        );
        assert!(
            exact_lines[0]["summary"]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"].as_f64() == Some(5.0)
                    && run["value"].as_f64() == Some(99.0)),
            "{kind}: thirteen-block zoom must retain the unsampled hourly change"
        );
        if kind != "country-fuel" {
            let mut top = summary(kind, 0);
            top["tags"] = serde_json::json!([]);
            top["family"]["top"] = if kind == "country-factories" {
                serde_json::json!({"count":1,"mode":"built","groups":["military"]})
            } else {
                serde_json::json!(1)
            };
            let ranked = query("series-summary", top);
            assert!(
                ranked["series"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|series| series["tag"] == tags[1]),
                "transient ranks use the same sampled numeric value"
            );
        }
    }
    let state = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"country-fuel"},"reads":[{"tag":tags[1],"hour":29}]}),
    );
    assert_eq!(state["states"][0]["fuelRaw"], "100");
    assert_eq!(state["states"][0]["approximate"], false);
    assert!(state["states"][0]["fuel"].is_null());
    assert!(state["states"][0]["consumption"].is_null());
    assert!(query("series-summary",serde_json::json!({"branchId":"1","family":{"kind":"country-fuel"},"tags":tags,"measures":["stripGain"],"from":0,"to":29,"width":30})).is_null(), "cross-frame accounting must remain pending");
    for (index, section) in ["fuel-1", "factories-values-1-6", "nukes-values-1"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            store.range_count(&key, section).unwrap(),
            range_counts[index]
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn progressive_nukes_bind_an_unprepared_origin_outside_the_view_and_keep_unchanged_read_dates() {
    let original = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    let make = |hour: i32| {
        let mut frame = scale_frame(1, hour, 2);
        if hour >= 4
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"later alias".to_vec();
            rows[1].tag = original.as_bytes().to_vec();
        }
        *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
            (0..2)
                .map(|index| replay::rows::NukeRow {
                    country: 100_000 + index,
                    nuke_type: 0,
                    amount_raw: if hour == 29 {
                        0
                    } else {
                        (7 + i64::from(index)) * 100_000_000
                    },
                    ..Default::default()
                })
                .collect(),
        ));
        frame
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(0), make(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![make(block * 2), make(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("progressive-nukes-origin");
    let flag = AtomicBool::new(false);
    let context = crate::products::LoadContext { content: None };
    let check = || {
        let data = crate::products::country_nukes::prepared::query(
            &source,
            "series-summary",
            &serde_json::json!({"branchId":"1","family":{"kind":"country-nukes","originHour":3},
            "tags":[original],"from":4,"to":29,"width":30}),
            &context,
            &flag,
        )
        .unwrap();
        let alias = data["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|series| series["tag"] == "later alias")
            .expect("exact origin selects its later alias");
        assert_eq!(alias["runs"][0]["total"], 7);
        assert!(
            data["series"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|series| series["tag"] == original)
                .all(|series| series["runs"].as_array().unwrap().is_empty()),
            "a reused tag must not select its replacement holder"
        );
        assert_eq!(
            data["readHours"],
            serde_json::json!((4..=29).collect::<Vec<_>>()),
            "equal values retain every fresh supplying date"
        );
        assert_eq!(
            data["emptyReadHours"],
            serde_json::json!([29]),
            "the complete zero census retains its empty-read fact"
        );
    };
    check();
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut nukes = crate::products::country_nukes::prepared::Prepared::default();
    let mut identities = crate::products::country_history::HistoryScan::new(false, &[]);
    for hour in 0..=1 {
        let frame = make(hour);
        nukes
            .observe(
                &frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(&frame, replay.format),
            )
            .unwrap();
        identities.observe(&frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    nukes.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    publish_coverage(&mut writer, 1, 0, 1, false).unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "nukes-values-1").unwrap();
    check();
    assert_eq!(store.range_count(&key, "nukes-values-1").unwrap(), ranges);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn territory_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    use crate::products::{LoadContext, country_history::HistoryScan, territory};
    use replay::rows::{CountryLinkRow, CountryRow, ProvinceRow};
    fn sampled(hour: i32) -> DecodedFrame {
        let mut frame = crate::test_fixtures::ownership_frame(1, hour, 44);
        if hour >= 2 {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                for row in rows.iter_mut() {
                    if row.id == 55 {
                        row.tag = if hour >= 20 {
                            b"next alias".to_vec()
                        } else {
                            b"country alias".to_vec()
                        };
                    } else if row.id == 66 && hour >= 20 {
                        row.tag = b"country alias".to_vec();
                    } else if row.id == 44 && hour == 29 {
                        row.id = 444;
                    }
                }
                rows.push(CountryRow {
                    id: 333,
                    canonical_slot: 12,
                    tag: b"tail only".to_vec(),
                    ..Default::default()
                });
                rows.sort_by_key(|row| row.id);
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                if hour == 29 {
                    rows.iter_mut().find(|row| row.id == 44).unwrap().id = 444;
                }
                rows.push(CountryLinkRow {
                    id: 333,
                    canonical_slot: 12,
                });
                rows.sort_by_key(|row| row.id);
            }
        }
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    owner: if hour == 29 { 444 } else { 44 },
                    controller: if hour == 29 { 444 } else { 44 },
                    victory_points: if hour % 2 == 0 { hour } else { 99 },
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    owner: if hour >= 2 { 333 } else { 0 },
                    controller: if hour >= 2 { 333 } else { 0 },
                    victory_points: hour,
                    ..Default::default()
                },
            ]));
        for table in [TableId::States, TableId::Provinces] {
            frame.metrics.table_ns[table.index()] = 1;
        }
        if hour == 29
            && let TableView::Complete(TableRows::States(rows)) =
                &mut *frame.tables[TableId::States.index()]
        {
            rows[0].owner = 444;
        }
        frame
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sampled(0), sampled(1)])]).unwrap();
    for block in 1..15 {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![sampled(block * 2), sampled(block * 2 + 1)],
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("territory-progressive");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let query = |product: &str, params: serde_json::Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let summary = |from: i32| serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"victoryPoints","rule":"controlled"},"tags":["FRA","tail only"],"from":from,"to":29,"width":30});
    let cold = query("series-summary", summary(0));
    assert!(cold["coverage"].is_null());
    assert_eq!(
        cold["series"][0]["summary"]["approximate"],
        serde_json::json!([{"firstHour":0,"lastHour":29}])
    );
    assert!(
        cold["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["tag"] == "tail only")
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "cold progressive queries never publish a cache"
    );
    let head = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"victoryPoints","rule":"owned"},"tags":["tail only"],"reads":[{"hour":29}]}),
    );
    let tail = head["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["tag"] == "tail only")
        .unwrap();
    assert_eq!(tail["runs"][0]["values"][6], 29);
    let top = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"victoryPoints","rule":"controlled","top":1},"tags":[],"reads":[{"hour":29}]}),
    );
    assert_eq!(top["series"].as_array().unwrap().len(), 1);
    assert_eq!(top["series"][0]["tag"], "FRA");
    let mut observer = territory::prepared::Prepared::default();
    let mut identities = HistoryScan::new(false, &[]);
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    observer
        .configure(&source, &LoadContext { content: None }, &flag)
        .unwrap();
    for frame in [sampled(0), sampled(1)] {
        let changes = tracker.observe(&frame, source.replay_ref().format);
        observer
            .observe_owned_states(&frame, &mut identities, Some(&changes))
            .unwrap();
        observer
            .observe(&frame, source.replay_ref().format, &flag, &changes)
            .unwrap();
    }
    let mut writer = store.replace(key.clone()).unwrap();
    observer.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "territory-values-1-2").unwrap();
    let rough = query("series-summary", summary(0));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    let fra = rough["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["tag"] == "FRA")
        .unwrap();
    assert_eq!(
        fra["summary"]["approximate"],
        serde_json::json!([{"firstHour":2,"lastHour":29}])
    );
    let runs = fra["summary"]["runs"].as_array().unwrap();
    assert_eq!(runs.iter().filter(|run| run["firstHour"] == 0).count(), 1);
    assert_eq!(fra["endRuns"][0]["values"][2], 99);
    assert!(
        fra["identities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 29 && run["countryId"] == 444)
    );
    assert!(
        !fra["identities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["lastHour"] == 29 && run["countryId"] == 44)
    );
    let mut dense_group = summary(0);
    dense_group["width"] = serde_json::json!(3);
    dense_group["family"]["aggregate"] = serde_json::json!(true);
    let dense_group = query("series-summary", dense_group);
    assert!(dense_group["series"][0]["summary"]["runs"].is_null());
    // Dense group geometry is the shared summary grid (power-of-two in fine views).
    assert_eq!(
        dense_group["series"][0]["summary"]["buckets"]
            .as_array()
            .unwrap()
            .len(),
        crate::prepared::series::summary_bins((0, 29), (0.0, 29.0, 3))
            .unwrap()
            .1
            .len()
    );
    assert_eq!(
        dense_group["sum"][0]["value"], 128.0,
        "dense group labels sum exact reached member values"
    );
    let unknown = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"states","rule":"controlled","aggregate":true},"tags":["never recorded"],"reads":[{"hour":29}]}),
    );
    assert_eq!(
        unknown["sum"],
        serde_json::json!([]),
        "an unproved empty selection remains unknown"
    );
    let aliases = query(
        "series-summary",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"states","rule":"controlled"},"tags":["CTL"],"from":0,"to":29,"width":30}),
    );
    let alias = aliases["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["tag"] == "country alias")
        .unwrap();
    assert_eq!(
        alias["summary"]["approximate"],
        serde_json::json!([{"firstHour":2,"lastHour":19}])
    );
    assert!(
        alias["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["lastHour"].as_i64().unwrap() <= 19)
    );
    assert_eq!(alias["endRuns"][0]["values"][0], 1);
    assert_eq!(alias["endRuns"][0]["lastHour"], 19);
    assert!(
        alias["identities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["countryId"] == 55)
    );
    let reached = query(
        "series-state",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"states","rule":"controlled"},"tags":["CTL"],"reads":[{"hour":29}]}),
    );
    assert!(
        reached["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["tag"] == "next alias" && line["runs"][0]["values"][0] == 1)
    );
    assert!(
        !reached["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["tag"] == "country alias"
                && !line["runs"].as_array().unwrap().is_empty())
    );
    let exact = query("series-summary", summary(4));
    let fra = exact["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["tag"] == "FRA")
        .unwrap();
    assert_eq!(fra["summary"]["approximate"], serde_json::json!([]));
    assert!(
        fra["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 5 && run["value"] == 99.0)
    );
    let navigation = query(
        "series-next-change",
        serde_json::json!({"branchId":"1","family":{"kind":"territory","measure":"victoryPoints","rule":"controlled"},"tags":["FRA"],"from":4,"firstHour":4,"lastHour":29,"key":"ArrowRight"}),
    );
    assert_eq!(navigation["hour"].as_f64(), Some(5.0));
    assert_eq!(navigation["approximate"], false);
    let mut group = summary(4);
    group["family"]["aggregate"] = serde_json::json!(true);
    let group = query("series-summary", group);
    assert_eq!(group["series"][0]["tag"], "group");
    assert_eq!(group["sum"][0]["value"], 128.0);
    assert_eq!(
        store.range_count(&key, "territory-values-1-2").unwrap(),
        ranges
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
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
fn game_presence_discards_block_and_library_resume_reads_only_unaccepted_tail() {
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
    assert_eq!(
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |next| {
                if next == 2 {
                    service.set_preparation_suspended(true);
                }
                Ok(())
            }
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
    // Resume must not reconstruct accepted frames. Earlier table sections can
    // still supply native anchors for the tail, so preserve them and their directory.
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
    service.set_preparation_suspended(false);
    service.prepare_library(&[]).unwrap();
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
        State::Ready,
        "preparation failed: {:?}",
        status.error
    );
    assert_eq!(unprepared_blocks(&source, None).unwrap(), Some(0));
    assert!(
        source
            .replay_ref()
            .read_block(&source.replay_ref().index[0])
            .is_err()
    );
    service.shutdown();
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_air_force_matches_native_replies_after_two_commits_resume_append_gaps_and_aliases() {
    use crate::products::{LoadContext, Product, air_force::*};
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 3);
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![
            replay::rows::AirWingRow {
                id: 1,
                owner: 100_000,
                country_override: 100_001,
                planes: if hour < 103 { 10 } else { 20 },
                ..Default::default()
            },
            replay::rows::AirWingRow {
                id: 2,
                owner: 100_001,
                country_override: 0,
                planes: 15,
                ..Default::default()
            },
        ]));
        *frame.tables[TableId::WingEquipment.index()] =
            TableView::Complete(TableRows::WingEquipment(vec![
                replay::rows::WingEquipmentRow {
                    wing: 1,
                    ordinal: 0,
                    variant: 77,
                    amount_raw: 1_000_000,
                },
                replay::rows::WingEquipmentRow {
                    wing: 2,
                    ordinal: 0,
                    variant: 78,
                    amount_raw: 1_500_000,
                },
            ]));
        if hour >= 104
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"later air alias".to_vec();
        }
        frame
    };
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let mut fixture = Fixture::new(&first);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let check = |source: &Source| {
        for measure in [AirForceMeasure::Planes, AirForceMeasure::Wings] {
            for ownership in [AirForceOwnership::Roster, AirForceOwnership::Effective] {
                for top in [None, Some(1)] {
                    let params = AirForceParams {
                        branch_id: "1".into(),
                        tags: if top.is_some() {
                            Vec::new()
                        } else {
                            vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()]
                        },
                        top,
                        measure,
                        ownership,
                    };
                    let mut actual=crate::products::air_force::prepared::query(source,"series-summary",&serde_json::json!({"branchId":"1","tags":params.tags,"family":{"kind":"air-force-size","top":top,"measure":measure,"ownership":ownership},"from":100,"to":source.metadata().branches[0].last_hour,"width":100}),&context,&flag).unwrap();
                    actual.as_object_mut().unwrap().remove("summaries");
                    let expected = AirForceSize::load(source, &params, &context, &flag).unwrap();
                    assert_eq!(
                        actual,
                        serde_json::to_value(expected).unwrap(),
                        "{measure:?} {ownership:?} {top:?}"
                    );
                }
            }
        }
    };
    let source = fixture.source("air-first");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let second = first
        .append(&[continuation_records(&first, 1, vec![make(103)])])
        .unwrap();
    fixture.replace(&second);
    let source = fixture.source("air-second");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let resumed = fixture.source("air-resume");
    assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 0);
    check(&resumed);
    let grown = second
        .append(&[continuation_records(&second, 1, vec![make(104)])])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.source("air-append");
    assert_eq!(run(&appended, None, &flag, |_| {}).unwrap().new_blocks, 1);
    check(&appended);
    let unchanged = grown
        .append(&[continuation_records(&grown, 1, vec![make(105)])])
        .unwrap();
    fixture.replace(&unchanged);
    let unchanged = fixture.source("air-unchanged-after-commit");
    assert_eq!(run(&unchanged, None, &flag, |_| {}).unwrap().new_blocks, 1);
    check(&unchanged);
    let (store, key) = super::super::files(&unchanged, None).unwrap();
    assert_eq!(store.range_count(&key, "air-mixes-1").unwrap(), Some(4));
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_air_force_top_first_page_at_five_thousand_countries_uses_rank_positions() {
    use crate::products::{LoadContext, Product, air_force::*};
    let mut frame = scale_frame(1, 100, 5000);
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
        (0..5000)
            .map(|index| replay::rows::AirWingRow {
                id: u64::from(index),
                owner: 100_000 + index,
                planes: (index % 101) as i32,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::WingEquipment.index()] =
        TableView::Complete(TableRows::WingEquipment(Vec::new()));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("air-scale");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let context = LoadContext { content: None };
    let started = Instant::now();
    let mut actual=crate::products::air_force::prepared::query(&source,"series-summary",&serde_json::json!({"branchId":"1","tags":[],"family":{"kind":"air-force-size","top":10,"measure":"planes","ownership":"roster"},"from":100,"to":100,"width":1}),&context,&flag).unwrap();
    eprintln!(
        "prepared air top-10 / 5000 countries: {:?}, {} bytes",
        started.elapsed(),
        serde_json::to_vec(&actual).unwrap().len()
    );
    assert_eq!(actual["series"].as_array().unwrap().len(), 10);
    actual.as_object_mut().unwrap().remove("summaries");
    let expected = AirForceSize::load(
        &source,
        &AirForceParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: Some(10),
            measure: AirForceMeasure::Planes,
            ownership: AirForceOwnership::Roster,
        },
        &context,
        &flag,
    )
    .unwrap();
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_navy_matches_native_after_two_commits_resume_append_gaps_aliases_and_five_thousand_countries()
 {
    use crate::products::{LoadContext, Product, navy::*};
    use replay::rows::{LexicalTokenRow, ShipCounterInputs, ShipModelRow, ShipRow, TaskForceRow};
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 5000);
        *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
            (0..5000)
                .map(|index| TaskForceRow {
                    id: u64::from(index) + 1,
                    roster_owner: 100_000 + index,
                    logical_owner: 100_000 + (index + 1) % 5000,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(
            (0..5000_u32)
                .map(|index| ShipRow {
                    id: u64::from(index) + 1,
                    task_force: u64::from(index) + 1,
                    class_id: 41,
                    strength: if index == 0 && hour >= 103 {
                        700_000
                    } else {
                        i64::from(index % 101) * 100_000
                    },
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(
            (0..5000_u32)
                .map(|index| ShipModelRow {
                    ship: u64::from(index) + 1,
                    counter: Some(ShipCounterInputs {
                        maximum_strength: if hour >= 104 { 20_000_000 } else { 10_000_000 },
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 41,
                literal: if hour >= 104 {
                    b"later_ship".to_vec()
                } else {
                    b"first_ship".to_vec()
                },
            }]));
        if hour >= 104
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"later navy alias".to_vec();
        }
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let check = |source: &Source| {
        for measure in [NavyMeasure::Ships, NavyMeasure::Strength] {
            for ownership in [NavyOwnership::Roster, NavyOwnership::Logical] {
                for grouping in [NavyGrouping::Class, NavyGrouping::Type] {
                    for top in [None, Some(10)] {
                        let params = NavyParams {
                            branch_id: "1".into(),
                            tags: if top.is_some() {
                                vec![]
                            } else {
                                vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()]
                            },
                            top,
                            measure,
                            ownership,
                            grouping,
                        };
                        let now = Instant::now();
                        let mut actual=crate::products::navy::prepared::query(source,"series-summary",&serde_json::json!({"branchId":"1","tags":params.tags,"family":{"kind":"navy-size","measure":measure,"ownership":ownership,"grouping":grouping,"top":top},"from":100,"to":source.metadata().branches[0].last_hour,"width":100}),&context,&flag).unwrap();
                        if top.is_some() {
                            eprintln!("prepared navy top-10 / 5000: {:?}", now.elapsed());
                        }
                        actual.as_object_mut().unwrap().remove("summaries");
                        let expected = NavySize::load(source, &params, &context, &flag).unwrap();
                        assert_eq!(
                            actual,
                            serde_json::to_value(expected).unwrap(),
                            "{measure:?} {ownership:?} {grouping:?} {top:?}"
                        );
                    }
                }
            }
        }
    };
    let source = fixture.source("navy-first");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let second = replay
        .append(&[continuation_records(&replay, 1, vec![make(103)])])
        .unwrap();
    fixture.replace(&second);
    let source = fixture.source("navy-second");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let resumed = fixture.source("navy-resume");
    assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 0);
    check(&resumed);
    let third = second
        .append(&[continuation_records(&second, 1, vec![make(104)])])
        .unwrap();
    fixture.replace(&third);
    let source = fixture.source("navy-append");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    check(&source);
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_eq!(store.range_count(&key, "navy-mixes-1-0").unwrap(), Some(3));
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_country_nukes_matches_native_after_two_commits_resume_append_gaps_aliases_and_five_thousand_countries()
 {
    use crate::products::{LoadContext, Product, country_nukes::*};
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 5000);
        *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
            (0..5000)
                .map(|index| replay::rows::NukeRow {
                    country: 100_000 + index,
                    nuke_type: if index == 1 && hour >= 104 { 1 } else { 0 },
                    amount_raw: i64::from(if hour == 103 { 0 } else { index % 101 }) * 100_000_000,
                    ..Default::default()
                })
                .collect(),
        ));
        if hour >= 104
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"later nuclear alias".to_vec();
        }
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let check = |source: &Source| {
        for top in [None, Some(10)] {
            let params = NukesParams {
                branch_id: "1".into(),
                tags: if top.is_some() {
                    vec![]
                } else {
                    vec![String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap()]
                },
                top,
                origin_hour: None,
            };
            let started = Instant::now();
            let mut actual=crate::products::country_nukes::prepared::query(source,"series-summary",&serde_json::json!({"branchId":"1","tags":params.tags,"family":{"kind":"country-nukes","top":top,"originHour":null},"from":100,"to":source.metadata().branches[0].last_hour,"width":100}),&context,&flag).unwrap();
            if top.is_some() {
                eprintln!("prepared nukes top-10 / 5000: {:?}", started.elapsed());
            }
            assert_eq!(
                actual["coverage"]["complete"].as_bool(),
                Some(true),
                "nuclear parity reads must have complete prepared coverage"
            );
            for field in ["summaries", "coverage"] {
                actual.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(
                actual,
                serde_json::to_value(CountryNukes::load(source, &params, &context, &flag).unwrap())
                    .unwrap()
            );
        }
    };
    let source = fixture.source("nukes-first");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let second = replay
        .append(&[continuation_records(&replay, 1, vec![make(103)])])
        .unwrap();
    fixture.replace(&second);
    let source = fixture.source("nukes-second");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let resumed = fixture.source("nukes-resume");
    assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 0);
    check(&resumed);
    let third = second
        .append(&[continuation_records(&second, 1, vec![make(104)])])
        .unwrap();
    fixture.replace(&third);
    let source = fixture.source("nukes-append");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    check(&source);
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut reads =
        crate::prepared::series::Reader::new(&store, &key, "nukes-mixes-1".into(), &flag).unwrap();
    let page = reads.events(&[u64::MAX], 100, 104, 0, 10).unwrap();
    assert_eq!(
        page.rows.iter().map(|row| row.hour).collect::<Vec<_>>(),
        vec![100, 101, 103, 104]
    );
    assert_eq!(page.rows[2].row, serde_json::json!(true));
    let state = load_branch(
        &store,
        &key,
        Some(&key),
        source.replay_ref(),
        1,
        100,
        &Refused::default(),
    )
    .unwrap();
    let json = serde_json::to_value(&state.nukes).unwrap();
    assert_eq!(json["scan"]["scanned"]["read_hours"], serde_json::json!([]));
    assert_eq!(
        json["scan"]["scanned"]["empty_read_hours"],
        serde_json::json!([])
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_country_factories_matches_native_after_two_commits_resume_append_gaps_aliases_and_five_thousand_countries()
 {
    use crate::products::{LoadContext, Product, country_factories::*};
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 5000);
        *frame.tables[TableId::CountryFactories.index()] =
            TableView::Complete(TableRows::CountryFactories(
                (0..5000)
                    .map(|index| replay::rows::CountryFactoriesRow {
                        country: 100_000 + index,
                        military_total_raw: i64::from(index % 101) * 100_000,
                        naval_total_raw: i64::from(index % 17) * 100_000,
                        civilian_total_raw: i64::from(if index == 0 && hour >= 103 {
                            1000
                        } else {
                            index % 73
                        }) * 100_000,
                        civilian_damaged_raw: -100_000,
                        consumer_goods: if hour >= 104 { 3 } else { 2 },
                        civilian_unavailable: if hour >= 104 { 3 } else { 2 },
                        ..Default::default()
                    })
                    .collect(),
            ));
        if hour >= 104
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"later factory alias".to_vec();
        }
        frame
    };
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![make(100), make(101)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let check = |source: &Source| {
        let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
        for mode in [
            FactoryCount::Free,
            FactoryCount::Built,
            FactoryCount::TopBar,
        ] {
            for mask in 1..=7 {
                for ranked in [false, true] {
                    let groups: Vec<_> = FactoryGroupKey::ALL
                        .into_iter()
                        .enumerate()
                        .filter_map(|(bit, group)| (mask & (1 << bit) != 0).then_some(group))
                        .collect();
                    let top = ranked.then(|| FactoryTop {
                        count: 10,
                        mode,
                        groups: groups.clone(),
                    });
                    let params = FactoriesParams {
                        branch_id: "1".into(),
                        tags: if ranked { vec![] } else { vec![tag.clone()] },
                        top: top.clone(),
                    };
                    let started = Instant::now();
                    let mut actual=crate::products::country_factories::prepared::query(source,"series-summary",&serde_json::json!({"branchId":"1","tags":params.tags,"family":{"kind":"country-factories","top":top,"mode":mode,"groups":groups},"from":100,"to":source.metadata().branches[0].last_hour,"width":100}),&context,&flag).unwrap();
                    if ranked && mask == 7 {
                        eprintln!(
                            "prepared factories top-10 / 5000 {mode:?}: {:?}",
                            started.elapsed()
                        );
                    }
                    assert_eq!(
                        actual["coverage"]["complete"].as_bool(),
                        Some(true),
                        "factory parity reads must have complete prepared coverage"
                    );
                    for field in ["summaries", "coverage"] {
                        actual.as_object_mut().unwrap().remove(field);
                    }
                    let expected =
                        CountryFactories::load(source, &params, &context, &flag).unwrap();
                    assert_eq!(
                        actual,
                        serde_json::to_value(expected).unwrap(),
                        "{mode:?} {mask} {ranked}"
                    );
                }
            }
        }
    };
    let source = fixture.source("factories-first");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let second = replay
        .append(&[continuation_records(&replay, 1, vec![make(103)])])
        .unwrap();
    fixture.replace(&second);
    let source = fixture.source("factories-second");
    run(&source, None, &flag, |_| {}).unwrap();
    check(&source);
    let resumed = fixture.source("factories-resume");
    assert_eq!(run(&resumed, None, &flag, |_| {}).unwrap().new_blocks, 0);
    check(&resumed);
    let third = second
        .append(&[continuation_records(&second, 1, vec![make(104)])])
        .unwrap();
    fixture.replace(&third);
    let source = fixture.source("factories-append");
    assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 1);
    check(&source);
    let (store, key) = super::super::files(&source, None).unwrap();
    assert_eq!(
        store.range_count(&key, "factories-mixes-1").unwrap(),
        Some(3)
    );
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(1)).unwrap();
    let state_reads=crate::products::country_factories::prepared::query(&source,"series-state",&serde_json::json!({"branchId":"1","family":{"kind":"country-factories","top":null,"mode":"free","groups":[]},"reads":[{"tag":tag,"hour":100},{"tag":tag,"hour":104}]}),&context,&flag).unwrap();
    let dates: Vec<_> = state_reads["series"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|country| country["runs"].as_array().unwrap())
        .map(|run| {
            assert_eq!(run["firstHour"], run["lastHour"]);
            run["firstHour"].as_i64().unwrap()
        })
        .collect();
    assert_eq!(dates, vec![100, 104]);
    let state = load_branch(
        &store,
        &key,
        Some(&key),
        source.replay_ref(),
        1,
        100,
        &Refused::default(),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&state.factories).unwrap().len();
    eprintln!("factory continuation / 5000 countries: {bytes} bytes");
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
            let oracle = UnitLeaders::load(&source, &params, &context, &flag).unwrap();
            assert_eq!(
                reply["data"],
                serde_json::to_value(oracle).unwrap(),
                "append={end} view={view:?}"
            );
        }
    }
}

#[test]
fn prepared_feed_uses_accepted_live_prefix_and_keeps_unprepared_reads_pending() {
    let frames: Vec<_> = (100..=102)
        .map(|hour| {
            let mut frame = crate::products::event_feed::prepared::tests::frame(hour, 3);
            *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
                (0..3)
                    .map(|index| replay::rows::NukeRow {
                        country: 100_000 + index,
                        amount_raw: i64::from(hour - 100) * 100_000_000,
                        ..Default::default()
                    })
                    .collect(),
            ));
            frame.metrics.table_ns[TableId::Nukes.index()] = 1;
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("feed-live-prefix");
    let flag = AtomicBool::new(false);
    run(&source, None, &flag, |_| {}).unwrap();
    let (store, key) = super::super::files(&source, None).unwrap();
    let mut writer = store.extend(&key).unwrap();
    publish_coverage(&mut writer, 1, 100, 101, false).unwrap();
    writer.commit().unwrap();
    let mut requests = 0;
    let mut query = |hour| {
        requests += 1;
        assert!(requests <= 2, "coverage checks never request-loop");
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "event-feed".into(),
            params: serde_json::json!({"branchId":"1", "lastHour":hour, "pageSize":256, "categories":["milestones"]}),
        };
        let reply: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    };
    let prefix = query(101);
    assert_eq!(prefix["coverage"]["lastHour"], 101);
    assert_eq!(prefix["coverage"]["complete"], false);
    assert_eq!(
        prefix["reply"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["kind"] == "nukesIncreased")
            .count(),
        3
    );
    assert!(
        prefix["reply"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["readHour"].as_i64().unwrap() <= 101)
    );
    assert!(
        query(102).is_null(),
        "unaccepted supplying reads remain pending rather than empty"
    );
    assert_eq!(requests, 2);
}

#[test]
fn progressive_game_score_state_reads_unprepared_hour_and_tail_only_countries_without_writing() {
    use crate::products::{
        BranchParams, LoadContext, Product,
        game_score::{self, GameScore},
    };
    use crate::transport::ProductRequest;
    use replay::rows::{CountryGameScoreInputRow, GameScoreMultiplierRow};
    fn score_frame(hour: i32, count: u32, army: i32) -> DecodedFrame {
        let mut frame = scale_frame(1, hour, count);
        for rows in [
            TableRows::CountryGameScoreInputs(
                (0..count)
                    .map(|index| CountryGameScoreInputRow {
                        country: 100_000 + index,
                        army_count: Some(army + index as i32),
                        navy_count: Some(0),
                        land_count: Some(11),
                        air_state: CountryGameScoreInputRow::AIR_ABSENT,
                        ..Default::default()
                    })
                    .collect(),
            ),
            TableRows::GameScoreMultipliers(vec![GameScoreMultiplierRow {
                army_raw: Some(100_000),
                navy_raw: Some(100_000),
                air_raw: Some(100_000),
                industry_raw: Some(100_000),
                land_raw: Some(100_000),
                total_raw: Some(100_000),
                ..Default::default()
            }]),
            TableRows::CountryFactories(Vec::new()),
            TableRows::AirWingGameScoreStats(Vec::new()),
            TableRows::AirBaseRegistrations(Vec::new()),
            TableRows::Factions(Vec::new()),
            TableRows::FactionMembers(Vec::new()),
        ] {
            let table = rows.table();
            *frame.tables[table.index()] = TableView::Complete(rows);
        }
        frame
    }
    let prefix = score_frame(100, 1, 2);
    let tail = score_frame(102, 2, 3);
    let mut next = score_frame(103, 2, 17);
    // Reusing the first tag supplies a replacement holder at this exact hour.
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows[0].id = 200_000;
        rows.sort_by_key(|row| row.id);
        rows.dedup_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *next.tables[TableId::CountryLinks.index()]
    {
        rows[0].id = 200_000;
        rows.sort_by_key(|row| row.id);
        rows.dedup_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
        &mut *next.tables[TableId::CountryGameScoreInputs.index()]
    {
        rows[0].country = 200_000;
        rows.sort_by_key(|row| row.country);
        rows.dedup_by_key(|row| row.country);
    }
    let initial =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![prefix.clone()])]).unwrap();
    let replay = initial
        .append(&[continuation_records(&initial, 1, vec![tail, next])])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("score-progressive");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::files(&source, None).unwrap();
    let query = |hour| {
        let request = ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "series-state".into(),
            params: serde_json::json!({"branchId":"1","family":{"kind":"game-score"},"reads":[{"hour":hour}]}),
        };
        let value: serde_json::Value = serde_json::from_str(
            &super::super::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap();
        let actual: game_score::prepared::GameScoreStateReply =
            serde_json::from_value(value["data"].clone()).unwrap();
        (value["data"].clone(), actual)
    };
    let (cold_value, cold) = query(103);
    assert!(cold_value["coverage"].is_null());
    assert!(cold.approximate);
    assert!(cold.stretches.is_empty());
    assert_eq!(cold.reply.reads, vec![103]);
    assert_eq!(cold.reply.countries.len(), 2);
    assert_eq!(
        cold.reply.countries[0].identity[0].value.native_country,
        Some(200_000)
    );
    let context = LoadContext { content: None };
    let reference = GameScore::load(
        &source,
        &BranchParams {
            branch_id: "1".into(),
        },
        &context,
        &flag,
    )
    .unwrap();
    for (actual, expected) in cold.reply.countries.iter().zip(&reference.countries) {
        assert_eq!(actual.tag, expected.tag);
        assert_eq!(
            actual.identity[0].value,
            expected
                .identity
                .iter()
                .find(|run| run.first_hour <= 103 && 103 <= run.last_hour)
                .unwrap()
                .value
        );
        for (actual, expected) in [
            (&actual.scores.army, &expected.scores.army),
            (&actual.scores.navy, &expected.scores.navy),
            (&actual.scores.air, &expected.scores.air),
            (&actual.scores.industry, &expected.scores.industry),
            (&actual.scores.land, &expected.scores.land),
            (&actual.scores.total, &expected.scores.total),
        ] {
            let run = expected
                .iter()
                .find(|run| run.first_hour <= 103 && 103 <= run.last_hour)
                .unwrap();
            assert_eq!(actual[0].value_at(103), run.value_at(103));
            assert_eq!(actual[0].read_each_hour, run.read_each_hour);
            assert_eq!(
                actual[0].missing.map(|index| &cold.reply.reasons[index]),
                run.missing.map(|index| &reference.reasons[index])
            );
        }
    }
    assert_eq!(
        cold.reply.countries[0].scores.army[0].raw.as_deref(),
        Some("1700000")
    );
    assert_eq!(
        cold.reply.countries[1].scores.army[0].raw.as_deref(),
        Some("1800000")
    );
    assert_eq!(
        cold.reply.multipliers.army[0].raw.as_deref(),
        Some("100000")
    );
    assert!(
        store.saved_key().unwrap().is_none(),
        "tail queries publish nothing"
    );

    let naming = game_score::Naming::load(&source, &context);
    let mut observer = naming.observer(&replay);
    let mut state = game_score::prepared::Prepared::default();
    state
        .observe(
            &prefix,
            &replay,
            &mut observer,
            &naming,
            &crate::prepared::frame_changes::FrameChangeTracker::new()
                .observe(&prefix, replay.format),
        )
        .unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    state.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: 100,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let ranges = store.range_count(&key, "game-score-1").unwrap();
    let (partial_value, partial) = query(103);
    assert_eq!(partial_value["coverage"]["lastHour"], 100);
    assert!(partial.approximate);
    assert_eq!(
        serde_json::to_value(&partial.reply).unwrap(),
        serde_json::to_value(&cold.reply).unwrap()
    );
    let (_, prepared) = query(100);
    assert!(!prepared.approximate);
    assert!(!prepared.stretches.is_empty());
    let (_, gap) = query(101);
    assert!(gap.reply.reads.is_empty());
    assert!(gap.reply.countries.is_empty());
    assert_eq!(store.range_count(&key, "game-score-1").unwrap(), ranges);
    let request = ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "series-state".into(),
        params: serde_json::json!({"branchId":"1","family":{"kind":"game-score"},"reads":[{"hour":103}]}),
    };
    assert_eq!(
        super::super::query::query(&source, &request, None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

/// Diagnostic only: print actual compiled layouts without constructing domain state.
/// Run before and after the heap-ownership fix to identify every large aggregate.
#[test]
fn preparation_stack_type_sizes() {
    fn report<T>(label: &str) {
        println!(
            "stack-layout {label}: type={} size={} align={} box={} option={}",
            std::any::type_name::<T>(),
            std::mem::size_of::<T>(),
            std::mem::align_of::<T>(),
            std::mem::size_of::<Box<T>>(),
            std::mem::size_of::<Option<T>>()
        );
    }
    report::<BranchState>("BranchState");
    report::<Checkpoint>("Checkpoint");
    report::<Accepted>("Accepted");
    report::<replay::frame::DecodedFrame>("DecodedFrame");
    report::<replay::frame::Frame>("Frame");
    report::<Source>("Source");
    report::<serde_json::Value>("Value");
    report::<super::super::series::Accumulator>("series.Accumulator");
    report::<FocusAccumulator>("focus");
    report::<FocusIndex>("index");
    report::<ProductionAccumulator>("production");
    report::<MeasureAccumulator>("measure");
    report::<crate::products::land_battles::Census>("land");
    report::<crate::products::naval_battles::Scan>("naval");
    report::<crate::products::division_log::Scan>("divisions");
    report::<crate::products::country_history::HistoryScan>("identities");
    report::<crate::products::army_size::prepared::Prepared>("army");
    report::<crate::products::air_force::prepared::Prepared>("air");
    report::<crate::products::navy::prepared::Prepared>("navy");
    report::<crate::products::country_nukes::prepared::Prepared>("nukes");
    report::<crate::products::country_factories::prepared::Prepared>("factories");
    report::<crate::products::supply_health::prepared::Prepared>("supply");
    report::<crate::products::construction_log::prepared::Prepared>("construction");
    report::<crate::products::resource_balance::prepared::Prepared>("resources");
    report::<crate::products::trade_flows::prepared::Prepared>("trade");
    report::<crate::products::equipment_stockpile::prepared::Prepared>("stockpile");
    report::<crate::products::country_groups::Scan>("groups");
    report::<crate::products::military_air_ledger::prepared::Prepared>("air_ledger");
    report::<crate::products::air_aces::prepared::Prepared>("aces");
    report::<crate::products::raids::prepared::Prepared>("raids");
    report::<crate::products::air_superiority::prepared::Prepared>("air_superiority");
    report::<crate::products::subjects::Scan>("subjects");
    report::<crate::products::autonomy::ScannedAutonomy>("autonomy");
    report::<crate::products::world_tension::Scan>("tension");
    report::<crate::products::wars::prepared::Observer>("wars");
    report::<crate::products::idea_history::Scan>("ideas");
    report::<crate::products::party_history::prepared::Prepared>("parties");
    report::<crate::products::intelligence_prepared::Observer>("intelligence");
    report::<crate::prepared_stability::StabilityAccumulator>("stability");
    report::<crate::products::politics_ledger::prepared::Prepared>("ledger");
    report::<crate::products::country_fuel::prepared::Prepared>("fuel");
    report::<crate::products::territory::prepared::Prepared>("territory");
    report::<crate::products::occupied_states::prepared::Prepared>("occupation");
    report::<crate::products::game_score::prepared::Prepared>("game_score");
    report::<crate::products::unit_leaders::prepared::Prepared>("leaders");
    report::<crate::products::research::prepared::Prepared>("research");
    report::<crate::products::peace_deals::prepared::Prepared>("peace");
    report::<crate::products::special_projects::prepared::Prepared>("projects");
    report::<crate::products::war_casualties::prepared::Prepared>("casualties");
    report::<crate::products::equipment_losses::prepared::Prepared>("equipment_losses");
    report::<crate::products::decryption::prepared::Accumulator>("intelligence.decryption");
    report::<crate::products::operative_placements::prepared::Prepared>("intelligence.placements");
    report::<crate::products::agency_comparison::prepared::Prepared>("intelligence.agencies");
    report::<crate::products::operatives_roster::prepared::Prepared>("intelligence.roster");
    report::<crate::products::espionage_log::prepared::Prepared>("intelligence.espionage");
    report::<crate::products::territory_breakdown::Prepared>("territory.breakdown");
    report::<[super::super::series::Accumulator; 2]>("series.Accumulator[2]");
    report::<[super::super::series::Accumulator; 4]>("series.Accumulator[4]");
    report::<[super::super::series::Accumulator; 6]>("series.Accumulator[6]");
    report::<[super::super::series::Accumulator; 8]>("series.Accumulator[8]");
    report::<[super::super::series::Accumulator; 9]>("series.Accumulator[9]");
    report::<[super::super::series::Accumulator; 11]>("series.Accumulator[11]");
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
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: full preparation, resume, append and query stack regression"
)]
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
                    for product in ["country-groups", "subjects"] {
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
