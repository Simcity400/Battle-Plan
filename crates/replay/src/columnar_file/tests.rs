use super::*;
use crate::{
    columnar_frames::{Recording, Segment, Views, schema},
    format::FormatVersion,
    frame::{DecodedFrame, FrameInput, Metrics, TableInput, TableView},
    native_frame::{FrameHour, FrameWriter},
    rows::{CountryRow, TABLE_COUNT, TableId, TableRows},
    scope::{self, CaptureMode},
};
use replay_columnar::{manifest::Manifest, walk::State};
use std::io::Cursor;

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
    (file.finish().unwrap().file.into_inner(), frames)
}

/// Every hour walked from the file's start, and every segment begun from its restart, gives
/// the frames the hours were written with.
#[test]
fn a_run_reads_back_from_its_start_and_from_every_restart() {
    let hours = 2 * envelope::UNIT_ORDINALS as i32 + 5;
    let (bytes, frames) = written(hours, EVERY_UNIT);
    let recording = Recording::open(bytes).unwrap();
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
        recording.begin_segment(segment, &mut state).unwrap();
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
    let recording = Recording::open(bytes).unwrap();
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
    let recording = Recording::open(file.finish().unwrap().file.into_inner()).unwrap();
    assert!(recording.damage().is_empty(), "{:?}", recording.damage());
    assert_eq!(recording.hours(), 3);
}
