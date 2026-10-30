use super::*;
use replay::columnar_frames::{Limits, Recording, Segment};
use replay::native_frame::{OneThread, RowFrame};

/// A row encoder's view of an hour that is no keyframe, lending no baselines.
fn row() -> RowFrame<'static> {
    RowFrame {
        baselines: None,
        keyframe: false,
        changed: &[],
    }
}
use replay_data::{
    identity::{CaptureMode, Identity, SampleBoundary, Verification},
    rows::{CountryRow, TABLE_COUNT, TableId, TableRows},
    scope,
};
use replay_frame::frame::{Metrics, TableInput, TableView};

/// Removes the directory even when an assertion fails.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("hoi4-columnar-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn session() -> SessionStart {
    SessionStart {
        id: 1,
        process_id: 100,
        process_start_time: 1000,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "fixture".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "english".into(),
            recorder_version: "fixture".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    }
}

/// Hour `hour` of `branch`: two countries read complete, every other table unavailable.
fn frame(branch: u64, hour: i32) -> FrameInput {
    let rows = (1..=2)
        .map(|id| CountryRow {
            id,
            canonical_slot: 1_000 + id,
            tag: format!("T{id}").into_bytes(),
            stability_raw: i64::from(hour % 5) * 1_000 + i64::from(id),
            ..Default::default()
        })
        .collect();
    let mut tables: [TableInput; TABLE_COUNT] = std::array::from_fn(|index| {
        TableInput::Sampled(if scope::records(CaptureMode::Full, TableId::ALL[index]) {
            TableView::Unavailable {
                reason: "not in this test".into(),
            }
        } else {
            TableView::NotRecorded {
                mode: CaptureMode::Full,
            }
        })
    });
    tables[TableId::Countries.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::Countries(rows)));
    let mut table_ns = [0; TABLE_COUNT];
    table_ns[TableId::Countries.index()] = 100;
    FrameInput {
        branch,
        hour,
        tables,
        metrics: Metrics {
            total_ns: 1_000,
            table_ns,
            pool_bytes: 7,
            dropped_frames: 0,
        },
    }
}

fn write(columnar: &mut Columnar, branch: u64, hours: std::ops::RangeInclusive<i32>) {
    for hour in hours {
        let prepared = columnar.prepare(&frame(branch, hour), row(), &OneThread).expect("prepared");
        columnar.commit(prepared, false, &OneThread);
    }
}

/// A branch's hours, a gap, then another branch: three runs, the first crossing a unit
/// position and every run reading back from its start, all under the finished name in a
/// campaign directory the first hour creates, as the row spool creates it only later.
#[test]
fn hours_across_a_gap_and_a_branch_change_finish_as_runs_of_one_file() {
    let root = Folder::new("runs");
    let folder = Folder(root.0.join("campaign"));
    let mut columnar = Columnar::open(&folder.0, "campaign", &session());
    let first = envelope::UNIT_ORDINALS as i32 + 3;
    write(&mut columnar, 1, 1..=first);
    write(&mut columnar, 1, first + 5..=first + 6);
    write(&mut columnar, 2, first + 7..=first + 7);
    columnar.finish();
    assert_eq!(columnar.status().unwrap()["stopped"], Value::Null);
    assert!(!folder.0.join(PARTIAL).exists());
    let recording = Recording::open(fs::read(folder.0.join(FINISHED)).unwrap(), Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), first as u64 + 3);
    let capture = recording.capture().unwrap().expect("a capture in the root");
    let mut encoded = Vec::new();
    session().encode(&mut encoded).unwrap();
    assert_eq!(capture.campaign_id, b"campaign");
    assert_eq!(capture.sessions, [encoded]);
    assert_eq!(capture.run_sessions, [0, 0, 0]);
    assert_eq!(capture.stop, None);
    assert_eq!(
        recording.segments(),
        [
            Segment { run: 0, restart: None, first: 0, last: first as u32 - 1 },
            Segment { run: 1, restart: None, first: 0, last: 1 },
            Segment { run: 2, restart: None, first: 0, last: 0 },
        ]
    );
    let epochs: Vec<u8> = (0..3).map(|run| recording.run(run).unwrap().epoch).collect();
    assert_eq!(
        epochs,
        [envelope::epoch::START, envelope::epoch::GAP, envelope::epoch::BRANCH]
    );
    let mut state = replay_columnar::walk::State::new(schema().unwrap()).unwrap();
    let mut walked = 0;
    recording
        .walk(&mut state, |_, _, _| {
            walked += 1;
            Ok(true)
        })
        .unwrap();
    assert_eq!(walked, first as usize + 3);
}

/// A row write failure starts the next hour's run even when the hours continue, and a
/// columnar failure stops the file at its last hour, which the root names.
#[test]
fn a_write_failure_starts_a_run_and_a_stop_is_named_in_the_root() {
    let folder = Folder::new("interrupt");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session());
    write(&mut columnar, 1, 1..=2);
    columnar.interrupt();
    write(&mut columnar, 1, 3..=3);
    columnar.finish();
    let recording = Recording::open(fs::read(folder.0.join(FINISHED)).unwrap(), Limits::default()).unwrap();
    assert_eq!(recording.segments().len(), 2);
    assert_eq!(recording.run(1).unwrap().epoch, envelope::epoch::WRITE_FAILURE);

    let failed = Folder::new("failed");
    let mut columnar = Columnar::open(&failed.0, "campaign", &session());
    write(&mut columnar, 1, 1..=2);
    let _uncommitted = columnar.prepare(&frame(1, 3), row(), &OneThread).expect("prepared");
    // Preparing the hour again before its commit would lose the first preparation's strings.
    assert!(columnar.prepare(&frame(1, 3), row(), &OneThread).is_none());
    assert!(columnar.status().unwrap()["stopped"].is_string());
    assert!(columnar.prepare(&frame(1, 4), row(), &OneThread).is_none());
    columnar.finish();
    assert!(!failed.0.join(PARTIAL).exists());
    let recording = Recording::open(fs::read(failed.0.join(FINISHED)).unwrap(), Limits::default()).unwrap();
    assert_eq!(recording.hours(), 2);
    let stop = recording.capture().unwrap().unwrap().stop.expect("a stop record");
    assert_eq!((stop.branch, stop.last_hour), (1, Some(2)));
    assert!(!stop.reason.is_empty());

    // A recording that stopped before holding an hour leaves no file.
    let empty = Folder::new("empty");
    let mut columnar = Columnar::open(&empty.0, "campaign", &session());
    let _uncommitted = columnar.prepare(&frame(1, 1), row(), &OneThread).expect("prepared");
    assert!(columnar.prepare(&frame(1, 1), row(), &OneThread).is_none());
    columnar.finish();
    assert!(!empty.0.join(PARTIAL).exists());
    assert!(!empty.0.join(FINISHED).exists());
}

/// Queued hours reach the file only once their rows' write completes; a write failure drops
/// the hours still waiting and starts the next run, so the file never holds an hour the rows
/// lost.
#[test]
fn queued_hours_wait_for_their_rows_and_a_failure_drops_them() {
    let folder = Folder::new("queued");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session());
    for hour in 1..=4 {
        let prepared = columnar.prepare(&frame(1, hour), row(), &OneThread).expect("prepared");
        columnar.commit(prepared, true, &OneThread);
    }
    assert_eq!(columnar.status().unwrap()["pending"], 4);
    columnar.complete(1, 2);
    assert_eq!(columnar.status().unwrap()["pending"], 2);
    // Hours 3 and 4 were cancelled with the rows' failed write.
    columnar.interrupt();
    for hour in 5..=6 {
        let prepared = columnar.prepare(&frame(1, hour), row(), &OneThread).expect("prepared");
        columnar.commit(prepared, true, &OneThread);
        columnar.complete(1, hour);
    }
    columnar.finish();
    let recording = Recording::open(fs::read(folder.0.join(FINISHED)).unwrap(), Limits::default()).unwrap();
    assert_eq!(recording.hours(), 4);
    assert_eq!(recording.run(0).unwrap().ordinals, 2);
    assert_eq!(recording.run(1).unwrap().first_hour, 5);
    assert_eq!(recording.run(1).unwrap().epoch, envelope::epoch::WRITE_FAILURE);
}

#[test]
fn an_off_recording_holds_nothing_and_reports_nothing() {
    let mut columnar = Columnar::off();
    assert!(columnar.prepare(&frame(1, 1), row(), &OneThread).is_none());
    columnar.complete(1, 1);
    columnar.interrupt();
    columnar.finish();
    assert!(columnar.status().is_none());
}

/// A branch change while the old branch's hours still wait for their rows starts the file's
/// next run only once those hours are written.
#[test]
fn a_run_starts_in_the_file_after_the_hours_queued_before_it() {
    let folder = Folder::new("queued-runs");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session());
    for (branch, hour) in [(1, 1), (1, 2), (2, 3), (2, 4)] {
        let prepared = columnar.prepare(&frame(branch, hour), row(), &OneThread).expect("prepared");
        columnar.commit(prepared, true, &OneThread);
    }
    for (branch, hour) in [(1, 1), (1, 2), (2, 3), (2, 4)] {
        columnar.complete(branch, hour);
    }
    columnar.finish();
    assert_eq!(columnar.status().unwrap()["stopped"], Value::Null);
    let recording = Recording::open(fs::read(folder.0.join(FINISHED)).unwrap(), Limits::default()).unwrap();
    assert_eq!(recording.hours(), 4);
    let runs: Vec<(u64, u8, i32)> = (0..2)
        .map(|run| {
            let run = recording.run(run).unwrap();
            (run.branch, run.epoch, run.first_hour)
        })
        .collect();
    assert_eq!(runs, [(1, envelope::epoch::START, 1), (2, envelope::epoch::BRANCH, 3)]);
    assert_eq!(recording.capture().unwrap().unwrap().run_sessions, [0, 0]);
}
