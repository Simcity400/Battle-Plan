use super::*;
use crate::{
    columnar_frames::{Limits, Recording, Segment, Views, Walker, schema},
    format::FormatVersion,
    frame::{DecodedFrame, FrameInput, Metrics, TableInput, TableView},
    native_frame::{FrameHour, FrameWriter},
    rows::{CountryRow, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_columnar::{manifest::Manifest, walk::State};
use std::{io::Cursor, sync::atomic::AtomicBool};

const TABLES: [TableId; 1] = [TableId::Countries];

/// A restart after every unit, so a short run crosses two.
const EVERY_UNIT: Settings = Settings {
    restart_units: 1,
    ..Settings::RECORDING
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

/// `hours` hours from hour 1 written through a frame writer into a file, with the frames a
/// walker applying each hour as written gives.
fn written(hours: i32, settings: Settings) -> (Vec<u8>, Vec<DecodedFrame>) {
    written_with(hours, settings, frame)
}

/// [`written`], hour `hour` being `frame(hour)`.
fn written_with(
    hours: i32,
    settings: Settings,
    frame: impl Fn(i32) -> FrameInput,
) -> (Vec<u8>, Vec<DecodedFrame>) {
    let mut writer = FrameWriter::new().unwrap();
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        writer.columns().to_vec(),
        settings,
    )
    .unwrap();
    file.start_run(1, envelope::epoch::START, 1).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut frames = Vec::new();
    for hour in 1..=hours {
        let prepared = writer.hour(&frame(hour)).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let manifest = Manifest::read(&prepared.manifest, writer.columns()).unwrap();
        state.apply(&manifest, |at| Ok(sections[at].1)).unwrap();
        frames.push(views.advance(&state, &manifest, hour == 1).unwrap());
        let position = file
            .hour(Hour {
                hour,
                manifest: &prepared.manifest,
                sections: &sections,
                statuses: prepared.statuses(),
                read: prepared.read(),
            })
            .unwrap();
        drop(sections);
        writer.commit(prepared).unwrap();
        if let Position::Full { restart } = position {
            if restart {
                let (strings, tables) = writer.restart().unwrap();
                file.close_with_restart(&strings, &tables).unwrap();
            } else {
                file.close_position().unwrap();
            }
        }
    }
    (file.finish(|_| Ok(())).unwrap().file.into_inner(), frames)
}

/// Every hour walked from the file's start, and every segment begun from its restart, gives
/// the frames the hours were written with.
#[test]
fn a_run_reads_back_from_its_start_and_from_every_restart() {
    let hours = 2 * envelope::UNIT_ORDINALS as i32 + 5;
    let (bytes, frames) = written(hours, EVERY_UNIT);
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), hours as u64);
    let segments = recording.segments();
    assert_eq!(
        segments,
        [
            Segment { run: 0, restart: None, first: 0, last: 719 },
            Segment { run: 0, restart: Some(0), first: 720, last: 1439 },
            Segment { run: 0, restart: Some(1), first: 1440, last: 1444 },
        ]
    );
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
    assert_eq!(walked, hours as usize);
    for segment in segments.iter().filter(|segment| segment.restart.is_some()) {
        let mut state = State::new(schema().unwrap()).unwrap();
        let mut views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        let before = &frames[segment.first as usize - 1];
        Walker::new(&recording, &AtomicBool::new(false))
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

/// Without a due restart a full position closes bare, and the next hour waits for it.
#[test]
fn a_full_position_closes_before_the_next_hour_and_restarts_only_when_due() {
    let (bytes, _) = written(envelope::UNIT_ORDINALS as i32 + 1, Settings::RECORDING);
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(
        recording.segments(),
        [Segment { run: 0, restart: None, first: 0, last: 720 }]
    );

    let mut writer = FrameWriter::new().unwrap();
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        writer.columns().to_vec(),
        Settings::RECORDING,
    )
    .unwrap();
    file.start_run(1, envelope::epoch::START, 1).unwrap();
    for hour in 1..=envelope::UNIT_ORDINALS as i32 + 1 {
        let prepared = writer.hour(&frame(hour)).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let result = file.hour(Hour {
            hour,
            manifest: &prepared.manifest,
            sections: &sections,
            statuses: prepared.statuses(),
            read: prepared.read(),
        });
        if hour as u32 <= envelope::UNIT_ORDINALS {
            let position = result.unwrap();
            assert_eq!(
                position == (Position::Full { restart: false }),
                hour as u32 == envelope::UNIT_ORDINALS
            );
        } else {
            assert!(result.is_err(), "an hour past a full position");
        }
        drop(sections);
        writer.commit(prepared).unwrap();
    }
    let (strings, tables) = writer.restart().unwrap();
    assert!(file.close_with_restart(&strings, &tables).is_err());
    file.close_position().unwrap();
}

#[test]
fn chunks_hold_the_ids_from_their_start_and_empty_chunks_hold_none() {
    // Chunk 1 is empty: it starts where chunk 2 does.
    let starts = [0, 4, 4, 9];
    assert_eq!(chunks_holding(&starts, &[0, 3]).unwrap(), [0]);
    assert_eq!(chunks_holding(&starts, &[4, 8, 9, 20]).unwrap(), [2, 3]);
    assert_eq!(chunks_holding(&starts, &[]).unwrap(), Vec::<u32>::new());
}

/// `prepared`'s hour with `sections` in place of its own.
fn given<'a>(prepared: &'a FrameHour, sections: &'a [(usize, &'a [u8])]) -> Hour<'a> {
    Hour {
        hour: prepared.hour(),
        manifest: &prepared.manifest,
        sections,
        statuses: prepared.statuses(),
        read: prepared.read(),
    }
}

/// Hours refused before anything is written leave the file whole: a first hour that is not
/// the run's, and sections out of table order or past the tables. A restart waits for the
/// last hour's commit.
#[test]
fn refused_hours_leave_the_file_whole_and_restarts_wait_for_the_commit() {
    let mut writer = FrameWriter::new().unwrap();
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        writer.columns().to_vec(),
        Settings::RECORDING,
    )
    .unwrap();
    file.start_run(1, envelope::epoch::START, 1).unwrap();
    for hour in 1..=3 {
        let prepared = writer.hour(&frame(hour)).unwrap();
        assert!(writer.restart().is_err(), "a restart before hour {hour} commits");
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let mut late = given(&prepared, &sections);
        late.hour = hour + 1;
        assert!(file.hour(late).is_err(), "hour {} in place of {hour}", hour + 1);
        let (table, section) = sections[0];
        let twice = [(table, section), (table, section)];
        assert!(file.hour(given(&prepared, &twice)).is_err());
        let past = [(TABLE_COUNT, section)];
        assert!(file.hour(given(&prepared, &past)).is_err());
        assert_eq!(file.hour(given(&prepared, &sections)).unwrap(), Position::Open);
        drop(sections);
        writer.commit(prepared).unwrap();
        writer.restart().unwrap();
    }
    let recording = Recording::open(file.finish(|_| Ok(())).unwrap().file.into_inner(), Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), 3);
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
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        schema().unwrap().iter().map(Vec::len).collect(),
        Settings::RECORDING,
    )
    .unwrap();
    for (run, &epoch) in epochs.iter().enumerate() {
        let hour = 1 + 10 * run as i32;
        let mut writer = FrameWriter::new().unwrap();
        file.start_run(1, epoch, hour).unwrap();
        let prepared = writer.hour(&frame(hour)).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        assert_eq!(file.hour(given(&prepared, &sections)).unwrap(), Position::Open);
        drop(sections);
        writer.commit(prepared).unwrap();
    }
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
        controls: vec![b"a branch start".to_vec(), Vec::new(), b"a detach".to_vec()],
    };
    let bytes = file.finish_with_capture(&capture, |_| Ok(())).unwrap().file.into_inner();
    let recording = Recording::open(bytes, Limits::default()).unwrap();
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
        controls: Vec::new(),
    };
    assert_eq!(Capture::read(&capture.write().unwrap()).unwrap(), capture);
    let orphan = Capture {
        run_sessions: vec![0, 1],
        ..capture.clone()
    };
    assert!(Capture::read(&orphan.write().unwrap()).is_err());
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        schema().unwrap().iter().map(Vec::len).collect(),
        Settings::RECORDING,
    )
    .unwrap();
    file.start_run(1, envelope::epoch::START, 1).unwrap();
    assert!(file.finish_with_capture(&capture, |_| Ok(())).is_err(), "two runs named for one");
}

/// The hours of a file revised every `every` hours, appended up to `hours`, with the frames
/// written and, after each revision, the file's bytes then.
fn revised(
    hours: i32,
    every: i32,
    settings: Settings,
    capture: Option<&Capture>,
) -> (FileWriter<Cursor<Vec<u8>>>, FrameWriter, Vec<DecodedFrame>, Vec<(i32, Vec<u8>)>) {
    let mut writer = FrameWriter::new().unwrap();
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        writer.columns().to_vec(),
        settings,
    )
    .unwrap();
    file.start_run(1, envelope::epoch::START, 1).unwrap();
    let mut state = State::new(schema().unwrap()).unwrap();
    let mut views = Views::new(&TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let (mut frames, mut published) = (Vec::new(), Vec::new());
    for hour in 1..=hours {
        let prepared = writer.hour(&frame(hour)).unwrap();
        let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
        let manifest = Manifest::read(&prepared.manifest, writer.columns()).unwrap();
        state.apply(&manifest, |at| Ok(sections[at].1)).unwrap();
        frames.push(views.advance(&state, &manifest, hour == 1).unwrap());
        let position = file
            .hour(Hour {
                hour,
                manifest: &prepared.manifest,
                sections: &sections,
                statuses: prepared.statuses(),
                read: prepared.read(),
            })
            .unwrap();
        drop(sections);
        writer.commit(prepared).unwrap();
        if hour % every == 0 {
            file.revise(capture, |_| Ok(())).unwrap();
            published.push((hour, file.file.get_ref().clone()));
        }
        if let Position::Full { restart } = position {
            if restart {
                let (strings, tables) = writer.restart().unwrap();
                file.close_with_restart(&strings, &tables).unwrap();
            } else {
                file.close_position().unwrap();
            }
        }
    }
    (file, writer, frames, published)
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

/// Each revision publishes every hour so far, open positions as chains of unit pieces, and the
/// finished file reads whole, its sealed positions holding the pieces in turn.
#[test]
fn revisions_publish_every_hour_so_far_and_the_finished_file_reads_whole() {
    let hours = 2 * envelope::UNIT_ORDINALS as i32 + 35;
    let (file, _, frames, published) = revised(hours, 100, EVERY_UNIT, None);
    assert_eq!(published.len(), 14);
    for (hour, bytes) in published {
        let recording = Recording::open(bytes, Limits::default()).unwrap();
        assert!(recording.damage().is_empty(), "hour {hour}: {:?}", recording.damage());
        assert_eq!(recording.hours(), hour as u64);
        walks(&recording, &frames[..hour as usize]);
    }
    let bytes = file.finish(|_| Ok(())).unwrap().file.into_inner();
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), hours as u64);
    walks(&recording, &frames);
}

/// A file whose writer stopped after a revision finishes from it: the hours appended since and
/// a torn tail are cut, and the capture records why the file stops.
#[test]
fn an_unfinished_file_finishes_from_its_last_revision() {
    let capture = Capture {
        campaign_id: b"columnar file test".to_vec(),
        content: Vec::new(),
        capture_mode: CaptureMode::Full.byte(),
        sessions: vec![b"session".to_vec()],
        run_sessions: vec![0],
        stop: None,
        format: 1,
        controls: Vec::new(),
    };
    let (file, _, frames, published) =
        revised(envelope::UNIT_ORDINALS as i32 + 50, 400, Settings::RECORDING, Some(&capture));
    let (hour, _) = *published.last().unwrap();
    assert_eq!(hour, 400);
    let mut bytes = file.file.into_inner();
    bytes.extend_from_slice(b"a torn tail");
    let unfinished = Unfinished::read(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(unfinished.last, Some((1, hour)));
    assert_eq!(unfinished.capture.as_ref(), Some(&capture));
    let stopped = Capture {
        stop: Some(Stop {
            branch: 1,
            last_hour: Some(hour),
            reason: "the collector ended before finishing this file".into(),
        }),
        ..capture
    };
    let mut cursor = Cursor::new(bytes);
    unfinished
        .finish(
            &mut cursor,
            |cursor, length| {
                cursor.get_mut().truncate(length as usize);
                Ok(())
            },
            |_| Ok(()),
            &stopped,
            Settings::RECORDING,
        )
        .unwrap();
    let recording = Recording::open(cursor.into_inner(), Limits::default()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), hour as u64);
    assert_eq!(recording.capture().unwrap(), Some(stopped));
    walks(&recording, &frames[..hour as usize]);
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

/// Each hour of run `run`'s page `at` as its file holds it: its ordinal, manifest and sections.
fn hours_of(recording: &Recording, run: u32, at: usize) -> Vec<(u32, Vec<u8>, Vec<Vec<u8>>)> {
    let mut hours = Vec::new();
    recording
        .page_hours(run, at, u32::MAX, |ordinal, manifest, _, sections| {
            hours.push((
                ordinal,
                manifest.to_vec(),
                sections.iter().map(|section| section.to_vec()).collect(),
            ));
            Ok(true)
        })
        .unwrap();
    hours
}

/// The chain through an hour covers every hour of its run to it: where its page ends, the
/// page's own chain; within its page, the page's hours to it as well. A recording whose hours
/// differ only within that page gives another chain through it.
#[test]
fn the_chain_through_an_hour_covers_every_hour_of_its_run_to_it() {
    let unit = envelope::UNIT_ORDINALS;
    let hours = unit as i32 + 10;
    let (bytes, _) = written(hours, EVERY_UNIT);
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    let mut chain = envelope::chain_start(&identity().chain_bytes().unwrap(), 0);
    let mut through = Vec::new();
    for at in 0..2 {
        recording
            .page_hours(0, at, u32::MAX, |_, manifest, _, sections| {
                chain = envelope::chain_hour(&chain, manifest, sections.iter().copied());
                through.push(chain);
                Ok(true)
            })
            .unwrap();
        assert_eq!(through.last(), Some(&recording.page(0, at).unwrap().chain));
    }
    assert_eq!(through.len(), hours as usize);
    for ordinal in [0, 3, unit - 1, unit, unit + 4, unit + 9] {
        assert_eq!(
            recording.chain_through(0, ordinal).unwrap(),
            through[ordinal as usize],
            "{ordinal}"
        );
    }
    assert!(recording.chain_through(0, unit + 10).is_err());
    // Hour 724 is ordinal 723, on the second page.
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
    let (other, _) = written_with(hours, EVERY_UNIT, changed);
    let other = Recording::open(other, Limits::default()).unwrap();
    for (ordinal, same) in [
        (unit - 1, true),
        (unit + 1, true),
        (unit + 3, false),
        (unit + 5, false),
    ] {
        assert_eq!(
            other.chain_through(0, ordinal).unwrap() == recording.chain_through(0, ordinal).unwrap(),
            same,
            "{ordinal}"
        );
    }
}

/// `runs`, each a branch, its first hour and its hours, written into one file with a restart
/// after every unit, its capture `capture`.
fn written_runs(runs: &[(u64, i32, i32)], capture: &Capture) -> Vec<u8> {
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        schema().unwrap().iter().map(Vec::len).collect(),
        EVERY_UNIT,
    )
    .unwrap();
    for &(branch, first, hours) in runs {
        let mut writer = FrameWriter::new().unwrap();
        file.start_run(branch, envelope::epoch::LOAD, first).unwrap();
        for hour in first..first + hours {
            let mut input = frame(hour);
            input.branch = branch;
            let prepared = writer.hour(&input).unwrap();
            let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
            let position = file.hour(given(&prepared, &sections)).unwrap();
            drop(sections);
            writer.commit(prepared).unwrap();
            if let Position::Full { restart } = position {
                if restart {
                    let (strings, tables) = writer.restart().unwrap();
                    file.close_with_restart(&strings, &tables).unwrap();
                } else {
                    file.close_position().unwrap();
                }
            }
        }
    }
    file.finish_with_capture(capture, |_| Ok(()))
        .unwrap()
        .file
        .into_inner()
}

/// Keeping some branches rewrites their runs hour for hour, restarts and all, each chained from
/// its new place, and the capture names only their runs and sessions; keeping none writes
/// nothing.
#[test]
fn kept_branches_rewrite_their_runs_hour_for_hour_and_name_only_their_sessions() {
    let unit = envelope::UNIT_ORDINALS as i32;
    let runs = [(1, 1, 2 * unit + 5), (2, 5_000, unit + 3), (3, 9_000, 2 * unit)];
    use crate::spool::{ControlRecord, Record, encode_record};
    use replay_frame::branches::{BranchReason, BranchStart, Gap, Parentage};
    let control = |record: Record| encode_record(&record).unwrap();
    let start = |id: u64| {
        control(Record::BranchStart(BranchStart {
            id,
            session: 1,
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
        sessions: vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
        run_sessions: vec![0, 1, 2],
        stop: Some(Stop {
            branch: 2,
            last_hour: Some(5_000 + unit + 2),
            reason: "stopped in this test".into(),
        }),
        format: FormatVersion::CURRENT.version(),
        controls: vec![start(1), start(2), gap.clone(), start(3), detach.clone()],
    };
    let original = Recording::open(written_runs(&runs, &capture), Limits::default()).unwrap();
    assert!(original.damage().is_empty(), "{:?}", original.damage());
    let kept = keep_branches(&original, |branch| branch != 2, Cursor::new(Vec::new()), EVERY_UNIT, |_| Ok(()))
        .unwrap()
        .unwrap()
        .file
        .into_inner();
    let mut rewritten = Recording::open(kept, Limits::default()).unwrap();
    assert!(rewritten.damage().is_empty(), "{:?}", rewritten.damage());
    assert_eq!(rewritten.run_count(), 2);
    assert_eq!(
        rewritten.capture().unwrap(),
        Some(Capture {
            sessions: vec![b"first".to_vec(), b"third".to_vec()],
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
            assert_eq!(hours_of(&rewritten, new, at), hours_of(&original, old, at));
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
    let stop = AtomicBool::new(false);
    for segment in rewritten.segments().to_vec() {
        let mut state = State::new(schema().unwrap()).unwrap();
        let mut walker = Walker::new(&rewritten, &stop).unwrap();
        walker.begin(&segment, &mut state).unwrap();
        assert!(walker.walk(&segment, &mut state, |_, _, _| Ok(true)).unwrap());
    }
    // One session's runs alone name that session alone; a stop on a kept branch stays.
    let second = keep_branches(&original, |branch| branch == 2, Cursor::new(Vec::new()), EVERY_UNIT, |_| Ok(()))
        .unwrap()
        .unwrap()
        .file
        .into_inner();
    assert_eq!(
        Recording::open(second, Limits::default()).unwrap().capture().unwrap(),
        Some(Capture {
            sessions: vec![b"second".to_vec()],
            run_sessions: vec![0],
            controls: vec![start(2), gap, detach],
            ..capture.clone()
        })
    );
    assert!(
        keep_branches(&original, |_| false, Cursor::new(Vec::new()), EVERY_UNIT, |_| Ok(()))
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
        controls: vec![noise(1)],
    };
    let mut file = FileWriter::create(
        Cursor::new(Vec::new()),
        identity(),
        FrameWriter::new().unwrap().columns().to_vec(),
        Settings::RECORDING,
    )
    .unwrap();
    file.revise(Some(&capture), |_| Ok(())).unwrap();
    let first = file.file.get_ref().len();
    capture.controls.push(b"later".to_vec());
    file.revise(Some(&capture), |_| Ok(())).unwrap();
    assert!(file.file.get_ref().len() - first < 4 * 1024);
    let unfinished = Unfinished::read(&mut Cursor::new(file.file.get_ref().clone())).unwrap();
    assert_eq!(unfinished.capture.as_ref(), Some(&capture));
    capture.controls.push(b"last".to_vec());
    let bytes = file
        .finish_with_capture(&capture, |_| Ok(()))
        .unwrap()
        .file
        .into_inner();
    let recording = Recording::open(bytes, Limits::default()).unwrap();
    assert_eq!(recording.capture().unwrap(), Some(capture));
}
