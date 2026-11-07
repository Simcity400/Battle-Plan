use super::*;
use crate::{
    columnar_frames::{
        fixtures,
        synthetic::{Input, Sample, Synthetic},
    },
    frame::SharedView,
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


/// `recording` as an index holds it.
fn alone(recording: Recording) -> Whole {
    Whole {
        campaign: recording.campaign().to_vec(),
        recordings: vec![recording],
    }
}

/// The index of `tables` of `bytes`, a recording, walked on `threads` workers.
fn prepared(bytes: &[u8], tables: &[TableId], threads: usize) -> Vec<u8> {
    let recording = Recording::open(bytes.to_vec()).unwrap();
    let (written, _) = prepare(
        &alone(recording),
        tables,
        LAYOUT,
        LEVEL,
        threads,
        &std::env::temp_dir(),
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
    let recording = Recording::open(bytes).unwrap();
    assert!(recording.segments().len() > RUNS.len());
    let path = std::env::temp_dir().join(format!("columnar-runs-{}.h4runs", std::process::id()));
    std::fs::write(&path, &serial).unwrap();
    for file in [
        IndexFile::from_bytes(serial.clone()).unwrap(),
        IndexFile::open(&path).unwrap(),
    ] {
        let recordings = std::slice::from_ref(&recording);
        assert!(file.indexes(recordings, &TableId::ALL));
        assert!(!file.indexes(recordings, &TABLES));
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
        format: 1,
        journaled: 0,
        controls: Vec::new(),
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
    let recording = |bytes: Vec<u8>| Recording::open(bytes).unwrap();
    assert!(file.indexes(&[recording(bytes.clone())], &tables));
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
        assert!(!file.indexes(&[other], &tables));
    }
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
    let recording = Recording::open(bytes).unwrap();
    let stopped = prepare(
        &alone(recording),
        &TABLES,
        LAYOUT,
        LEVEL,
        2,
        &std::env::temp_dir(),
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
                    assert!(snapshots.move_to(branch, hour, &alone).unwrap());
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
    )
    .unwrap();
    let mut before: Option<BTreeSet<u128>> = None;
    for (at, &(branch, hour)) in hours.iter().enumerate().take(2 * UNIT_ORDINALS as usize) {
        assert!(snapshots.move_to(branch, hour, &|| {}).unwrap());
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
