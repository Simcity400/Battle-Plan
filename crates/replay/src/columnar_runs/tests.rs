use super::*;
use crate::{
    branches::{BranchReason, Parentage},
    columnar_frames::{
        Limits, fixtures,
        synthetic::{Input, Sample, Synthetic},
    },
    frame::SharedView,
    identity::{Identity, Roster, SampleBoundary, Verification},
    rows::{CountryRow, TableRows, WorldTensionRow, tables},
};
use replay_columnar::envelope::UNIT_ORDINALS;
use std::collections::BTreeSet;

/// Runs (branch, first hour, units): two of one branch one after the other, and another.
const RUNS: [(u64, i32, u32); 3] = [
    (1, 100, 3),
    (1, 100 + 3 * UNIT_ORDINALS as i32, 2),
    (2, 50_000, 2),
];
const TABLES: [TableId; 2] = [TableId::Countries, TableId::WorldTension];
/// The tables an index serving frames of [`TABLES`] holds: theirs and Campaign, which every
/// frame holds.
const INDEXED: [TableId; 3] = [TableId::Campaign, TableId::Countries, TableId::WorldTension];
const LAYOUT: Layout = Layout { block: 4, runs: 16 };

/// Countries that come and go in many blocks, read in part for a while every few days, and
/// world tension that is sometimes unavailable.
fn wide(runs: &[(u64, i32, u32)]) -> (Vec<u8>, Vec<fixtures::Seen>) {
    fixtures::written(
        b"wide synthetic",
        runs,
        1,
        &TABLES,
        |synthetic, _, ordinal| {
            let rows: Vec<CountryRow> = (1..=30_u32)
                .filter(|&id| (id + ordinal / 40) % 7 != 0)
                .map(|id| CountryRow {
                    id,
                    tag: format!("tag {id} {}", (ordinal / 90 + id) % 5).into_bytes(),
                    stability_raw: i64::from(id) * 1_000 + i64::from(ordinal / 3),
                    political_power_raw: (id % 3 != 0).then_some(i64::from(ordinal % 11)),
                    ..Default::default()
                })
                .collect();
            let partial = ordinal % 120 >= 100;
            let read: Vec<CountryRow> = rows
                .iter()
                .filter(|row| !partial || row.id % 2 == 0)
                .cloned()
                .collect();
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &read,
                        partial: partial.then_some(3),
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
            let block = ordinal / 200;
            let tension = if block % 2 == 0 {
                synthetic.table(
                    TableId::WorldTension,
                    Input::Snapshot {
                        rows: &[WorldTensionRow {
                            total_raw: i64::from(ordinal / 7),
                            source_count: 1,
                            country_aggregate_count: 2,
                        }],
                        partial: None,
                        read: ordinal % 5 == 0,
                        sample: Sample::FromReadBit,
                    },
                )
            } else {
                synthetic.table::<WorldTensionRow>(
                    TableId::WorldTension,
                    Input::Unavailable {
                        reason: &format!("refused {}", block % 3),
                        read: false,
                        sample: Sample::FromReadBit,
                    },
                )
            };
            tension.unwrap();
        },
    )
}

/// What a cursor's tables may hold: far more than any fixture's.
fn tables_memory() -> MemoryBudget {
    MemoryBudget::new(1 << 30)
}

/// `recording` as an index holds it alone, no session pending.
fn alone(recording: Recording) -> Whole {
    Whole {
        campaign: recording.campaign().to_vec(),
        recordings: vec![recording],
        pending: Vec::new(),
    }
}

/// The index of `tables` of `bytes`, a recording, walked on `threads` workers.
fn prepared(bytes: &[u8], tables: &[TableId], threads: usize) -> Vec<u8> {
    let recording = Recording::open(bytes.to_vec(), Limits::default()).unwrap();
    let (written, _) = prepare(
        &alone(recording),
        tables,
        LAYOUT,
        LEVEL,
        &Settings::new(threads),
        &AtomicBool::new(false),
        Vec::new(),
    )
    .unwrap();
    written
}

/// Every hour of `runs`, in their order, with the branch it is of.
fn hours(runs: &[(u64, i32, u32)]) -> Vec<(u64, i32)> {
    runs.iter()
        .flat_map(|&(branch, first, units)| {
            (0..(units * UNIT_ORDINALS) as i32).map(move |at| (branch, first + at))
        })
        .collect()
}

/// Segments walked on one worker and on several write the same index, byte for byte, which
/// indexes the recording it was prepared from, read from memory or from its file.
#[test]
fn segments_walked_in_parallel_write_the_index_one_worker_writes() {
    let (bytes, _) = fixtures::churned(&RUNS, 1);
    let serial = prepared(&bytes, &TableId::ALL, 1);
    assert_eq!(prepared(&bytes, &TableId::ALL, 4), serial);
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    assert!(recording.segments().len() > RUNS.len());
    let path = std::env::temp_dir().join(format!("columnar-runs-{}.h4runs", std::process::id()));
    std::fs::write(&path, &serial).unwrap();
    for file in [
        IndexFile::from_bytes(serial.clone()).unwrap(),
        IndexFile::open(&path).unwrap(),
    ] {
        let recordings = std::slice::from_ref(&recording);
        assert!(file.indexes(recordings, &[], &TableId::ALL));
        assert!(!file.indexes(recordings, &[], &TABLES));
        assert_eq!(file.campaign(), b"churned synthetic");
        assert_eq!(&IndexFile::summary_of(&path).unwrap(), file.summary());
        assert_eq!(file.len(), serial.len() as u64);
        assert_eq!(
            Some(file.runs().to_vec()),
            recorded_runs(&recording).unwrap()
        );
        assert_eq!(
            file.index
                .runs
                .iter()
                .map(|run| run.hours.len())
                .sum::<usize>(),
            hours(&RUNS).len()
        );
    }
    std::fs::remove_file(&path).unwrap();
}

/// Countries over `runs`, each country's stability `shift` past the hour's, in a recording whose
/// root names `capture`.
fn countries(runs: &[(u64, i32, u32)], shift: i64, capture: Option<&Capture>) -> Vec<u8> {
    let write = |synthetic: &mut Synthetic, _: u32, ordinal: u32| {
        let rows: Vec<CountryRow> = (1..=12_u32)
            .map(|id| CountryRow {
                id,
                tag: format!("tag {id}").into_bytes(),
                stability_raw: shift + i64::from(ordinal),
                ..Default::default()
            })
            .collect();
        synthetic
            .table(
                TableId::Countries,
                Input::Snapshot {
                    rows: &rows,
                    partial: None,
                    read: true,
                    sample: Sample::FromReadBit,
                },
            )
            .unwrap();
    };
    fixtures::written_captured(
        b"countries synthetic",
        capture,
        runs,
        1,
        &[TableId::Countries],
        write,
    )
    .0
}

/// A capture of one session, `session`, naming it for one run.
fn capture(session: &[u8]) -> Capture {
    Capture {
        campaign_id: b"countries synthetic".to_vec(),
        content: b"content".to_vec(),
        capture_mode: 0,
        sessions: vec![session.to_vec()],
        run_sessions: vec![0],
        stop: None,
    }
}

/// An index indexes the recording it was prepared from and no other of its campaign and run
/// shapes: not one holding other values, nor one of another capture or none.
#[test]
fn an_index_indexes_only_the_recording_it_was_prepared_from() {
    let runs = [(1, 100, 1)];
    let tables = [TableId::Campaign, TableId::Countries];
    let bytes = countries(&runs, 0, Some(&capture(b"one")));
    let file = IndexFile::from_bytes(prepared(&bytes, &tables, 2)).unwrap();
    let recording = |bytes: Vec<u8>| Recording::open(bytes, Limits::default()).unwrap();
    assert!(file.indexes(&[recording(bytes.clone())], &[], &tables));
    assert_eq!(file.summary().natives[0].capture, Some(capture(b"one")));
    assert_eq!(
        file.summary().natives[0].identity,
        recording(bytes).identity()
    );
    for other in [
        countries(&runs, 1, Some(&capture(b"one"))),
        countries(&runs, 0, Some(&capture(b"two"))),
        countries(&runs, 0, None),
    ] {
        let other = recording(other);
        assert_eq!(recorded_runs(&other).unwrap(), Some(file.runs().to_vec()));
        assert!(!file.indexes(&[other], &[], &tables));
    }
}

/// The start of session `id`.
fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: id as u32,
        process_start_time: id,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "test".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: format!("test-{id}"),
            collector_hash: [id as u8; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

/// Branch `id` of session `session`, from `hour`.
fn branch(id: u64, session: u64, hour: i32) -> BranchStart {
    BranchStart {
        id,
        session,
        loaded_hour: Some(hour - 1),
        first_hour: hour,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    }
}

/// The capture of `held`'s native recording of `runs` runs.
fn captured(held: &SessionStart, runs: usize) -> Capture {
    Capture {
        campaign_id: b"countries synthetic".to_vec(),
        content: encoded(&held.identity).unwrap(),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![encoded(held).unwrap()],
        run_sessions: vec![0; runs],
        stop: None,
    }
}

/// Session `session`'s detach, after `hour`.
fn detach(session: u64, hour: i32) -> ControlRecord {
    ControlRecord::Detach {
        session,
        last_hour: Some(hour),
    }
}

/// A replay of two sessions, 1 and 2, each of one branch of one unit's hours, both detached:
/// branch 1 from hour 100, and branch 2 from hour `second`.
struct TwoSessions {
    sessions: [SessionStart; 2],
    branches: [BranchStart; 2],
    index: [BlockIndex; 2],
    controls: Vec<ControlRecord>,
}

impl TwoSessions {
    fn new(second: i32) -> Self {
        let last = |first: i32| first + UNIT_ORDINALS as i32 - 1;
        Self {
            sessions: [session(1), session(2)],
            branches: [branch(1, 1, 100), branch(2, 2, second)],
            index: [block(1, 100, last(100)), block(2, second, last(second))],
            controls: vec![detach(1, last(100)), detach(2, last(second))],
        }
    }

    /// The same replay while its last session is still being recorded: it records no detach
    /// of session 2.
    fn live(second: i32) -> Self {
        let mut replay = Self::new(second);
        replay.controls.truncate(1);
        replay
    }

    fn held(&self) -> Sessions<'_> {
        Sessions {
            campaign: b"countries synthetic",
            capture_mode: CaptureMode::Full.byte(),
            sessions: &self.sessions,
            branches: &self.branches,
            index: &self.index,
            controls: &self.controls,
        }
    }

    /// Session `at`'s native recording of its branch, its values `shift` on.
    fn native(&self, at: usize, shift: i64) -> Recording {
        let first = self.branches[at].first_hour;
        let runs = [(self.branches[at].id, first, 1)];
        let bytes = countries(&runs, shift, Some(&captured(&self.sessions[at], 1)));
        Recording::open(bytes, Limits::default()).unwrap()
    }
}

fn named(recordings: Vec<Recording>) -> Vec<(String, Recording)> {
    recordings
        .into_iter()
        .enumerate()
        .map(|(at, recording)| (format!("native {at}"), recording))
        .collect()
}

/// A replay of two sessions, one right after the other or with hours between them no block
/// records, is indexed whole from its two sessions' native recordings, given in any order: the
/// index holds both in its sessions' order, serves every hour each holds, and no hour between
/// them; it indexes those recordings alone, in that order.
#[test]
fn two_sessions_native_recordings_are_indexed_whole_gaps_and_all() {
    let tables = [TableId::Campaign, TableId::Countries];
    for second in [100 + UNIT_ORDINALS as i32, 5_000] {
        let replay = TwoSessions::new(second);
        let natives = named(vec![replay.native(1, 0), replay.native(0, 0)]);
        let whole = whole_of(natives, replay.held()).unwrap().unwrap();
        assert!(whole.pending.is_empty());
        assert_eq!(
            whole
                .recordings
                .iter()
                .map(|recording| recording.run(0).unwrap().branch)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let (bytes, _) = prepare(
            &whole,
            &tables,
            LAYOUT,
            LEVEL,
            &Settings::new(2),
            &AtomicBool::new(false),
            Vec::new(),
        )
        .unwrap();
        let file = Arc::new(IndexFile::from_bytes(bytes).unwrap());
        assert!(file.summary().held_by(replay.held()));
        assert_eq!(file.summary().natives.len(), 2);
        assert!(file.indexes(&whole.recordings, &[], &tables));
        assert!(!file.indexes(&whole.recordings[..1], &[], &tables));
        let reversed = [replay.native(1, 0), replay.native(0, 0)];
        assert!(!file.indexes(&reversed, &[], &tables));
        let replaced = [replay.native(0, 0), replay.native(1, 1)];
        assert!(!file.indexes(&replaced, &[], &tables));
        let mut snapshots = Snapshots::new(
            Arc::clone(&file),
            &[TableId::Countries],
            FormatVersion::CURRENT,
            CaptureMode::Full,
            1,
            tables_memory(),
        )
        .unwrap();
        for (branch, hour, held) in [
            (1, 100, true),
            (1, 819, true),
            (2, second, true),
            (2, second + 700, true),
            (1, 820, false),
            (2, second - 1, false),
        ] {
            let frame = snapshots.frame(branch, hour, None).unwrap();
            assert_eq!(frame.is_some(), held, "branch {branch} hour {hour}");
            if let Some(frame) = frame {
                let TableView::Complete(rows) = &*frame.tables[TableId::Countries.index()] else {
                    panic!("countries are complete at branch {branch} hour {hour}");
                };
                let first = if branch == 1 { 100 } else { second };
                let rows = rows.rows::<tables::Countries>().unwrap();
                assert_eq!(rows[0].stability_raw, i64::from(hour - first));
            }
        }
    }
}

/// Native recordings that do not hold a replay's hours whole are refused, naming why: a
/// session with none, a session with two, a recording of no session of the replay, one naming
/// no capture, and one whose hours stop short of the replay's.
#[test]
fn native_recordings_not_holding_a_replay_whole_are_refused_by_name() {
    let replay = TwoSessions::new(100 + UNIT_ORDINALS as i32);
    let refused = |recordings: Vec<Recording>| -> String {
        let Err(reason) = whole_of(named(recordings), replay.held()).unwrap() else {
            panic!("native recordings not holding the replay whole were taken");
        };
        reason
    };
    let missing = refused(vec![replay.native(0, 0)]);
    assert!(missing.contains("session 2"), "{missing}");
    let twice = refused(vec![replay.native(0, 0), replay.native(0, 1)]);
    assert!(
        twice.contains("both native recordings of session 1"),
        "{twice}"
    );
    let stranger = {
        let runs = [(3, 9_000, 1)];
        let bytes = countries(&runs, 0, Some(&captured(&session(3), 1)));
        Recording::open(bytes, Limits::default()).unwrap()
    };
    let other = refused(vec![replay.native(0, 0), replay.native(1, 0), stranger]);
    assert!(
        other.contains("not the native recording of a session"),
        "{other}"
    );
    let bare = {
        let runs = [(1, 100, 1)];
        Recording::open(countries(&runs, 0, None), Limits::default()).unwrap()
    };
    let unnamed = refused(vec![bare, replay.native(1, 0)]);
    assert!(unnamed.contains("names no capture"), "{unnamed}");
    // Session 2's branch goes on for a unit its native recording stopped short of.
    let longer = TwoSessions {
        index: [
            block(1, 100, 100 + UNIT_ORDINALS as i32 - 1),
            block(2, 820, 820 + 2 * UNIT_ORDINALS as i32 - 1),
        ],
        ..TwoSessions::new(820)
    };
    let Err(short) = whole_of(
        named(vec![longer.native(0, 0), longer.native(1, 0)]),
        longer.held(),
    )
    .unwrap() else {
        panic!("a native recording stopped short was taken");
    };
    assert!(short.contains("branch 2"), "{short}");
    // An index of both no longer serves a replay recording more than they hold.
    let both = whole_of(
        named(vec![replay.native(0, 0), replay.native(1, 0)]),
        replay.held(),
    )
    .unwrap()
    .unwrap();
    let (bytes, _) = prepare(
        &both,
        &[TableId::Countries],
        LAYOUT,
        LEVEL,
        &Settings::new(1),
        &AtomicBool::new(false),
        Vec::new(),
    )
    .unwrap();
    let file = IndexFile::from_bytes(bytes).unwrap();
    assert!(file.summary().held_by(replay.held()));
    assert!(!file.summary().held_by(longer.held()));
}

/// A replay's last session, of which it records no detach, is still being recorded: its hours
/// are pending, not missing. The index holds the ended session's native recording, names the
/// open one pending, holds none of its hours, and serves the replay; a native recording of the
/// open session is left for when it ends. A capture that went on after a detach records its
/// reattachment and is still being recorded; its failure ends it.
#[test]
fn a_session_still_being_recorded_is_pending_not_missing() {
    let tables = [TableId::Campaign, TableId::Countries];
    let mut replay = TwoSessions::live(100 + UNIT_ORDINALS as i32);
    assert!(replay.held().ended(1) && !replay.held().ended(2));
    let open = encoded(&replay.sessions[1]).unwrap();
    for natives in [
        vec![replay.native(0, 0)],
        vec![replay.native(1, 0), replay.native(0, 0)],
    ] {
        let whole = whole_of(named(natives), replay.held()).unwrap().unwrap();
        assert_eq!(whole.recordings.len(), 1);
        assert_eq!(whole.pending, [open.clone()]);
    }
    let whole = whole_of(named(vec![replay.native(0, 0)]), replay.held())
        .unwrap()
        .unwrap();
    let (bytes, _) = prepare(
        &whole,
        &tables,
        LAYOUT,
        LEVEL,
        &Settings::new(2),
        &AtomicBool::new(false),
        Vec::new(),
    )
    .unwrap();
    let file = Arc::new(IndexFile::from_bytes(bytes).unwrap());
    assert_eq!(file.summary().pending, [open.clone()]);
    assert!(file.summary().held_by(replay.held()));
    assert_eq!(replay.held().branches_of(&file.summary().pending), [(2, 2)]);
    assert!(file.indexes(&whole.recordings, &whole.pending, &tables));
    assert!(!file.indexes(&whole.recordings, &[], &tables));
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &[TableId::Countries],
        FormatVersion::CURRENT,
        CaptureMode::Full,
        1,
        tables_memory(),
    )
    .unwrap();
    assert!(snapshots.frame(1, 300, None).unwrap().is_some());
    assert!(snapshots.frame(2, 900, None).unwrap().is_none());
    replay.controls.push(detach(2, 900));
    assert!(replay.held().ended(2));
    replay.controls.push(ControlRecord::EventLoss {
        session: 2,
        branch: 2,
        after_hour: 900,
        hour: 905,
    });
    assert!(!replay.held().ended(2));
    replay.controls.push(ControlRecord::Partial {
        session: 2,
        reason: "synthetic failure".into(),
        failed_hour: Some(950),
    });
    assert!(replay.held().ended(2));
}

/// Once the open session ends, the recording records its detach: until its native recording
/// lands beside it, that session's hours are missing and refused by name; once it lands, the
/// index of the session pending is stale, and the index prepared again holds both whole.
#[test]
fn a_pending_session_s_native_recording_landing_makes_the_index_stale_then_whole() {
    let tables = [TableId::Campaign, TableId::Countries];
    let second = 100 + UNIT_ORDINALS as i32;
    let mut replay = TwoSessions::live(second);
    let live = whole_of(named(vec![replay.native(0, 0)]), replay.held())
        .unwrap()
        .unwrap();
    let prepared = |whole: &Whole| {
        let (bytes, _) = prepare(
            whole,
            &tables,
            LAYOUT,
            LEVEL,
            &Settings::new(2),
            &AtomicBool::new(false),
            Vec::new(),
        )
        .unwrap();
        Arc::new(IndexFile::from_bytes(bytes).unwrap())
    };
    let earlier = prepared(&live);
    replay.controls = TwoSessions::new(second).controls;
    assert!(!earlier.summary().held_by(replay.held()));
    let Err(waiting) = whole_of(named(vec![replay.native(0, 0)]), replay.held()).unwrap() else {
        panic!("an ended session with no native recording was taken");
    };
    assert!(waiting.contains("session 2"), "{waiting}");
    let landed = whole_of(
        named(vec![replay.native(0, 0), replay.native(1, 0)]),
        replay.held(),
    )
    .unwrap()
    .unwrap();
    assert!(landed.pending.is_empty());
    assert!(!earlier.indexes(&landed.recordings, &landed.pending, &tables));
    let file = prepared(&landed);
    assert!(file.indexes(&landed.recordings, &landed.pending, &tables));
    assert!(file.summary().pending.is_empty());
    assert!(file.summary().held_by(replay.held()));
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &[TableId::Countries],
        FormatVersion::CURRENT,
        CaptureMode::Full,
        1,
        tables_memory(),
    )
    .unwrap();
    assert!(snapshots.frame(2, 900, None).unwrap().is_some());
}

/// The summary of the index of `whole`'s native recordings of `tables`.
fn summary_of(whole: &Whole, tables: &[TableId]) -> Summary {
    let (bytes, _) = prepare(
        whole,
        tables,
        LAYOUT,
        LEVEL,
        &Settings::new(1),
        &AtomicBool::new(false),
        Vec::new(),
    )
    .unwrap();
    IndexFile::from_bytes(bytes).unwrap().summary().clone()
}

/// An index holds a replay only while the sessions it holds pending are exactly those the
/// replay records no end of: not once one of them ended, nor once a session it holds whole
/// went on after its detach, nor with one pending more, one fewer or one of another replay.
#[test]
fn an_index_holds_a_replay_only_while_its_pending_sessions_are_the_open_ones() {
    let tables = [TableId::Campaign, TableId::Countries];
    let second = 100 + UNIT_ORDINALS as i32;
    let mut replay = TwoSessions::live(second);
    let live = summary_of(
        &whole_of(named(vec![replay.native(0, 0)]), replay.held())
            .unwrap()
            .unwrap(),
        &tables,
    );
    assert!(live.held_by(replay.held()));
    replay.controls = TwoSessions::new(second).controls;
    assert!(!live.held_by(replay.held()));
    let ended = summary_of(
        &whole_of(
            named(vec![replay.native(0, 0), replay.native(1, 0)]),
            replay.held(),
        )
        .unwrap()
        .unwrap(),
        &tables,
    );
    assert!(ended.held_by(replay.held()));
    replay.controls.push(ControlRecord::EventLoss {
        session: 2,
        branch: 2,
        after_hour: second + 10,
        hour: second + 20,
    });
    assert!(!ended.held_by(replay.held()));
    assert!(live.held_by(replay.held()));
    let mut stranger = live.clone();
    stranger.pending.push(encoded(&session(3)).unwrap());
    assert!(!stranger.held_by(replay.held()));
    let mut fewer = live.clone();
    fewer.pending.clear();
    assert!(!fewer.held_by(replay.held()));
    let mut twice = live.clone();
    twice.pending.push(twice.pending[0].clone());
    assert!(!twice.held_by(replay.held()));
}

/// A replay none of whose sessions ended has an index of no native recording, its sessions
/// pending in its sessions' order, however many of their native recordings lie beside it:
/// that index serves the replay, holding none of its hours. Once a session ended, the index
/// no longer serves it, and that session with no native recording is refused by name.
#[test]
fn a_replay_none_of_whose_sessions_ended_is_indexed_with_every_session_pending() {
    let tables = [TableId::Campaign, TableId::Countries];
    let mut replay = TwoSessions::live(100 + UNIT_ORDINALS as i32);
    replay.controls.clear();
    let pending: Vec<Vec<u8>> = replay
        .sessions
        .iter()
        .map(|session| encoded(session).unwrap())
        .collect();
    for natives in [Vec::new(), vec![replay.native(1, 0), replay.native(0, 0)]] {
        let whole = whole_of(named(natives), replay.held()).unwrap().unwrap();
        assert!(whole.recordings.is_empty());
        assert_eq!(whole.campaign, b"countries synthetic");
        assert_eq!(whole.pending, pending);
    }
    let whole = whole_of(Vec::new(), replay.held()).unwrap().unwrap();
    let (bytes, _) = prepare(
        &whole,
        &tables,
        LAYOUT,
        LEVEL,
        &Settings::new(1),
        &AtomicBool::new(false),
        Vec::new(),
    )
    .unwrap();
    let file = Arc::new(IndexFile::from_bytes(bytes).unwrap());
    assert!(file.summary().natives.is_empty() && file.runs().is_empty());
    assert_eq!(file.summary().pending, pending);
    assert!(file.summary().held_by(replay.held()));
    assert!(file.indexes(&[], &pending, &tables));
    assert!(!file.indexes(&[], &pending[..1], &tables));
    assert_eq!(
        replay.held().branches_of(&file.summary().pending),
        [(1, 1), (2, 2)]
    );
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &[TableId::Countries],
        FormatVersion::CURRENT,
        CaptureMode::Full,
        1,
        tables_memory(),
    )
    .unwrap();
    assert!(snapshots.frame(1, 300, None).unwrap().is_none());
    replay
        .controls
        .push(detach(1, 100 + UNIT_ORDINALS as i32 - 1));
    assert!(!file.summary().held_by(replay.held()));
    let Err(missing) = whole_of(Vec::new(), replay.held()).unwrap() else {
        panic!("an ended session with no native recording was taken");
    };
    assert!(missing.contains("session 1"), "{missing}");
    assert!(!missing.contains("session 2"), "{missing}");
}

/// A map whose tables need more than their memory may ever hold is refused by name, never
/// taken for the recording's gap, and holds nothing after; memory that holds it serves it.
#[test]
fn a_map_needing_more_than_its_memory_may_ever_hold_is_refused_by_name() {
    let tables = [TableId::Campaign, TableId::Countries];
    let bytes = countries(&[(1, 100, 1)], 0, Some(&capture(b"one")));
    let file = Arc::new(IndexFile::from_bytes(prepared(&bytes, &tables, 1)).unwrap());
    let snapshots = |memory: MemoryBudget| {
        Snapshots::new(
            Arc::clone(&file),
            &[TableId::Countries],
            FormatVersion::CURRENT,
            CaptureMode::Full,
            1,
            memory,
        )
        .unwrap()
    };
    let small = MemoryBudget::new(1 << 10);
    let error = snapshots(small.clone()).frame(1, 100, None).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(beyond_memory(&error), "{error}");
    assert_eq!(small.used(), 0);
    assert!(
        snapshots(tables_memory())
            .frame(1, 100, None)
            .unwrap()
            .is_some()
    );
}

/// A page is read only from the pages' region, of no more than a page's bytes, and packed only
/// as one frame of its raw length: each is checked before any of its claimed bytes are held.
#[test]
fn a_page_s_place_and_length_are_checked_before_its_bytes_are_held() {
    let (bytes, _) = wide(&RUNS[..1]);
    let file = IndexFile::from_bytes(prepared(&bytes, &INDEXED, 1)).unwrap();
    // A dictionary chunk, read as pages are: its strings pack well.
    let packed = file.index.runs[0]
        .chunks
        .iter()
        .map(|chunk| chunk.location)
        .find(|location| location.stored + 1 < location.raw)
        .unwrap();
    let mut into = Vec::new();
    Pages::read(&file, packed, &mut into).unwrap();
    assert_eq!(into.len(), packed.raw as usize);
    let end = file.len();
    for location in [
        Location {
            stored: u32::MAX,
            raw: u32::MAX,
            ..packed
        },
        Location {
            offset: 0,
            ..packed
        },
        Location {
            offset: end - u64::from(packed.stored),
            ..packed
        },
        Location {
            offset: u64::MAX - 4,
            ..packed
        },
        Location {
            raw: packed.raw * 1_000,
            ..packed
        },
        Location {
            raw: packed.raw - 1,
            ..packed
        },
        Location {
            stored: packed.stored - 1,
            ..packed
        },
        Location {
            raw: PAGE_BYTES + 1,
            ..packed
        },
    ] {
        let error = Pages::read(&file, location, &mut into).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{location:?}");
        assert!(into.capacity() < 1 << 20, "{location:?}");
    }
}

/// Moves and lookups read only the dictionary chunks holding the strings their rows name, each
/// once while its readers hold it: a numeric table's rows read none.
#[test]
fn moves_and_lookups_read_only_the_dictionary_chunks_their_rows_name() {
    let (bytes, _) = fixtures::written(
        b"many strings",
        &[(1, 100, 1)],
        1,
        &TABLES,
        |synthetic, _, ordinal| {
            let rows: Vec<CountryRow> = (1..=30_u32)
                .map(|id| CountryRow {
                    id,
                    tag: format!("tag {id} {ordinal}").into_bytes(),
                    ..Default::default()
                })
                .collect();
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &rows,
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
            synthetic
                .table(
                    TableId::WorldTension,
                    Input::Snapshot {
                        rows: &[WorldTensionRow {
                            total_raw: i64::from(ordinal),
                            source_count: 1,
                            country_aggregate_count: 2,
                        }],
                        partial: None,
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
        },
    );
    let file = Arc::new(IndexFile::from_bytes(prepared(&bytes, &INDEXED, 2)).unwrap());
    let chunks = file.index.runs[0].chunks.len();
    assert!(chunks >= 5, "{chunks} chunks");
    let lookup = |table: TableId, keys: &[u128]| {
        file.rows_at(
            1,
            400,
            table,
            keys,
            FormatVersion::CURRENT,
            CaptureMode::Full,
        )
        .unwrap()
        .unwrap()
    };
    let (tension, _) = lookup(TableId::WorldTension, &[0, 1]);
    assert_eq!(keys(&tension).len(), 1);
    assert_eq!(file.reads().1, 0, "a numeric table's rows name no string");
    for _ in 0..3 {
        let (view, _) = lookup(TableId::Countries, &[5]);
        let TableView::Complete(rows) = view else {
            panic!("countries are complete");
        };
        assert_eq!(
            rows.rows::<tables::Countries>().unwrap()[0].tag,
            b"tag 5 300"
        );
    }
    assert_eq!(file.reads().1, 1, "one string, in one chunk, read once");
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &TABLES,
        FormatVersion::CURRENT,
        CaptureMode::Full,
        2,
        tables_memory(),
    )
    .unwrap();
    snapshots.frame(1, 700, None).unwrap().unwrap();
    let read = file.reads().1;
    // The hour's 30 strings lie in at most two chunks, besides the one read above.
    assert!((2..=3).contains(&read), "{read} of {chunks} chunks read");
}

/// Playing forward, a move to the next hour applies only that hour's changes: it reads no
/// page again, only the pages whose first ordinal is that hour, and one move and one build
/// make each frame.
#[test]
fn playing_forward_reads_only_the_pages_the_next_hour_opens() {
    let (bytes, expected) = fixtures::churned(&RUNS[..1], 1);
    let file = Arc::new(IndexFile::from_bytes(prepared(&bytes, &INDEXED, 2)).unwrap());
    let run = &file.index.runs[0];
    let opened = |ordinal: u32| -> usize {
        run.strips
            .iter()
            .map(|strips| {
                let columns = strips.columns.iter().flatten().flatten();
                let rows = strips.rows.iter().flatten().map(|held| &held.page);
                columns
                    .chain(rows)
                    .filter(|page| page.first == ordinal)
                    .count()
            })
            .sum()
    };
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &TABLES,
        FormatVersion::CURRENT,
        CaptureMode::Full,
        1,
        tables_memory(),
    )
    .unwrap();
    let first = run.first_hour;
    snapshots.frame(1, first, None).unwrap().unwrap();
    let hours = 3 * UNIT_ORDINALS / 2;
    for ordinal in 1..hours {
        let frame = snapshots
            .frame(1, first + ordinal as i32, None)
            .unwrap()
            .unwrap();
        assert!(frame == expected[ordinal as usize].0, "ordinal {ordinal}");
        assert_eq!(snapshots.read().0, opened(ordinal), "ordinal {ordinal}");
    }
    assert_eq!(snapshots.counts(), (hours as usize, hours as usize));
}

/// A preparation asked to stop stops at its next hour, refusing the index as interrupted.
#[test]
fn a_preparation_asked_to_stop_writes_no_index() {
    let (bytes, _) = fixtures::churned(&RUNS[..1], 1);
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    let stopped = prepare(
        &alone(recording),
        &TABLES,
        LAYOUT,
        LEVEL,
        &Settings::new(2),
        &AtomicBool::new(true),
        Vec::new(),
    );
    assert_eq!(
        stopped.err().map(|error| error.kind()),
        Some(io::ErrorKind::Interrupted)
    );
}

/// Frames read from the index at every hour, in order and then at hours far and near in
/// either direction, moved to without building and built later, on one worker and with
/// views built on several, are the frames the writer saw there: statuses, reasons, truncated
/// views, sample hours, rows that leave and come back, and metrics. An hour no run holds is
/// a gap.
#[test]
fn frames_read_from_the_index_are_the_writer_s_at_every_hour() {
    for (bytes, expected, runs) in [
        {
            let (bytes, seen) = fixtures::churned(&RUNS, 1);
            (bytes, seen, &RUNS[..])
        },
        {
            let (bytes, seen) = wide(&RUNS[..2]);
            (bytes, seen, &RUNS[..2])
        },
    ] {
        let file = Arc::new(IndexFile::from_bytes(prepared(&bytes, &INDEXED, 3)).unwrap());
        let hours = hours(runs);
        assert_eq!(hours.len(), expected.len());
        let alone = || {};
        let parallel = Parallel {
            workers: 3,
            on_worker: &alone,
        };
        for (workers, parallel) in [(1, None), (2, Some(&parallel))] {
            let mut snapshots = Snapshots::new(
                Arc::clone(&file),
                &TABLES,
                FormatVersion::CURRENT,
                CaptureMode::Full,
                workers,
                tables_memory(),
            )
            .unwrap();
            let check = |at: usize, snapshots: &mut Snapshots| {
                let (branch, hour) = hours[at];
                let frame = snapshots.frame(branch, hour, parallel).unwrap().unwrap();
                let sampled: Vec<Option<i32>> =
                    frame.tables.iter().map(SharedView::sampled_at).collect();
                assert!(frame == expected[at].0, "branch {branch} hour {hour}");
                assert_eq!(sampled, expected[at].1, "branch {branch} hour {hour}");
            };
            for at in 0..hours.len() {
                check(at, &mut snapshots);
            }
            let mut seed = 9_u64;
            for step in 0..500 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let at = (seed >> 33) as usize % hours.len();
                if step % 4 == 0 {
                    // Moves taken without a frame are built into the next one.
                    let (branch, hour) = hours[(at + 1) % hours.len()];
                    assert!(snapshots.move_to(branch, hour, &alone, &|| false).unwrap());
                }
                check(at, &mut snapshots);
            }
            assert!(snapshots.frame(1, 99, parallel).unwrap().is_none());
            assert!(snapshots.frame(3, 100, parallel).unwrap().is_none());
        }
    }
}

/// The keys of `view`'s rows, when it holds rows.
fn keys(view: &TableView) -> BTreeSet<u128> {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.iter().map(|row| row.key()).collect()
        }
        _ => BTreeSet::new(),
    }
}

/// Rows read by key are the frame's rows at those keys, with its status, truncation and sample
/// hour; and a move names the rows it inserted and deleted since the move before.
#[test]
fn rows_by_key_and_changes_between_moves_are_the_frames() {
    let (bytes, expected) = wide(&RUNS[..2]);
    let file = Arc::new(IndexFile::from_bytes(prepared(&bytes, &INDEXED, 2)).unwrap());
    let hours = hours(&RUNS[..2]);
    let asked = [1_u128, 4, 7, 14, 22, 77];
    for (at, &(branch, hour)) in hours.iter().enumerate().step_by(13) {
        let frame = &expected[at].0;
        let (view, sampled) = file
            .rows_at(
                branch,
                hour,
                TableId::Countries,
                &asked,
                FormatVersion::CURRENT,
                CaptureMode::Full,
            )
            .unwrap()
            .unwrap();
        let held = &frame.tables[TableId::Countries.index()];
        let pick = |rows: &TableRows| {
            let wanted: Vec<_> = rows
                .rows::<tables::Countries>()
                .unwrap()
                .iter()
                .filter(|row| asked.contains(&row.key()))
                .cloned()
                .collect();
            TableRows::from_rows::<tables::Countries>(wanted)
        };
        let wanted = match &**held {
            TableView::Complete(rows) => TableView::Complete(pick(rows)),
            TableView::Truncated { rows, missing } => TableView::Truncated {
                rows: pick(rows),
                missing: *missing,
            },
            other => panic!("countries are read at every hour, not {other:?}"),
        };
        assert_eq!(view, wanted, "hour {hour}");
        assert_eq!(sampled, held.sampled_at());
        let (tension, _) = file
            .rows_at(
                branch,
                hour,
                TableId::WorldTension,
                &[0],
                FormatVersion::CURRENT,
                CaptureMode::Full,
            )
            .unwrap()
            .unwrap();
        match &*frame.tables[TableId::WorldTension.index()] {
            TableView::Unavailable { reason } => {
                assert_eq!(
                    tension,
                    TableView::Unavailable {
                        reason: reason.clone()
                    }
                )
            }
            held => assert_eq!(keys(&tension), keys(held)),
        }
    }
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        &TABLES,
        FormatVersion::CURRENT,
        CaptureMode::Full,
        1,
        tables_memory(),
    )
    .unwrap();
    let mut before: Option<BTreeSet<u128>> = None;
    for (at, &(branch, hour)) in hours.iter().enumerate().take(2 * UNIT_ORDINALS as usize) {
        assert!(snapshots.move_to(branch, hour, &|| {}, &|| false).unwrap());
        let changed = snapshots.changed(TableId::Countries).unwrap();
        let inserted: BTreeSet<u128> = changed.inserted.iter().copied().collect();
        let deleted: BTreeSet<u128> = changed.deleted.iter().copied().collect();
        // The rows held, read or not: a truncated view leaves the rows it did not read held.
        let now: BTreeSet<u128> = match &before {
            Some(before) => before
                .union(&inserted)
                .copied()
                .filter(|key| !deleted.contains(key))
                .collect(),
            None => inserted.clone(),
        };
        if let TableView::Complete(rows) = &*expected[at].0.tables[TableId::Countries.index()] {
            assert_eq!(
                now,
                rows.iter().map(|row| row.key()).collect(),
                "hour {hour}"
            );
        }
        before = Some(now);
    }
    assert!(snapshots.changed(TableId::Divisions).is_none());
}

fn block(branch: u64, first_hour: i32, last_hour: i32) -> BlockIndex {
    BlockIndex {
        branch,
        first_hour,
        last_hour,
        offset: 0,
        length: 0,
        raw_length: 0,
    }
}

/// Runs cover a replay when they hold exactly the hours its blocks record.
#[test]
fn runs_cover_a_replay_when_they_hold_exactly_the_hours_its_blocks_record() {
    let index = [block(1, 100, 819), block(1, 820, 1_539), block(2, 100, 400)];
    assert!(covers(&index, &[(1, 100, 1_440), (2, 100, 301)]));
    // A run ends at hours a block skips, and the next starts after them.
    assert!(covers(
        &index,
        &[(1, 100, 500), (1, 650, 890), (2, 100, 301)]
    ));
    // A run past the replay's last hour.
    assert!(!covers(&index, &[(1, 100, 1_441), (2, 100, 301)]));
    // A block's last hour no run holds.
    assert!(!covers(&index, &[(1, 100, 1_439), (2, 100, 301)]));
    // A branch without runs, and runs of a branch the replay does not record.
    assert!(!covers(&index, &[(1, 100, 1_440)]));
    assert!(!covers(
        &index,
        &[(1, 100, 1_440), (2, 100, 301), (3, 0, 10)]
    ));
    // A run across hours between two blocks, which no block records.
    let holed = [block(1, 100, 819), block(1, 900, 1_000)];
    assert!(!covers(&holed, &[(1, 100, 901)]));
    assert!(covers(&holed, &[(1, 100, 720), (1, 900, 101)]));
    // No runs hold no blocks, and no others.
    assert!(covers(&[], &[]));
    assert!(!covers(&index, &[]));
}
