use super::*;
use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    container::Replay,
    frame::{Frame, Metrics, TableUpdate},
    identity::{Identity, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, ClockRow, TableId, TableRows},
    spool::Record,
};
use std::{cell::Cell, fs};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4-app-compaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("synthetic.battleplan");
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "synthetic".into(),
                recorder_version: "synthetic".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        };
        let mut records = vec![
            Record::SessionStart(session),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: Some(0),
                first_hour: 1,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        for hour in 1..=3 {
            // Only the campaign and clock are recorded; tables that must hold one
            // row when complete stay unavailable.
            let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: "synthetic unavailable table".into(),
            });
            tables[TableId::Campaign.index()] =
                TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                    id: b"synthetic".to_vec(),
                }]));
            tables[TableId::Clock.index()] =
                TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]));
            records.push(Record::Frame(Frame {
                branch: 1,
                hour,
                keyframe: hour == 1,
                metrics: Metrics::default(),
                tables,
            }));
        }
        Replay::from_records(b"synthetic".to_vec(), &[records])
            .unwrap()
            .write_atomic(&path)
            .unwrap();
        Self(path)
    }
    fn clean(&self) {
        assert_eq!(fs::read_dir(self.0.parent().unwrap()).unwrap().count(), 1);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn automatic_requires_small_game_and_recorder_exit_and_manual_ignores_both() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.0).unwrap();
    for (preference, game, recorder) in [
        (RecordingCompression::Fast, false, false),
        (RecordingCompression::Fast, true, false),
        (RecordingCompression::Small, true, false),
        (RecordingCompression::Small, false, true),
    ] {
        let error = compact_in_environment(
            &fixture.0,
            true,
            || Ok((preference, game, recorder)),
            |_, _| panic!("ineligible automatic work started"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(&fixture.0).unwrap(), before);
    }
    assert!(
        compact_in_environment(
            &fixture.0,
            true,
            || Ok((RecordingCompression::Small, false, false)),
            |_, _| Ok(())
        )
        .is_ok()
    );
    for preference in [RecordingCompression::Fast, RecordingCompression::Small] {
        let calls = Cell::new(0);
        let result = compact_in_environment(
            &fixture.0,
            false,
            || {
                calls.set(calls.get() + 1);
                Ok((preference, true, true))
            },
            |_, _| Ok(()),
        )
        .unwrap();
        assert!(matches!(
            result.outcome,
            CompactionOutcome::AlreadySmall | CompactionOutcome::Compacted
        ));
        assert_eq!(
            calls.get(),
            0,
            "manual work never consults the automatic gate"
        );
    }
    fixture.clean();
}

#[test]
fn new_game_recorder_or_fast_preference_cancels_automatic_work_promptly_and_cleans_candidate() {
    for new_environment in [
        (RecordingCompression::Small, true, false),
        (RecordingCompression::Small, false, true),
        (RecordingCompression::Fast, false, false),
    ] {
        let fixture = Fixture::new();
        let before = fs::read(&fixture.0).unwrap();
        let changed = Cell::new(false);
        let verified = Cell::new(false);
        let error = compact_in_environment(
            &fixture.0,
            true,
            || {
                Ok(if changed.get() {
                    new_environment
                } else {
                    (RecordingCompression::Small, false, false)
                })
            },
            |stage, _| {
                if stage == CompactionStage::TempCreated {
                    changed.set(true);
                    thread::sleep(Duration::from_millis(260));
                }
                if stage == CompactionStage::Verified {
                    verified.set(true);
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(
            !verified.get(),
            "stop at the next streaming checkpoint, before verification completes"
        );
        assert_eq!(fs::read(&fixture.0).unwrap(), before);
        fixture.clean();
    }
}

#[test]
fn automatic_checkpoints_do_not_overwrite_manual_results_and_manual_requests_cancel_them() {
    let inner = Inner::default();
    let manual = CompactionProgress {
        total: 3,
        completed: 3,
        compacted: 1,
        bytes_saved: 1234,
        unchanged: vec![CompactionFailure {
            path: "synthetic".into(),
            reason: "Busy; try again later".into(),
        }],
        ..Default::default()
    };
    *inner.progress.lock().unwrap() = manual.clone();
    checkpoint(&inner, true, CompactionStage::Started { blocks: 10 }).unwrap();
    checkpoint(&inner, true, CompactionStage::BlockWritten(5)).unwrap();
    assert_eq!(*inner.progress.lock().unwrap(), manual);
    inner.manual_active.store(true, Ordering::Release);
    assert_eq!(
        checkpoint(&inner, true, CompactionStage::BeforeReplace)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(*inner.progress.lock().unwrap(), manual);
    checkpoint(&inner, false, CompactionStage::Started { blocks: 2 }).unwrap();
    assert_eq!(inner.progress.lock().unwrap().blocks_total, 2);
}

#[test]
fn manual_results_report_every_unchanged_outcome_without_size_increases() {
    let fixture = Fixture::new();
    let revision = replay_revision(&fixture.0).unwrap();
    let mut progress = CompactionProgress::default();
    for (outcome, after) in [
        (CompactionOutcome::Compacted, 75),
        (CompactionOutcome::AlreadySmall, 100),
        (CompactionOutcome::Changed, 100),
    ] {
        record_manual_result(
            &mut progress,
            &fixture.0,
            Ok(CompactionResult {
                bytes_before: 100,
                bytes_after: after,
                blocks_encoded: 1,
                outcome,
                revision: revision.clone(),
            }),
        );
    }
    record_manual_result(
        &mut progress,
        &fixture.0,
        Err(io::Error::new(io::ErrorKind::WouldBlock, "held")),
    );
    record_manual_result(
        &mut progress,
        &fixture.0,
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Still being recorded",
        )),
    );
    assert_eq!(progress.bytes_saved, 25);
    assert_eq!(progress.compacted, 1);
    assert_eq!(progress.skipped, 4);
    assert_eq!(progress.completed, 5);
    assert_eq!(
        progress
            .unchanged
            .iter()
            .map(|item| item.reason.as_str())
            .collect::<Vec<_>>(),
        [
            "Already as small as it gets",
            "Changed while compacting; original kept",
            "Busy; try again later",
            "Still being recorded"
        ]
    );
}

#[test]
fn loaded_recordings_skip_automatic_work_but_remain_manually_compactable() {
    let fixture = Fixture::new();
    let inner = Inner::default();
    inner
        .loaded
        .lock()
        .unwrap()
        .insert(store::key(&fixture.0), 1);
    let before = fs::read(&fixture.0).unwrap();
    let error = compact_in_host(
        &inner,
        &fixture.0,
        true,
        || Ok((RecordingCompression::Small, false, false)),
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(fs::read(&fixture.0).unwrap(), before);
    compact_in_host(
        &inner,
        &fixture.0,
        false,
        || panic!("manual work checked the recorder/game/preference"),
        |_| Ok(()),
    )
    .unwrap();
    assert!(is_loaded(&inner, &fixture.0));
    fixture.clean();
}

#[test]
fn opening_a_recording_cancels_automatic_frame_work() {
    let fixture = Fixture::new();
    let inner = Inner::default();
    let before = fs::read(&fixture.0).unwrap();
    let error = compact_in_host(
        &inner,
        &fixture.0,
        true,
        || Ok((RecordingCompression::Small, false, false)),
        |stage| {
            if matches!(stage, CompactionStage::FrameEncoded { frame: 0, .. }) {
                inner
                    .loaded
                    .lock()
                    .unwrap()
                    .insert(store::key(&fixture.0), 1);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(fs::read(&fixture.0).unwrap(), before);
    fixture.clean();
}

#[test]
fn game_start_during_frame_verification_stops_before_the_next_frame() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.0).unwrap();
    let game = Cell::new(false);
    let frames = Cell::new(0);
    let error = compact_in_environment(
        &fixture.0,
        true,
        || Ok((RecordingCompression::Small, game.get(), false)),
        |stage, _| {
            if matches!(stage, CompactionStage::FrameVerified { .. }) {
                frames.set(frames.get() + 1);
                game.set(true);
                thread::sleep(Duration::from_millis(260));
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(frames.get(), 1);
    assert_eq!(fs::read(&fixture.0).unwrap(), before);
    fixture.clean();
}

#[test]
fn closed_app_cancels_frame_work_and_library_traversal() {
    let fixture = Fixture::new();
    let inner = Inner::default();
    let before = fs::read(&fixture.0).unwrap();
    let error = compact_in_host(
        &inner,
        &fixture.0,
        false,
        || panic!("manual environment check"),
        |stage| {
            if matches!(stage, CompactionStage::FrameEncoded { frame: 0, .. }) {
                inner.closed.store(true, Ordering::Release);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let mut checked = Instant::now();
    let error = crate::replay_library::scan_with(fixture.0.parent().unwrap(), || {
        scan_checkpoint(&inner, false, &mut checked)
    })
    .err()
    .unwrap();
    assert!(error.contains("cancelled"));
    assert_eq!(fs::read(&fixture.0).unwrap(), before);
    fixture.clean();
}

#[test]
fn shutdown_does_not_wait_for_an_owned_worker_inside_a_long_step() {
    let host = CompactionHost::default();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        wait.recv().unwrap();
    });
    host.0.workers.lock().unwrap().push(worker);
    let host = Arc::new(host);
    let shutting_down = Arc::clone(&host);
    let (done, completed) = std::sync::mpsc::channel();
    let shutdown = thread::spawn(move || {
        shutting_down.shutdown();
        done.send(()).unwrap();
    });
    let result = completed.recv_timeout(Duration::from_millis(500));
    release.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(result.is_ok(), "shutdown waited for a worker's long step");
    assert!(host.0.closed.load(Ordering::Acquire));
}
