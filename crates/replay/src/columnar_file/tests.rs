use super::*;
use crate::{
    columnar_blocks::BlockBuilder,
    columnar_frames::{Recording, Segment, Views, Walker, schema},
    format::FormatVersion,
    frame::{DecodedFrame, FrameInput, Metrics, TableInput, TableView},
    native_frame::FrameWriter,
    rows::{CountryRow, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_columnar::{manifest::Manifest, walk::State};
use std::io::Cursor;

const TABLES: [TableId; 1] = [TableId::Countries];

/// Fast compression, as the format is the same at every level.
const TEST: Settings = Settings {
    level: 1,
    ..Settings::HIGH
};

/// Hour `hour`'s countries: two whose stability moves every hour and whose tags change
/// twice, so restarts carry strings first used in earlier chunks and in the latest one.
fn frame(hour: i32) -> FrameInput {
    let tag = |id: u32| match hour {
        ..=400 => format!("A{id}"),
        401..=1000 => format!("B{id}"),
        _ => format!("C{id}"),
    };
    let rows = (1..=2)
        .map(|id| CountryRow {
            id,
            canonical_slot: 1_000 + id,
            tag: tag(id).into_bytes(),
            stability_raw: i64::from(hour % 7) * 1_000 + i64::from(id),
            ..Default::default()
        })
        .collect();
    let mut tables: [TableInput; TABLE_COUNT] = std::array::from_fn(|index| {
        TableInput::Sampled(
            if scope::records(CaptureMode::Full, TableId::ALL[index]) {
                TableView::Unavailable {
                    reason: "not in this test".into(),
                }
            } else {
                TableView::NotRecorded {
                    mode: CaptureMode::Full,
                }
            },
        )
    });
    tables[TableId::Countries.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::Countries(rows)));
    let mut table_ns = [0; TABLE_COUNT];
    table_ns[TableId::Countries.index()] = 100 + hour as u64;
    FrameInput {
        branch: 1,
        hour,
        tables,
        metrics: Metrics {
            total_ns: 1_000 + hour as u64,
            table_ns,
            pool_bytes: 7,
            dropped_frames: 0,
        },
    }
}

fn identity() -> envelope::Identity {
    envelope::Identity {
        schema: envelope::schema_digest(&schema().unwrap()),
        campaign: b"columnar file test".to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
    }
}

fn file() -> FileWriter<Cursor<Vec<u8>>> {
    FileWriter::create(Cursor::new(Vec::new()), identity(), TABLE_COUNT, TEST).unwrap()
}

/// The views and sample hours the restored or walked frame gives match the written hour's.
fn same(written: &DecodedFrame, read: &DecodedFrame) {
    for table in TABLES {
        let index = table.index();
        assert_eq!(*read.tables[index], *written.tables[index], "{table:?} at hour {}", written.hour);
        assert_eq!(
            read.tables[index].sampled_at(),
            written.tables[index].sampled_at(),
            "{table:?} sample at hour {}",
            written.hour
        );
    }
}

/// Writes runs of `frame` hours into `file` as the recorder does, each through a frame writer
/// and a block builder restarting every `restart_blocks` blocks, and records the frames a
/// walker applying each hour as written gives.
struct Runs {
    file: FileWriter<Cursor<Vec<u8>>>,
    writer: FrameWriter,
    builder: BlockBuilder,
    state: State,
    views: Views,
    frames: Vec<DecodedFrame>,
    /// Every block written.
    blocks: u32,
}

impl Runs {
    fn new(restart_blocks: u32) -> Self {
        Self {
            file: file(),
            writer: FrameWriter::new().unwrap(),
            builder: BlockBuilder::new().unwrap().with_restart_blocks(restart_blocks),
            state: State::new(schema().unwrap()).unwrap(),
            views: Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap(),
            frames: Vec::new(),
            blocks: 0,
        }
    }

    fn start(&mut self, branch: u64, epoch: u8, first_hour: i32) {
        self.end();
        self.file.start_run(branch, epoch, first_hour).unwrap();
        self.builder.start_run(branch, first_hour).unwrap();
        self.writer = FrameWriter::new().unwrap();
        self.state = State::new(schema().unwrap()).unwrap();
    }

    /// Hour `input`; whether it filled a block.
    fn hour(&mut self, input: &FrameInput, first: bool) -> bool {
        let prepared = self.writer.hour(input).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let manifest = Manifest::read(&prepared.manifest, self.writer.columns()).unwrap();
        self.state.apply(&manifest, |at| Ok(sections[at].1)).unwrap();
        self.frames.push(self.views.advance(&self.state, &manifest, first).unwrap());
        let joined: Vec<u8> = sections.iter().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
        let block = self.builder.hour(&prepared.manifest, &joined).unwrap();
        drop(sections);
        self.writer.commit(prepared).unwrap();
        let filled = block.is_some();
        if let Some(block) = block {
            self.file.block(block.block()).unwrap();
            self.blocks += 1;
        }
        filled
    }

    /// Ends the run under way with its last block, short or not.
    fn end(&mut self) {
        if let Some(block) = self.builder.end_run().unwrap() {
            self.file.block(block.block()).unwrap();
            self.blocks += 1;
        }
        self.file.finish_run().unwrap();
    }
}

/// `hours` hours of `frame` from hour 1, restarting every `restart_blocks` blocks: the file,
/// and the frames a walker applying each hour as written gives.
fn written_with(hours: i32, restart_blocks: u32, frame: impl Fn(i32) -> FrameInput) -> (Vec<u8>, Vec<DecodedFrame>) {
    let mut runs = Runs::new(restart_blocks);
    runs.start(1, envelope::epoch::START, 1);
    for hour in 1..=hours {
        runs.hour(&frame(hour), hour == 1);
    }
    runs.end();
    (runs.file.finish(|_| Ok(())).unwrap().file.into_inner(), runs.frames)
}

fn written(hours: i32, restart_blocks: u32) -> (Vec<u8>, Vec<DecodedFrame>) {
    written_with(hours, restart_blocks, frame)
}

/// Every hour walked from the file's start gives the frames it was written with.
fn walks(recording: &Recording, frames: &[DecodedFrame]) {
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut walked = 0;
    recording
        .walk(&mut state, |state, manifest, starts| {
            same(&frames[walked], &views.advance(state, manifest, starts)?);
            walked += 1;
            Ok(true)
        })
        .unwrap();
    assert_eq!(walked, frames.len());
}

/// Every hour walked from the file's start, and every segment begun from its restart, gives
/// the frames the hours were written with.
#[test]
fn a_run_reads_back_from_its_start_and_from_every_restart() {
    let hours = 2 * envelope::UNIT_ORDINALS as i32 + 5;
    let (bytes, frames) = written(hours, 1);
    let recording = Recording::open(bytes).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), hours as u64);
    let segments = recording.segments();
    let branch = recording.run(0).unwrap().branch;
    assert_eq!(
        segments,
        [
            Segment { branch, run: 0, restart: None, first: 0, last: 719 },
            Segment { branch, run: 0, restart: Some(0), first: 720, last: 1439 },
            Segment { branch, run: 0, restart: Some(1), first: 1440, last: 1444 },
        ]
    );
    walks(&recording, &frames);
    segments_read_back(&recording, &frames);
}

/// Every segment begun from its restart gives the frames the hours were written with.
fn segments_read_back(recording: &Recording, frames: &[DecodedFrame]) {
    for segment in recording.segments().iter().filter(|segment| segment.restart.is_some()) {
        let mut state = State::new(schema().unwrap()).unwrap();
        let mut views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        let before = &frames[segment.first as usize - 1];
        Walker::new(recording)
            .unwrap()
            .begin(segment, &mut state)
            .unwrap();
        same(before, &views.restored(&state, 1, before.hour).unwrap());
        let mut at = segment.first as usize;
        recording
            .walk_segment(segment, &mut state, |state, manifest, _| {
                same(&frames[at], &views.advance(state, manifest, false)?);
                at += 1;
                Ok(true)
            })
            .unwrap();
        assert_eq!(at, segment.last as usize + 1);
    }
}

/// Every page's units stand alone, listed chunk by chunk, and every hour reads back; restarts
/// past the first are changes from the one before, and every keyframe-th is stored whole.
#[test]
fn standalone_units_and_restart_deltas_read_back_from_every_segment() {
    let blocks = envelope::KEYFRAME_RESTARTS * 2 + 3;
    let hours = (blocks * envelope::UNIT_ORDINALS) as i32;
    let (bytes, frames) = written(hours, 2);
    let recording = Recording::open(bytes).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    walks(&recording, &frames);
    let mut restarts = 0;
    for at in 0..recording.run(0).unwrap().pages() {
        let page = recording.page(0, at).unwrap();
        for (unit, chunks) in page.units() {
            assert_eq!(chunks.len() as u64, envelope::chunk_count(unit.raw), "page {at}");
        }
        let units = recording.page_units(0, at).unwrap();
        assert_eq!(units.len(), 1 + page.tables.len(), "page {at}");
        if let Some(restart) = &page.restart {
            assert_eq!(restart.ordinal, restarts);
            let whole = restarts % envelope::KEYFRAME_RESTARTS == 0;
            assert!(restart.tables.iter().all(|table| table.base != whole), "restart {restarts}");
            restarts += 1;
        }
    }
    assert_eq!(restarts, blocks / 2);
    segments_read_back(&recording, &frames);
}

/// A block follows its run's last whole one: none before a run starts, and none after a short
/// block ends its run.
#[test]
fn blocks_follow_their_runs_last_whole_block() {
    let mut runs = Runs::new(1);
    runs.start(1, envelope::epoch::START, 1);
    for hour in 1..=3 {
        runs.hour(&frame(hour), hour == 1);
    }
    let short = runs.builder.end_run().unwrap().unwrap();
    runs.file.block(short.block()).unwrap();
    assert!(runs.file.block(short.block()).is_err(), "a block after a short one");
    let mut file = file();
    assert!(file.block(short.block()).is_err(), "a block before a run");
}

#[test]
fn chunks_hold_the_ids_from_their_start_and_empty_chunks_hold_none() {
    // Chunk 1 is empty: it starts where chunk 2 does.
    let starts = [0, 4, 4, 9];
    assert_eq!(chunks_holding(&starts, &[0, 3]).unwrap(), [0]);
    assert_eq!(chunks_holding(&starts, &[4, 8, 9, 20]).unwrap(), [2, 3]);
    assert_eq!(chunks_holding(&starts, &[]).unwrap(), Vec::<u32>::new());
}

/// Every run start reason reads back as written, each run from empty tables, and the root's
/// capture names a session for each.
#[test]
fn runs_keep_their_start_reasons_and_the_root_names_their_sessions() {
    let epochs = [
        envelope::epoch::START,
        envelope::epoch::BRANCH,
        envelope::epoch::GAP,
        envelope::epoch::CONTINUATION,
        envelope::epoch::WRITE_FAILURE,
    ];
    let mut runs = Runs::new(1);
    for (run, &epoch) in epochs.iter().enumerate() {
        let hour = 1 + 10 * run as i32;
        runs.start(1, epoch, hour);
        runs.hour(&frame(hour), true);
    }
    runs.end();
    let capture = Capture {
        campaign_id: b"columnar file test".to_vec(),
        content: vec![1, 2, 3],
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![b"first".to_vec(), b"second".to_vec()],
        run_sessions: vec![0, 0, 0, 1, 1],
        stop: Some(Stop {
            branch: 1,
            last_hour: Some(41),
            reason: "stopped in this test".into(),
        }),
        format: 7,
        journaled: 3,
        controls: vec![b"a branch start".to_vec(), Vec::new(), b"a detach".to_vec()],
    };
    let bytes = runs.file.finish_with_capture(&capture, |_| Ok(())).unwrap().file.into_inner();
    let recording = Recording::open(bytes).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    let read: Vec<u8> = (0..epochs.len() as u32)
        .map(|run| recording.run(run).unwrap().epoch)
        .collect();
    assert_eq!(read, epochs);
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut starts = 0;
    recording
        .walk(&mut state, |_, _, first| {
            starts += usize::from(first);
            Ok(true)
        })
        .unwrap();
    assert_eq!(starts, epochs.len());
    walks(&recording, &runs.frames);
    assert_eq!(recording.capture().unwrap(), Some(capture));
}

#[test]
fn a_capture_reads_back_and_refuses_a_run_without_its_session() {
    let capture = Capture {
        campaign_id: b"campaign".to_vec(),
        content: Vec::new(),
        capture_mode: 0,
        sessions: vec![b"only".to_vec()],
        run_sessions: vec![0, 0],
        stop: None,
        format: 1,
        journaled: 9,
        controls: Vec::new(),
    };
    assert_eq!(Capture::read(&capture.write().unwrap()).unwrap(), capture);
    let orphan = Capture {
        run_sessions: vec![0, 1],
        ..capture.clone()
    };
    assert!(Capture::read(&orphan.write().unwrap()).is_err());
    let mut runs = Runs::new(1);
    runs.start(1, envelope::epoch::START, 1);
    runs.hour(&frame(1), true);
    runs.end();
    assert!(runs.file.finish_with_capture(&capture, |_| Ok(())).is_err(), "two runs named for one");
}

/// Each revision publishes every block so far, the run under way's included, and the finished
/// file reads whole.
#[test]
fn revisions_publish_every_block_so_far_and_the_finished_file_reads_whole() {
    let unit = envelope::UNIT_ORDINALS as i32;
    let hours = 3 * unit + 35;
    let mut runs = Runs::new(2);
    runs.start(1, envelope::epoch::START, 1);
    let mut published = Vec::new();
    for hour in 1..=hours {
        if runs.hour(&frame(hour), hour == 1) {
            runs.file.revise(None, None, |_| Ok(())).unwrap();
            published.push((hour, runs.file.file_mut().get_ref().clone()));
        }
    }
    assert_eq!(published.len(), 3);
    for (hour, bytes) in published {
        let recording = Recording::open(bytes).unwrap();
        assert!(recording.damage().is_empty(), "hour {hour}: {:?}", recording.damage());
        assert_eq!(recording.hours(), hour as u64);
        walks(&recording, &runs.frames[..hour as usize]);
    }
    runs.end();
    let bytes = runs.file.finish(|_| Ok(())).unwrap().file.into_inner();
    let recording = Recording::open(bytes).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), hours as u64);
    walks(&recording, &runs.frames);
}

/// The chain through an ordinal is its block's page's: every block of its run to that one. A
/// recording whose hours differ only within a later block gives the same chain before it and
/// another from it on.
#[test]
fn the_chain_through_an_ordinal_covers_every_block_of_its_run_to_it() {
    let unit = envelope::UNIT_ORDINALS;
    let hours = 2 * unit as i32 + 10;
    let (bytes, _) = written(hours, 1);
    let recording = Recording::open(bytes).unwrap();
    let mut chain = [0; 32];
    for at in 0..3 {
        let page = recording.page(0, at).unwrap();
        chain = envelope::chain_block(&chain, &page.records, &page.tables);
        assert_eq!(page.chain, chain);
        for ordinal in [page.first, page.first + page.hours - 1] {
            assert_eq!(recording.chain_through(0, ordinal).unwrap(), chain, "{ordinal}");
        }
    }
    assert_eq!(recording.run(0).unwrap().chain, chain);
    assert!(recording.chain_through(0, 2 * unit + 10).is_err());
    // Hour 724 is ordinal 723, in the second block.
    let changed = |hour: i32| {
        let mut input = frame(hour);
        if hour == 724
            && let TableInput::Sampled(TableView::Complete(TableRows::Countries(rows))) =
                &mut input.tables[TableId::Countries.index()]
        {
            rows[0].stability_raw += 1;
        }
        input
    };
    let (other, _) = written_with(hours, 1, changed);
    let other = Recording::open(other).unwrap();
    for (ordinal, same) in [(unit - 1, true), (unit, false), (2 * unit + 9, false)] {
        assert_eq!(
            other.chain_through(0, ordinal).unwrap() == recording.chain_through(0, ordinal).unwrap(),
            same,
            "{ordinal}"
        );
    }
}

/// Capture and published names read back as written, and no other name is a capture's.
#[test]
fn capture_and_published_names_read_back_as_written() {
    let mut out = [0; CAPTURE_NAME_BYTES];
    assert_eq!(capture_name(0, false, &mut out), "columnar.0.h4col");
    assert_eq!(
        capture_name(u32::MAX, true, &mut out),
        "columnar.4294967295.h4col.partial"
    );
    for (n, partial) in [(0, false), (7, true), (u32::MAX, false)] {
        let name = capture_name(n, partial, &mut out).to_owned();
        assert_eq!(capture_of(OsStr::new(&name)), Some((n, partial)));
    }
    for name in [
        "columnar.h4col",
        "columnar.h4col.partial",
        "columnar.01.h4col",
        "columnar.+1.h4col",
        "columnar..h4col",
        "columnar.4294967296.h4col",
        "columnar.1.h4col.counts.json",
        "other.1.h4col",
    ] {
        assert_eq!(capture_of(OsStr::new(name)), None, "{name}");
    }
    let published = published_name(OsStr::new("session-1.5"), 12);
    assert_eq!(published, "session-1.5.12.h4col");
    assert_eq!(published_of(&published), Some(("session-1.5", 12)));
    for name in [".0.h4col", "session.h4col", "session.00.h4col", "session.0.h4col.partial"] {
        assert_eq!(published_of(OsStr::new(name)), None, "{name}");
    }
    assert_eq!(
        campaign_folder(Path::new("replays"), b"Ab-1"),
        Path::new("replays").join("61622d31")
    );
}

/// `runs`, each a branch, its first hour and its hours, written into one file with a restart
/// after every block, its capture `capture`.
fn written_runs(runs: &[(u64, i32, i32)], capture: &Capture) -> Vec<u8> {
    let mut written = Runs::new(1);
    for &(branch, first, hours) in runs {
        written.start(branch, envelope::epoch::LOAD, first);
        for hour in first..first + hours {
            let mut input = frame(hour);
            input.branch = branch;
            written.hour(&input, hour == first);
        }
    }
    written.end();
    written.file.finish_with_capture(capture, |_| Ok(())).unwrap().file.into_inner()
}

/// Run `run`'s page `at` as its file holds it: its records and every table's block, raw.
fn blocks_of(recording: &Recording, run: u32, at: usize) -> Vec<(u32, Vec<u8>)> {
    recording
        .page_units(run, at)
        .unwrap()
        .into_iter()
        .map(|(extent, raw)| (extent.stream, raw))
        .collect()
}

/// Keeping some branches rewrites their runs hour for hour, restarts and all, each chained from
/// its new place, and the capture names only their runs and sessions; keeping none writes
/// nothing.
#[test]
fn kept_branches_rewrite_their_runs_hour_for_hour_and_name_only_their_sessions() {
    let unit = envelope::UNIT_ORDINALS as i32;
    let runs = [(1, 1, 2 * unit + 5), (2, 5_000, unit + 3), (3, 9_000, 2 * unit)];
    use crate::identity::{IdentityCodec, SampleBoundary, SessionStart};
    use crate::spool::{ControlRecord, Record, encode_record};
    use replay_frame::branches::{BranchReason, BranchStart, Gap, Parentage};
    let [s1, s2, s3] = [1, 2, 3].map(|id| {
        let mut bytes = Vec::new();
        SessionStart {
            id,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: replay_identity::test_fixtures::identity(),
        }
        .encode(&mut bytes)
        .unwrap();
        bytes
    });
    let control = |record: Record| encode_record(&record).unwrap();
    let start = |id: u64| {
        control(Record::BranchStart(BranchStart {
            id,
            session: id,
            loaded_hour: None,
            first_hour: 1,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }))
    };
    let gap = control(Record::Gap(Gap {
        branch: 2,
        first_hour: 5_000,
        last_hour: 5_001,
        reason: "lost in this test".into(),
    }));
    let detach = control(Record::Control(ControlRecord::Detach {
        session: 1,
        last_hour: None,
    }));
    let capture = Capture {
        campaign_id: b"columnar file test".to_vec(),
        content: vec![1, 2, 3],
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![s1.clone(), s2.clone(), s3.clone()],
        run_sessions: vec![0, 1, 2],
        stop: Some(Stop {
            branch: 2,
            last_hour: Some(5_000 + unit + 2),
            reason: "stopped in this test".into(),
        }),
        format: FormatVersion::CURRENT.version(),
        journaled: 0,
        controls: vec![start(1), start(2), gap.clone(), start(3), detach.clone()],
    };
    let original = Recording::open(written_runs(&runs, &capture)).unwrap();
    assert!(original.damage().is_empty(), "{:?}", original.damage());
    let kept = keep_branches(&original, |branch| branch != 2, Cursor::new(Vec::new()), TEST, |_| Ok(()))
        .unwrap()
        .unwrap()
        .file
        .into_inner();
    let mut rewritten = Recording::open(kept).unwrap();
    assert!(rewritten.damage().is_empty(), "{:?}", rewritten.damage());
    assert_eq!(rewritten.run_count(), 2);
    assert_eq!(
        rewritten.capture().unwrap(),
        Some(Capture {
            sessions: vec![s1.clone(), s3],
            run_sessions: vec![0, 1],
            stop: None,
            controls: vec![start(1), start(3), detach.clone()],
            ..capture.clone()
        })
    );
    for (new, old) in [(0, 0), (1, 2)] {
        let (kept, held) = (rewritten.run(new).unwrap(), original.run(old).unwrap());
        assert_eq!(
            (kept.branch, kept.epoch, kept.first_hour, kept.ordinals, kept.pages()),
            (held.branch, held.epoch, held.first_hour, held.ordinals, held.pages())
        );
        for at in 0..kept.pages() {
            assert_eq!(blocks_of(&rewritten, new, at), blocks_of(&original, old, at));
            assert_eq!(
                rewritten.page(new, at).unwrap().restart.is_some(),
                original.page(old, at).unwrap().restart.is_some()
            );
        }
    }
    assert_eq!(rewritten.run(0).unwrap().chain, original.run(0).unwrap().chain);
    assert_ne!(rewritten.run(1).unwrap().chain, original.run(2).unwrap().chain);
    // Every walk, from each run's start and from every restart, checks every hour's chain.
    rewritten.verify(true);
    for segment in rewritten.segments().to_vec() {
        let mut state = State::new(schema().unwrap()).unwrap();
        let mut walker = Walker::new(&rewritten).unwrap();
        walker.begin(&segment, &mut state).unwrap();
        assert!(walker.walk(&segment, &mut state, |_, _, _| Ok(true)).unwrap());
    }
    // The detach still names session 1 even when none of its runs remain.
    let second = keep_branches(&original, |branch| branch == 2, Cursor::new(Vec::new()), TEST, |_| Ok(()))
        .unwrap()
        .unwrap()
        .file
        .into_inner();
    assert_eq!(
        Recording::open(second).unwrap().capture().unwrap(),
        Some(Capture {
            sessions: vec![s2, s1],
            run_sessions: vec![0],
            controls: vec![start(2), gap, detach],
            ..capture.clone()
        })
    );
    assert!(
        keep_branches(&original, |_| false, Cursor::new(Vec::new()), TEST, |_| Ok(()))
            .unwrap()
            .is_none()
    );
}

/// Each revision writes only the control records added since the last, and every reader
/// joins them back into the capture in order.
#[test]
fn revisions_append_only_new_control_records() {
    let noise = |seed: u64| {
        let mut state = seed;
        (0..64 * 1024)
            .map(|_| {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (state >> 56) as u8
            })
            .collect::<Vec<_>>()
    };
    let mut capture = Capture {
        campaign_id: b"columnar file test".to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![b"session".to_vec()],
        run_sessions: Vec::new(),
        stop: None,
        format: 1,
        journaled: 0,
        controls: vec![noise(1)],
    };
    let mut file = file();
    file.revise(Some(&capture), None, |_| Ok(())).unwrap();
    let first = file.file_mut().get_ref().len();
    capture.controls.push(b"later".to_vec());
    file.revise(Some(&capture), None, |_| Ok(())).unwrap();
    assert!(file.file_mut().get_ref().len() - first < 4 * 1024);
    let published = Recording::open(file.file_mut().get_ref().clone()).unwrap();
    assert_eq!(published.capture().unwrap(), Some(capture.clone()));
    capture.controls.push(b"last".to_vec());
    let bytes = file
        .finish_with_capture(&capture, |_| Ok(()))
        .unwrap()
        .file
        .into_inner();
    let recording = Recording::open(bytes).unwrap();
    assert_eq!(recording.capture().unwrap(), Some(capture));
}
