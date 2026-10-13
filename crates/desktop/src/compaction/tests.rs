use super::*;
use replay::{
    branches::{BranchReason, BranchStart, Parentage},
    container::Replay,
    frame::{Frame, Metrics, TableUpdate},
    identity::{Identity, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, ClockRow, TableId, TableRows},
    spool::Record,
};
use std::{fs, time::Duration};

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
fn the_app_never_compacts_on_its_own() {
    // The recorder compacts after the game closes; starting the app only loads hints.
    let host = CompactionHost::default();
    host.start().unwrap();
    assert!(host.0.workers.lock().unwrap().is_empty());
    assert!(!host.0.active.load(Ordering::Acquire));
    assert_eq!(
        *host.0.progress.lock().unwrap(),
        CompactionProgress::default()
    );
}

#[test]
fn manual_compaction_runs_whatever_the_preference_game_or_recorder() {
    let fixture = Fixture::new();
    let inner = Inner::default();
    let result = compact_in_host(&inner, &fixture.0, |_| Ok(())).unwrap();
    assert!(matches!(
        result.outcome,
        CompactionOutcome::AlreadySmall | CompactionOutcome::Compacted
    ));
    fixture.clean();
}

#[test]
fn manual_checkpoints_report_block_progress() {
    let inner = Inner::default();
    checkpoint(&inner, CompactionStage::Started { blocks: 10 }).unwrap();
    checkpoint(&inner, CompactionStage::BlockWritten(5)).unwrap();
    let progress = inner.progress.lock().unwrap().clone();
    assert_eq!((progress.blocks_total, progress.blocks_completed), (10, 6));
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
fn closed_app_cancels_frame_work_and_library_traversal() {
    let fixture = Fixture::new();
    let inner = Inner::default();
    let before = fs::read(&fixture.0).unwrap();
    let error = compact_in_host(&inner, &fixture.0, |stage| {
        if matches!(stage, CompactionStage::FrameEncoded { frame: 0, .. }) {
            inner.closed.store(true, Ordering::Release);
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let error =
        crate::replay_library::scan_with(fixture.0.parent().unwrap(), || scan_checkpoint(&inner))
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
