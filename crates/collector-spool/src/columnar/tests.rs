use super::*;
use replay::campaign::{
    Replay,
    live::{Admit, Asked, Host, LiveWriter, Progress},
};
use replay_settings::compression::Settings;
use replay_recording::columnar_frames::{Recording, Segment};
use replay_journal::columnar_journal::{Journal, Record as Journaled, Source};
use replay_journal::native_frame::{OneThread, RowFrame};
use std::fs;

fn journal_thread() -> Arc<JournalThread> {
    JournalThread::start().unwrap()
}

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

struct Every;

impl Host for Every {
    fn admit(&mut self, _: &Asked) -> Admit {
        Admit::Write
    }

    fn saves(&mut self) -> bool {
        true
    }
}

struct Recorded {
    controls: Vec<Vec<u8>>,
    replay: Replay,
}

impl std::ops::Deref for Recorded {
    type Target = Recording;

    fn deref(&self) -> &Recording {
        self.replay.recording()
    }
}

fn journaled_controls(folder: &Path, capture: u32) -> Vec<Vec<u8>> {
    let mut journal = Journal::open(folder, capture);
    journal.start(0).unwrap();
    let mut controls = Vec::new();
    while let Some(record) = journal.record().unwrap() {
        if let Journaled::Controls(given) = record {
            controls.extend(given.into_iter().map(<[u8]>::to_vec));
        }
        journal.advance();
    }
    controls
}

fn recorded_capture(folder: &Path, capture: u32) -> Recorded {
    let controls = journaled_controls(folder, capture);
    let path = folder.join(format!("recorded.{capture}.battleplan"));
    let state = folder.join(format!("recorded.{capture}.state"));
    let mut writer = LiveWriter::open_with(
        &path,
        &state,
        "session",
        capture,
        Settings::LOW,
        Vec::new(),
        recorder_derive::writers(),
    )
    .unwrap();
    let mut journal = Journal::open(folder, capture);
    journal.start(writer.segments()).unwrap();
    assert_eq!(
        writer.advance(&mut journal, &mut Every, &|| false).unwrap(),
        Progress::Finished
    );
    drop(writer);
    Recorded {
        controls,
        replay: Replay::open_path(&path).unwrap(),
    }
}

fn recorded(folder: &Path) -> Recorded {
    recorded_capture(folder, 0)
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("hoi4-columnar-{name}-{}", std::process::id()));
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

fn start(columnar: &mut Columnar, branch: u64, first_hour: i32) {
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    let reason = match branch {
        1 => BranchReason::SessionStart,
        _ => BranchReason::LoadSignal,
    };
    columnar.note(&[
        Record::SessionStart(session()),
        Record::BranchStart(BranchStart {
            id: branch,
            session: 1,
            loaded_hour: Some(first_hour - 1),
            first_hour,
            parent: Parentage::Unknown,
            reason,
        }),
    ]);
}

fn write(columnar: &mut Columnar, branch: u64, hours: std::ops::RangeInclusive<i32>) {
    for hour in hours {
        let prepared = columnar
            .prepare(&frame(branch, hour), row(), &OneThread)
            .unwrap()
            .expect("prepared");
        columnar.commit(prepared, false, &OneThread).unwrap();
    }
}

#[test]
fn hours_across_a_gap_and_a_branch_change_finish_as_runs_of_one_file() {
    let root = Folder::new("runs");
    let folder = Folder(root.0.join("campaign"));
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let first = envelope::UNIT_ORDINALS as i32 + 3;
    start(&mut columnar, 1, 1);
    write(&mut columnar, 1, 1..=first);
    write(&mut columnar, 1, first + 5..=first + 6);
    start(&mut columnar, 2, first + 7);
    write(&mut columnar, 2, first + 7..=first + 7);
    columnar.finish();
    assert_eq!(columnar.status().unwrap()["stopped"], Value::Null);
    let recording = recorded(&folder.0);
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), first as u64 + 3);
    assert_eq!(recording.replay.campaign_id, b"campaign");
    assert_eq!(recording.replay.sessions, [session()]);
    assert!(recording.replay.stops.is_empty());
    let branch = |run: u32| recording.run(run).unwrap().branch;
    assert_eq!(
        recording.segments(),
        [
            Segment {
                branch: branch(0),
                run: 0,
                first: 0,
                last: first as u32 - 1
            },
            Segment {
                branch: branch(1),
                run: 1,
                first: 0,
                last: 1
            },
            Segment {
                branch: branch(2),
                run: 2,
                first: 0,
                last: 0
            },
        ]
    );
    let epochs: Vec<u8> = (0..3)
        .map(|run| recording.run(run).unwrap().epoch)
        .collect();
    assert_eq!(
        epochs,
        [
            envelope::epoch::START,
            envelope::epoch::GAP,
            envelope::epoch::BRANCH
        ]
    );
    let mut frames =
        replay_recording::columnar_frames::Frames::new(&*recording, &[TableId::Countries]).unwrap();
    let mut read = 0;
    for run in 0..recording.run_count() {
        for ordinal in 0..recording.run(run).unwrap().ordinals {
            frames.go_to(run, ordinal).unwrap();
            frames.frame(branch(run)).unwrap();
            read += 1;
        }
    }
    assert_eq!(read, first as usize + 3);
}

#[test]
fn a_failed_hour_starts_a_run_and_the_file_goes_on() {
    let folder = Folder::new("interrupt");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    start(&mut columnar, 1, 1);
    write(&mut columnar, 1, 1..=2);
    columnar.interrupt();
    write(&mut columnar, 1, 3..=3);
    columnar.finish();
    let recording = recorded(&folder.0);
    assert_eq!(recording.segments().len(), 2);
    assert_eq!(
        recording.run(1).unwrap().epoch,
        envelope::epoch::WRITE_FAILURE
    );

    let failed = Folder::new("failed");
    let mut columnar = Columnar::open(&failed.0, "campaign", &session(), &journal_thread());
    start(&mut columnar, 1, 1);
    write(&mut columnar, 1, 1..=2);
    let _uncommitted = columnar
        .prepare(&frame(1, 3), row(), &OneThread)
        .unwrap()
        .expect("prepared");
    assert!(columnar.prepare(&frame(1, 3), row(), &OneThread).is_err());
    let status = columnar.status().unwrap();
    assert_eq!(
        (&status["stopped"], &status["failed_hours"]),
        (&Value::Null, &json!(1))
    );
    write(&mut columnar, 1, 4..=4);
    columnar.finish();
    let recording = recorded(&failed.0);
    assert_eq!(recording.hours(), 3);
    assert_eq!(recording.run(1).unwrap().first_hour, 4);
    assert_eq!(
        recording.run(1).unwrap().epoch,
        envelope::epoch::WRITE_FAILURE
    );
    assert!(recording.replay.stops.is_empty());
}

#[test]
fn a_failed_file_stops_the_recording_and_fails_every_later_hour() {
    let folder = Folder::new("blocked");
    let blocker = folder.0.join("blocker");
    fs::write(&blocker, b"").unwrap();
    let mut columnar = Columnar::open(
        &blocker.join("campaign"),
        "campaign",
        &session(),
        &journal_thread(),
    );
    let prepared = columnar
        .prepare(&frame(1, 1), row(), &OneThread)
        .unwrap()
        .expect("prepared");
    assert!(columnar.commit(prepared, false, &OneThread).is_ok());
    columnar.settle();
    assert!(columnar.prepare(&frame(1, 2), row(), &OneThread).is_err());
    let status = columnar.status().unwrap();
    assert!(status["stopped"].is_string());
    assert_eq!(status["lost_hours"], 1);
    columnar.finish();
    assert_eq!(fs::read_dir(&folder.0).unwrap().count(), 1);
}

#[test]
fn queued_hours_wait_for_their_rows_and_a_failure_drops_them() {
    let folder = Folder::new("queued");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    start(&mut columnar, 1, 1);
    for hour in 1..=4 {
        let prepared = columnar
            .prepare(&frame(1, hour), row(), &OneThread)
            .unwrap()
            .expect("prepared");
        columnar.commit(prepared, true, &OneThread).unwrap();
    }
    assert_eq!(columnar.status().unwrap()["pending"], 4);
    columnar.complete(1, 2);
    assert_eq!(columnar.status().unwrap()["pending"], 2);
    columnar.interrupt();
    for hour in 5..=6 {
        let prepared = columnar
            .prepare(&frame(1, hour), row(), &OneThread)
            .unwrap()
            .expect("prepared");
        columnar.commit(prepared, true, &OneThread).unwrap();
        columnar.complete(1, hour);
    }
    columnar.finish();
    let recording = recorded(&folder.0);
    assert_eq!(recording.hours(), 4);
    assert_eq!(recording.run(0).unwrap().ordinals, 2);
    assert_eq!(recording.run(1).unwrap().first_hour, 5);
    assert_eq!(
        recording.run(1).unwrap().epoch,
        envelope::epoch::WRITE_FAILURE
    );
}

#[test]
fn the_capture_keeps_a_context_given_again_once_and_every_gap_summary() {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    let folder = Folder::new("controls");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let start = Record::BranchStart(BranchStart {
        id: 1,
        session: 1,
        loaded_hour: Some(0),
        first_hour: 1,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    });
    let summary = Record::Control(ControlRecord::GapSummary {
        session: 1,
        dropped_frames: 3,
        reason: "synthetic loss".into(),
    });
    let clock = Record::Control(ControlRecord::ClockWitness {
        session: 1,
        qpc: 1,
        hour: 1,
        load_signal: None,
    });
    let context = [Record::SessionStart(session()), start.clone(), clock];
    for _ in 0..2 {
        columnar.note(&context);
        columnar.note(std::slice::from_ref(&summary));
    }
    write(&mut columnar, 1, 1..=2);
    columnar.finish();
    let recording = recorded(&folder.0);
    let encoded = |record: &Record| replay_capture_spool::spool::encode_record(record).unwrap();
    assert_eq!(
        recording.controls,
        [encoded(&start), encoded(&summary), encoded(&summary)]
    );
}

#[test]
fn an_hour_s_journal_record_carries_the_clock_witness_given_with_it() {
    use replay_journal::columnar_journal::{self, Witness};
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    let folder = Folder::new("witness");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let mut hour = |hour: i32, witnessed: i32, load_signal: Option<i64>| {
        let prepared = columnar
            .prepare(&frame(1, hour), row(), &OneThread)
            .unwrap()
            .expect("prepared");
        columnar.stage(&[Record::Control(ControlRecord::ClockWitness {
            session: 1,
            qpc: 100 + witnessed as u64,
            hour: witnessed,
            load_signal,
        })]);
        columnar.commit(prepared, true, &OneThread).unwrap();
        columnar.queued(1, hour);
        columnar.complete(1, hour);
    };
    hour(1, 1, Some(-7));
    hour(2, 2, None);
    hour(3, 9, Some(1));
    columnar.finish();
    let mut witnesses = Vec::new();
    for (_, _, path) in columnar_journal::segments(&folder.0).unwrap() {
        for record in columnar_journal::Segment::read(&path).unwrap().records(0) {
            if let columnar_journal::Record::Hour { witness, .. } = record.unwrap().0 {
                witnesses.push(witness);
            }
        }
    }
    assert_eq!(
        witnesses,
        [
            Some(Witness {
                qpc: 101,
                load_signal: Some(-7)
            }),
            Some(Witness {
                qpc: 102,
                load_signal: None
            }),
            None,
        ]
    );
}

#[test]
fn records_no_hour_followed_reach_a_journal_when_the_recording_finishes() {
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::{BranchReason, BranchStart, Gap, Parentage};
    let folder = Folder::new("records-alone");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let start = Record::BranchStart(BranchStart {
        id: 1,
        session: 1,
        loaded_hour: Some(0),
        first_hour: 1,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    });
    let gap = Record::Gap(Gap {
        branch: 1,
        first_hour: 1,
        last_hour: 3,
        reason: "synthetic loss".into(),
    });
    columnar.note(&[Record::SessionStart(session()), start.clone(), gap.clone()]);
    assert!(
        replay_journal::columnar_journal::segments(&folder.0)
            .unwrap()
            .is_empty()
    );
    columnar.finish();
    let recording = recorded(&folder.0);
    let encoded = |record: &Record| replay_capture_spool::spool::encode_record(record).unwrap();
    assert_eq!(recording.controls, [encoded(&start), encoded(&gap)]);
    assert_eq!(recording.run_count(), 0);
    let empty = Folder::new("nothing-given");
    let mut columnar = Columnar::open(&empty.0, "campaign", &session(), &journal_thread());
    columnar.finish();
    assert!(
        replay_journal::columnar_journal::segments(&empty.0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_queued_frame_s_records_wait_for_its_write_and_a_failed_one_s_are_dropped() {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    let folder = Folder::new("queued-controls");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let start = Record::BranchStart(BranchStart {
        id: 1,
        session: 1,
        loaded_hour: Some(0),
        first_hour: 1,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    });
    let summary = Record::Control(ControlRecord::GapSummary {
        session: 1,
        dropped_frames: 3,
        reason: "synthetic loss".into(),
    });
    let detach = Record::Control(ControlRecord::Detach {
        session: 1,
        last_hour: Some(4),
    });
    let context = [start.clone(), summary.clone()];
    columnar.stage(&context);
    columnar.queued(1, 1);
    columnar.interrupt();
    columnar.stage(&context);
    columnar.queued(1, 2);
    columnar.note(std::slice::from_ref(&detach));
    columnar.complete(1, 2);
    write(&mut columnar, 1, 3..=4);
    columnar.finish();
    let recording = recorded(&folder.0);
    let encoded = |record: &Record| replay_capture_spool::spool::encode_record(record).unwrap();
    assert_eq!(
        recording.controls,
        [encoded(&start), encoded(&summary), encoded(&detach)]
    );
}

#[test]
fn each_capture_takes_the_next_number_and_keeps_the_earlier_files() {
    let folder = Folder::new("captures");
    for hours in [1..=2, 3..=5] {
        let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
        start(&mut columnar, 1, 1);
        write(&mut columnar, 1, hours);
        columnar.finish();
    }
    for (capture, hours) in [(0, 2), (1, 3)] {
        assert_eq!(
            recorded_capture(&folder.0, capture).hours(),
            hours,
            "{capture}"
        );
    }
}

#[test]
fn an_unopened_recording_fails_every_hour() {
    let mut columnar = Columnar::Unopened;
    assert!(columnar.prepare(&frame(1, 1), row(), &OneThread).is_err());
    assert!(columnar.status().unwrap()["stopped"].is_string());
}

#[test]
fn a_run_starts_in_the_file_after_the_hours_queued_before_it() {
    let folder = Folder::new("queued-runs");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    start(&mut columnar, 1, 1);
    start(&mut columnar, 2, 3);
    for (branch, hour) in [(1, 1), (1, 2), (2, 3), (2, 4)] {
        let prepared = columnar
            .prepare(&frame(branch, hour), row(), &OneThread)
            .unwrap()
            .expect("prepared");
        columnar.commit(prepared, true, &OneThread).unwrap();
    }
    for (branch, hour) in [(1, 1), (1, 2), (2, 3), (2, 4)] {
        columnar.complete(branch, hour);
    }
    columnar.finish();
    assert_eq!(columnar.status().unwrap()["stopped"], Value::Null);
    let recording = recorded(&folder.0);
    assert_eq!(recording.hours(), 4);
    let runs: Vec<(u64, u8, i32)> = (0..2)
        .map(|run| {
            let run = recording.run(run).unwrap();
            (run.branch, run.epoch, run.first_hour)
        })
        .collect();
    assert_eq!(
        runs,
        [
            (1, envelope::epoch::START, 1),
            (2, envelope::epoch::BRANCH, 3)
        ]
    );
}

#[test]
fn commands_without_an_hour_reach_the_journal_at_once() {
    use replay_recording::{command_packet::{Batch, Packet}, control::ControlRecord};
    use replay_capture_spool::spool::{Record, encode_record};
    let folder = Folder::new("commands");
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &journal_thread());
    let packet = Packet::pack(4, 1, 1, 0, 10, 20, 0, &Batch::default()).unwrap();
    let record = Record::Control(ControlRecord::PlayerCommands { session: 1, packet: Box::new(packet) });
    columnar.note(std::slice::from_ref(&record));
    columnar.settle();
    assert_eq!(
        journaled_controls(&folder.0, 0),
        [encode_record(&record).unwrap()]
    );
    drop(columnar);
}

fn journaled_hours(folder: &Path, capture: u32) -> Vec<i32> {
    let mut journal = Journal::open(folder, capture);
    journal.start(0).unwrap();
    let (mut hours, mut next) = (Vec::new(), None);
    while let Some(record) = journal.record().unwrap() {
        match record {
            Journaled::Run { first_hour, .. } => next = Some(first_hour),
            Journaled::Hour { .. } => {
                let hour = next.expect("an hour within its run");
                hours.push(hour);
                next = Some(hour + 1);
            }
            _ => {}
        }
        journal.advance();
    }
    hours
}

#[test]
fn a_held_journal_never_holds_the_spool_and_stays_a_whole_record_prefix() {
    let folder = Folder::new("held");
    let thread = journal_thread();
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &thread);
    start(&mut columnar, 1, 1);
    write(&mut columnar, 1, 1..=3);
    columnar.settle();
    let Columnar::Live(live) = &columnar else {
        panic!("a live recording")
    };
    let lane = Arc::clone(&live.lane);
    let last = envelope::UNIT_ORDINALS as i32 + 5;
    let (held, holding) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder = {
        let lane = Arc::clone(&lane);
        std::thread::spawn(move || {
            let guard = lane.hold_writer();
            held.send(()).unwrap();
            let in_time = released
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .is_ok();
            drop(guard);
            in_time
        })
    };
    holding.recv().unwrap();
    write(&mut columnar, 1, 4..=last);
    let queued = lane.counters()[0];
    let written = journaled_hours(&folder.0, 0);
    let _ = release.send(());
    assert!(
        holder.join().unwrap(),
        "the spool waited on the journal's write"
    );
    assert_eq!(written, [1, 2, 3]);
    assert_eq!(queued, (last - 3) as u64);
    columnar.settle();
    assert_eq!(
        journaled_hours(&folder.0, 0),
        (1..=last).collect::<Vec<_>>()
    );
    columnar.finish();
    assert_eq!(columnar.status().unwrap()["stopped"], Value::Null);
    let recording = recorded(&folder.0);
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), last as u64);
}

#[test]
fn a_stop_the_journal_queue_refused_still_ends_the_capture_and_the_next_one_records() {
    let folder = Folder::new("refused-end");
    let thread = journal_thread();
    let mut columnar = Columnar::open(&folder.0, "campaign", &session(), &thread);
    start(&mut columnar, 1, 1);
    write(&mut columnar, 1, 1..=3);
    columnar.settle();
    thread.refuse_queues(2);
    write(&mut columnar, 1, 4..=4);
    assert!(columnar.prepare(&frame(1, 5), row(), &OneThread).is_err());
    assert_eq!(columnar.status().unwrap()["lost_hours"], 1);
    let mut given = Vec::new();
    for hour in 6..=12 {
        if columnar.halted().is_none() {
            let prepared = columnar
                .prepare(&frame(1, hour), row(), &OneThread)
                .unwrap()
                .expect("prepared");
            columnar.commit(prepared, false, &OneThread).unwrap();
            given.push(hour);
        }
    }
    assert!(
        !given.is_empty(),
        "the capture after the failure takes hours"
    );
    columnar.settle();
    assert_eq!(
        columnar.status().unwrap()["lost_hours"],
        1,
        "no later hour is refused"
    );
    assert_eq!(journaled_hours(&folder.0, 0), [1, 2, 3]);
    assert_eq!(journaled_hours(&folder.0, 1), given);
    columnar.finish();
}
