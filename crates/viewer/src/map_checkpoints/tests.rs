use super::*;
use crate::playback::Current;
use dashboard_base::prepared_map::{
    Builder, Position, Progress, decode_checkpoint, header, load_resume, load_resume_prefix,
    section,
};
use replay::{container::SeekResult, spool::Record};
use replay_data::rows::tables;
use replay_data::{
    BranchId, Hour, rows::DivisionStatusRow, rows::Recorded, rows::TableId, rows::TableRows,
};
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::SharedView, frame::TableUpdate,
    frame::TableView,
};
use std::{num::NonZeroU32, path::PathBuf};
use store::{SourceKey, Store};

const SELECTED: &[TableId] = &[TableId::Campaign, TableId::Clock, TableId::DivisionStatus];

struct Fixture {
    path: PathBuf,
    store: Store,
    key: SourceKey,
    id: uuid::Uuid,
    owned: bool,
}
impl Fixture {
    fn new(replay: &Replay) -> Self {
        let id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("map-checkpoints-{id}.battleplan"));
        std::fs::write(&path, replay.bytes()).unwrap();
        let key = SourceKey::capture(&path, &[]).unwrap();
        Self {
            path,
            key,
            store: Store::local(id).unwrap(),
            id,
            owned: true,
        }
    }
    fn source(path: PathBuf) -> Self {
        let id = uuid::Uuid::new_v4();
        let key = SourceKey::capture(&path, &[]).unwrap();
        Self {
            path,
            key,
            store: Store::local(id).unwrap(),
            id,
            owned: false,
        }
    }
    fn reader(&self, progress: Progress, replay: Arc<Replay>, selected: &[TableId]) -> PreparedMap {
        PreparedMap::new(
            Store::local(self.id).unwrap(),
            self.key.clone(),
            progress,
            replay,
            selected,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        if self.owned {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn status_replay(hours: &[Hour]) -> Replay {
    let mut records = crate::playback::tests::records(1, hours);
    for record in &mut records {
        if let Record::Frame(frame) = record {
            frame.tables[TableId::DivisionStatus.index()] = match frame.hour {
                111 => TableUpdate::Unavailable {
                    reason: "status read failed".into(),
                },
                112 => TableUpdate::Truncated {
                    rows: TableRows::DivisionStatus(vec![status(frame.hour)]),
                    missing: 5_000,
                },
                hour if hour % 2 == 0 && !frame.keyframe && hour != 114 => {
                    TableUpdate::Unchanged { baseline: hour - 1 }
                }
                hour => TableUpdate::Snapshot(TableRows::DivisionStatus(vec![status(hour)])),
            };
            frame.metrics.table_ns[TableId::DivisionStatus.index()] =
                if frame.hour % 3 == 0 { 0 } else { 31 };
        }
    }
    Replay::from_records(b"campaign".to_vec(), &[records]).unwrap()
}

fn status(hour: Hour) -> DivisionStatusRow {
    DivisionStatusRow {
        division: u64::MAX,
        maximum_hit_points: i64::MAX - i64::from(hour),
        maximum_organisation: i64::MIN + i64::from(hour),
        border_combat_stats: Recorded::Value(None),
        name: Recorded::Value(Some(vec![0, 255])),
        ..Default::default()
    }
}

fn prepare(fixture: &Fixture, replay: &Replay, stride: u32, selected: &[TableId]) -> Progress {
    let mut builder = Builder::new(
        NonZeroU32::new(stride).unwrap(),
        selected,
        Progress::default(),
    )
    .unwrap();
    builder.set_ranges(replay);
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut last = None;
    for (block, index) in replay.index.iter().enumerate() {
        let frames = replay.read_block_selected(index, selected).unwrap();
        for (offset, frame) in frames.iter().enumerate() {
            let decoded = decoder.apply(frame).unwrap();
            let position = Position {
                block,
                next_frame: offset + 1,
            };
            builder
                .accept(&mut writer, &frame, &decoded, position)
                .unwrap();
            last = Some((decoded, position));
        }
    }
    let (last_frame, position) = last.unwrap();
    builder
        .save_resume(&mut writer, &last_frame, position, selected)
        .unwrap();
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    let progress = builder.progress().clone();
    writer.commit().unwrap();
    progress
}

fn assert_parity(map: &mut PreparedMap, replay: &Replay, branch: BranchId, hour: Hour) {
    let expected = replay.seek_selected(branch, hour, Some(SELECTED)).unwrap();
    let actual = map.seek(branch, hour).unwrap().unwrap();
    match (expected, actual) {
        (SeekResult::Frame(expected), Current::Frame(actual)) => {
            assert_eq!(expected.as_ref(), actual.as_ref());
            for table in SELECTED {
                assert_eq!(
                    expected.tables[table.index()].sampled_at(),
                    actual.tables[table.index()].sampled_at()
                );
            }
        }
        (
            SeekResult::Gap {
                branch: eb,
                hour: eh,
                reason: er,
            },
            Current::Gap {
                branch: ab,
                hour: ah,
                reason: ar,
            },
        ) => assert_eq!((eb, eh, er), (ab, ah, ar)),
        pair => panic!("checkpoint parity differs: {pair:?}"),
    }
}

#[test]
fn nearest_checkpoint_bounds_seek_work_and_step_reuses_current_state() {
    let replay = Arc::new(status_replay(&(100..=355).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    for hour in [355, 101, 300, 111, 112, 113, 114, 301, 100] {
        assert_parity(&mut map, &replay, 1, hour);
        assert!(
            map.applied_frames < 8,
            "seek replayed {} frames",
            map.applied_frames
        );
        assert!(
            map.checkpoint_reads <= 6,
            "checkpoint search read {} headers",
            map.checkpoint_reads
        );
    }
    let Current::Frame(before_frame) = map.advance(1, 100, |_| {}).unwrap().unwrap() else {
        panic!()
    };
    let before = before_frame.tables[TableId::DivisionStatus.index()].clone();
    drop(before_frame);
    let mut passed = Vec::new();
    let current = map
        .advance(1, 101, |frame| passed.push(frame.hour))
        .unwrap()
        .unwrap();
    assert!(matches!(current, Current::Frame(_)));
    assert_eq!(map.applied_frames, 1);
    assert_eq!(map.checkpoint_reads, 0);
    assert!(passed.is_empty());
    assert!(!SharedView::ptr_eq(
        &before,
        &match &current {
            Current::Frame(frame) => frame.tables[TableId::DivisionStatus.index()].clone(),
            _ => unreachable!(),
        }
    ));
    let after = map
        .advance(1, 102, |_| panic!("exact step passed another hour"))
        .unwrap()
        .unwrap();
    assert_eq!(map.applied_frames, 1);
    assert!(SharedView::ptr_eq(
        &match after {
            Current::Frame(frame) => frame.tables[TableId::DivisionStatus.index()].clone(),
            _ => unreachable!(),
        },
        &match current {
            Current::Frame(frame) => frame.tables[TableId::DivisionStatus.index()].clone(),
            _ => unreachable!(),
        }
    ));
}

#[test]
fn cold_checkpoint_seek_decodes_only_source_prefix_and_step_extends_once() {
    let replay = Arc::new(status_replay(&(100..=120).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    assert_parity(&mut map, &replay, 1, 108);
    assert_eq!(
        map.source_reads, 0,
        "an exact checkpoint needs no source payload"
    );
    assert_parity(&mut map, &replay, 1, 110);
    assert_eq!(map.applied_frames, 2);
    assert_eq!(map.source_reads, 1);
    assert_eq!(
        map.source_decoded_frames, 11,
        "source parsing begins at the source keyframe, native application at the checkpoint"
    );
    assert_eq!(map.retained_source_frames, 11);
    for hour in [111, 112, 113] {
        let actual = map
            .advance(1, hour, |_| panic!("step visited an extra hour"))
            .unwrap()
            .unwrap();
        equal_source(
            &actual,
            &replay.seek_selected(1, hour, Some(SELECTED)).unwrap(),
        );
        assert_eq!(map.applied_frames, 1);
        assert_eq!(map.source_reads, u64::from(hour == 111));
        assert_eq!(map.source_decoded_frames, if hour == 111 { 21 } else { 0 });
        assert_eq!(map.retained_source_frames, 21);
    }
}

fn equal_source(actual: &Current, expected: &SeekResult) {
    match (actual, expected) {
        (Current::Frame(actual), SeekResult::Frame(expected)) => {
            assert_eq!(actual.as_ref(), expected.as_ref())
        }
        _ => panic!("fixture hour must be recorded"),
    }
}

#[test]
fn source_step_does_not_parse_beyond_the_accepted_hour_or_block() {
    let replay = Arc::new(status_replay(&[100, 101, 102, 821, 823]));
    let fixture = Fixture::new(&replay);
    let mut progress = prepare(&fixture, &replay, 64, SELECTED);
    let branch = progress.branches.get_mut(&1).unwrap();
    branch.last_hour = Some(101);
    branch.last_block = Some(0);
    branch.since_checkpoint = 1;
    // The unprepared tail must never be decoded, including preloading for steps.
    let mut source = Replay::open_path(&fixture.path).unwrap();
    source.index[1].offset = u64::MAX;
    let mut map = fixture.reader(progress, Arc::new(source), SELECTED);
    drop(map.seek(1, 100).unwrap());
    let actual = map.advance(1, 101, |_| {}).unwrap().unwrap();
    equal_source(
        &actual,
        &replay.seek_selected(1, 101, Some(SELECTED)).unwrap(),
    );
    assert_eq!(map.source_reads, 1);
    assert_eq!(map.source_decoded_frames, 2);
    assert_eq!(map.retained_source_frames, 2);
    assert!(
        map.advance(1, 102, |_| panic!("visited unprepared hour"))
            .unwrap()
            .is_none()
    );
    assert_eq!(map.source_reads, 0);
}

#[test]
fn step_reuses_unretained_row_storage_and_preserves_retained_hours() {
    let initial = status_replay(&[100]);
    let SeekResult::Frame(mut source) = initial.seek(1, 100).unwrap() else {
        panic!("fixture must have an initial frame")
    };
    source.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            (1..=5_000)
                .map(|division| DivisionStatusRow {
                    division,
                    name: Recorded::Value(Some(vec![0, 255, 1])),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    let mut records = crate::playback::tests::records(1, &[100]);
    records.retain(|record| !matches!(record, Record::Frame(_)));
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    for hour in 100..=102 {
        source.hour = hour;
        let TableView::Complete(TableRows::Clock(clock)) =
            &mut *source.tables[TableId::Clock.index()]
        else {
            panic!()
        };
        clock[0].hour = hour;
        let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *source.tables[TableId::DivisionStatus.index()]
        else {
            panic!()
        };
        rows[0].exercising = (hour - 100) as u8;
        source.metrics.table_ns[TableId::DivisionStatus.index()] = 1;
        records.push(Record::Frame(
            encoder.encode(*source.clone(), false).unwrap(),
        ));
    }
    let replay = Arc::new(Replay::from_records(b"campaign".to_vec(), &[records]).unwrap());
    let updates = replay
        .read_block_selected(&replay.index[0], SELECTED)
        .unwrap();
    assert!(matches!(
        updates[1].tables[TableId::DivisionStatus.index()],
        TableUpdate::Delta { .. }
    ));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    playback.seek(1, 100).unwrap();
    let Current::Frame(first) = playback.current().clone() else {
        panic!()
    };
    let storage = first.complete::<tables::DivisionStatus>().unwrap().as_ptr();
    drop(first);
    playback.seek(1, 101).unwrap();
    assert_eq!(
        playback.checkpoint_metrics(),
        Some((1, 0, 1)),
        "5,000 rows require one source block load and no checkpoint search"
    );
    let Current::Frame(retained) = playback.current().clone() else {
        panic!()
    };
    assert_eq!(
        storage,
        retained
            .complete::<tables::DivisionStatus>()
            .unwrap()
            .as_ptr(),
        "the decoder must update an unretained table in its existing allocation"
    );
    playback.seek(1, 102).unwrap();
    assert_eq!(
        playback.checkpoint_metrics(),
        Some((1, 0, 0)),
        "later steps reuse the selected source block even with a retained hour"
    );
    let Current::Frame(next) = playback.current().clone() else {
        panic!()
    };
    assert_eq!(
        retained.complete::<tables::DivisionStatus>().unwrap()[0].exercising,
        1
    );
    assert_eq!(
        next.complete::<tables::DivisionStatus>().unwrap()[0].exercising,
        2
    );
    assert_eq!(
        retained.complete::<tables::DivisionStatus>().unwrap().len(),
        5_000
    );
}

#[test]
fn full_map_hook_matches_existing_playback_holes_branches_and_missing_tables() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 2, &TableId::ALL);
    let map = fixture.reader(progress, Arc::clone(&replay), &TableId::ALL);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    for hour in [100, 101, 102, 103, 104, 105, 100, 104, 819, 820] {
        let expected = replay.seek(1, hour).unwrap();
        playback.seek(1, hour).unwrap();
        match (expected, playback.current()) {
            (SeekResult::Frame(expected), Current::Frame(actual)) => {
                assert_eq!(expected.as_ref(), actual.as_ref());
            }
            (SeekResult::Gap { reason, .. }, Current::Gap { reason: actual, .. }) => {
                assert_eq!(&reason, actual)
            }
            pair => panic!("map playback differs: {pair:?}"),
        }
        assert_eq!(playback.hour(), hour);
    }
    assert!(playback.seek(999, 100).is_err());
    assert!(!playback.step().unwrap());
    playback.seek(1, 101).unwrap();
    assert!(playback.step().unwrap());
    assert!(
        matches!(playback.current(), Current::Gap { hour: 102, reason, .. } if reason == "flush failed")
    );
}

#[test]
fn persisted_resume_matches_last_accepted_selected_table_state() {
    let replay = Arc::new(status_replay(&(100..=117).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let (state, position) = load_resume(&fixture.store, &fixture.key, &progress, SELECTED)
        .unwrap()
        .unwrap();
    let expected = replay.seek_selected(1, 117, Some(SELECTED)).unwrap();
    assert!(matches!(expected, SeekResult::Frame(frame) if *frame == state));
    assert_eq!(
        position,
        Position {
            block: 0,
            next_frame: 18
        }
    );
    assert_eq!(
        state.tables[TableId::DivisionStatus.index()].sampled_at(),
        None
    );
    let mut resumed = FrameDecoder::resume_for_tables(SELECTED, state).unwrap();
    let source = replay
        .read_block_selected(&replay.index[0], SELECTED)
        .unwrap();
    assert!(
        resumed.advance(&source[17]).is_err(),
        "accepted hour must never be decoded twice"
    );
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    assert!(
        map.seek(1, 118).unwrap().is_none(),
        "unprepared target uses the ordinary path"
    );
}

#[test]
fn selected_source_reads_skip_interleaved_other_branches() {
    let first = crate::playback::tests::records(1, &[100, 101]);
    let second = crate::playback::tests::records(2, &[200, 201]);
    let mut first_tail = crate::playback::tests::records(1, &[820]);
    let mut second_tail = crate::playback::tests::records(2, &[920]);
    first_tail[1] = first[1].clone();
    second_tail[1] = second[1].clone();
    let replay = Replay::from_records(
        b"campaign".to_vec(),
        &[first, second, first_tail, second_tail],
    )
    .unwrap();
    assert_eq!(replay.index.len(), 4);
    // Encode interleaving in the file so anchor distances retain their ordinals.
    assert_eq!(
        replay
            .index
            .iter()
            .map(|block| (block.branch, block.first_hour))
            .collect::<Vec<_>>(),
        [(1, 100), (2, 200), (1, 820), (2, 920)]
    );
    let replay = Arc::new(replay);
    let fixture = Fixture::new(&replay);
    let held = Replay::open_path(&fixture.path).unwrap();
    for (resident, file) in replay.index.iter().zip(&held.index) {
        assert_eq!(
            replay.read_block(resident).unwrap(),
            held.read_block(file).unwrap()
        );
    }
    let progress = prepare(&fixture, &held, 24, &TableId::ALL);
    let mut map = fixture.reader(progress, Arc::new(held), &TableId::ALL);
    for (branch, hour) in [(1, 820), (2, 920), (1, 101), (2, 201)] {
        let actual = map.seek(branch, hour).unwrap().unwrap();
        let expected = replay.seek(branch, hour).unwrap();
        assert!(
            matches!((actual, expected), (Current::Frame(a), SeekResult::Frame(b)) if a.as_ref() == b.as_ref())
        );
    }
}

#[test]
fn sparse_checkpoints_store_no_hourly_updates_and_resume_keeps_full_selection() {
    let replay = Arc::new(status_replay(&[100, 101]));
    let fixture = Fixture::new(&replay);
    let mut builder = Builder::new(
        NonZeroU32::new(8).unwrap(),
        &[TableId::Campaign],
        Progress::default(),
    )
    .unwrap();
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    for (offset, frame) in replay
        .read_block_selected(&replay.index[0], SELECTED)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let decoded = decoder.advance(&frame).unwrap();
        builder
            .accept(
                &mut writer,
                &frame,
                decoded,
                Position {
                    block: 0,
                    next_frame: offset + 1,
                },
            )
            .unwrap();
    }
    builder
        .save_resume(
            &mut writer,
            decoder.current().unwrap(),
            Position {
                block: 0,
                next_frame: 2,
            },
            SELECTED,
        )
        .unwrap();
    let progress = builder.progress().clone();
    writer.commit().unwrap();
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "map.updates.1")
            .unwrap(),
        None,
        "hourly updates exist only in the recording"
    );
    let mut map = fixture.reader(progress.clone(), Arc::clone(&replay), &[TableId::Campaign]);
    let Current::Frame(checkpoint) = map.seek(1, 100).unwrap().unwrap() else {
        panic!()
    };
    assert!(matches!(
        &*checkpoint.tables[TableId::DivisionStatus.index()],
        TableView::Unavailable { .. }
    ));
    drop(checkpoint);
    let Current::Frame(update) = map.advance(1, 101, |_| {}).unwrap().unwrap() else {
        panic!()
    };
    assert!(matches!(
        &*update.tables[TableId::DivisionStatus.index()],
        TableView::Unavailable { .. }
    ));
    assert_eq!(
        map.checkpoint_bytes, 0,
        "step never reloads a native checkpoint"
    );
    let (resume, _) = load_resume(&fixture.store, &fixture.key, &progress, SELECTED)
        .unwrap()
        .unwrap();
    assert!(resume.complete::<tables::DivisionStatus>().is_ok());
    assert_eq!(progress.branches[&1].checkpoints, 1);
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "map.blocks.1")
            .unwrap(),
        None
    );
}

#[test]
fn step_across_source_keyframe_loads_one_selected_block_without_checkpoint() {
    let first = crate::playback::tests::records(1, &(100..108).collect::<Vec<_>>());
    let mut tail = crate::playback::tests::records(1, &[108, 109]);
    tail[1] = first[1].clone();
    let replay = Arc::new(Replay::from_records(b"campaign".to_vec(), &[first, tail]).unwrap());
    assert_eq!(replay.index.len(), 2);
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    assert_eq!(progress.branches[&1].checkpoints, 2);
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    drop(map.seek(1, 107).unwrap());
    let actual = map.advance(1, 108, |_| {}).unwrap().unwrap();
    let SeekResult::Frame(expected) = replay.seek_selected(1, 108, Some(SELECTED)).unwrap() else {
        panic!()
    };
    assert!(matches!(actual, Current::Frame(frame) if *frame == *expected));
    assert_eq!(
        (
            map.applied_frames,
            map.source_reads,
            map.checkpoint_reads,
            map.checkpoint_bytes
        ),
        (1, 1, 0, 0)
    );
}

#[test]
fn step_after_checkpoint_at_source_block_end_loads_only_the_new_block() {
    let first = crate::playback::tests::records(1, &[100, 101]);
    let mut tail = crate::playback::tests::records(1, &[102, 103]);
    tail[1] = first[1].clone();
    let replay = Arc::new(Replay::from_records(b"campaign".to_vec(), &[first, tail]).unwrap());
    assert_eq!(replay.index.len(), 2);
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 1, SELECTED);
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    drop(map.seek(1, 101).unwrap());
    assert_eq!(map.source_reads, 0);
    let actual = map
        .advance(1, 102, |_| panic!("step visited an extra hour"))
        .unwrap()
        .unwrap();
    equal_source(
        &actual,
        &replay.seek_selected(1, 102, Some(SELECTED)).unwrap(),
    );
    assert_eq!(map.source_reads, 1);
    assert_eq!(map.source_decoded_frames, 2);
    assert_eq!(map.applied_frames, 1);
    assert_eq!(map.checkpoint_reads, 0);
}

#[test]
fn later_checkpoint_needs_only_its_own_native_payload() {
    use std::io::Read;
    let replay = Arc::new(status_replay(&(100..=130).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let mut old = fixture
        .store
        .read(&fixture.key, &section(1), 1)
        .unwrap()
        .unwrap();
    let mut header_only = [0; 20];
    old.read_exact(&mut header_only).unwrap();
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    writer.write_at(&section(1), 1, &header_only[..]).unwrap();
    writer.commit().unwrap();
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    assert_parity(&mut map, &replay, 1, 130);
    assert_eq!(map.applied_frames, 6);
    let bytes: u64 = [3]
        .into_iter()
        .map(|range| {
            fixture
                .store
                .read(&fixture.key, &section(1), range)
                .unwrap()
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(
        map.checkpoint_bytes, bytes,
        "only the nearest checkpoint payload is read"
    );
}

#[test]
fn cancelled_preparation_resumes_only_new_frames_and_retains_old_checkpoint_pages() {
    let replay = Arc::new(status_replay(&(100..=130).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let frames = replay
        .read_block_selected(&replay.index[0], SELECTED)
        .unwrap();
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    let mut builder =
        Builder::new(NonZeroU32::new(8).unwrap(), SELECTED, Progress::default()).unwrap();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    for (offset, frame) in frames.iter().take(18).enumerate() {
        let decoded = decoder.apply(frame).unwrap();
        builder
            .accept(
                &mut writer,
                &frame,
                &decoded,
                Position {
                    block: 0,
                    next_frame: offset + 1,
                },
            )
            .unwrap();
    }
    builder
        .save_resume(
            &mut writer,
            decoder.current().unwrap(),
            Position {
                block: 0,
                next_frame: 18,
            },
            SELECTED,
        )
        .unwrap();
    let accepted = builder.progress().clone();
    writer.commit().unwrap();
    let old_page = fixture
        .store
        .read(&fixture.key, &section(1), 0)
        .unwrap()
        .unwrap();
    let old_resume = fixture
        .store
        .read(&fixture.key, "map.resume", 0)
        .unwrap()
        .unwrap();
    let (state, position) = load_resume(&fixture.store, &fixture.key, &accepted, SELECTED)
        .unwrap()
        .unwrap();
    let mut decoder = FrameDecoder::resume_for_tables(SELECTED, state).unwrap();
    let mut builder = Builder::new(NonZeroU32::new(8).unwrap(), SELECTED, accepted).unwrap();
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    let mut new_frames = 0;
    for (offset, frame) in frames.iter().enumerate().skip(position.next_frame) {
        let decoded = decoder.apply(frame).unwrap();
        builder
            .accept(
                &mut writer,
                &frame,
                &decoded,
                Position {
                    block: 0,
                    next_frame: offset + 1,
                },
            )
            .unwrap();
        new_frames += 1;
    }
    builder
        .save_resume(
            &mut writer,
            decoder.current().unwrap(),
            Position {
                block: 0,
                next_frame: frames.len(),
            },
            SELECTED,
        )
        .unwrap();
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    let progress = builder.progress().clone();
    writer.commit().unwrap();
    assert_eq!(new_frames, 13);
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "map.resume")
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        decode_checkpoint(old_resume, SELECTED).unwrap().frame.hour,
        117
    );
    assert_eq!(
        load_resume(&fixture.store, &fixture.key, &progress, SELECTED)
            .unwrap()
            .unwrap()
            .0
            .hour,
        130
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, &section(1))
            .unwrap(),
        Some(4)
    );
    assert_eq!(header(old_page).unwrap(), (1, 100));
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    assert_parity(&mut map, &replay, 1, 130);
    assert!(map.applied_frames < 8);
}

#[test]
fn clear_leaves_valid_recording_playback_on_the_ordinary_reader() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 2, &TableId::ALL);
    let map = fixture.reader(progress, Arc::clone(&replay), &TableId::ALL);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    playback.seek(1, 104).unwrap();
    fixture.store.clear().unwrap();
    playback.seek(1, 105).unwrap();
    assert!(
        matches!(replay.seek(1, 105).unwrap(), SeekResult::Frame(frame) if playback.frame() == Some(&*frame))
    );
}

#[test]
fn rejected_auxiliary_page_uses_oracle_without_repeating_passed_hours() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let fixture = Fixture::new(&replay);
    let mut progress = prepare(&fixture, &replay, 2, &TableId::ALL);
    // A page descriptor beyond the committed manifest is a cache failure,
    // including a Clear racing the request. It is not a recording failure.
    progress.branches.get_mut(&1).unwrap().checkpoints += 1_000;
    let map = fixture.reader(progress, Arc::clone(&replay), &TableId::ALL);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    let mut passed = Vec::new();
    playback
        .seek_with(1, 105, |frame| passed.push(frame.hour))
        .unwrap();
    assert_eq!(passed, vec![100, 101, 104]);
    assert!(
        matches!(replay.seek(1, 105).unwrap(), SeekResult::Frame(frame) if playback.frame() == Some(&*frame))
    );
}

#[test]
fn corrupt_checkpoint_falls_back_without_repeating_passed_hours() {
    let replay = Arc::new(crate::playback::tests::fixture());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, &TableId::ALL);
    let map = fixture.reader(progress, Arc::clone(&replay), &TableId::ALL);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    writer
        .write_at(&section(1), 0, &b"corrupt checkpoint"[..])
        .unwrap();
    writer.commit().unwrap();
    let mut passed = Vec::new();
    playback
        .seek_with(1, 105, |frame| passed.push(frame.hour))
        .unwrap();
    assert_eq!(passed, [100, 101, 104]);
    assert!(
        matches!(replay.seek(1, 105).unwrap(), SeekResult::Frame(frame)
        if playback.frame() == Some(&*frame))
    );
}

#[test]
fn checkpoint_cursor_must_match_source_before_visiting_history() {
    use std::io::Read;
    let replay = Arc::new(status_replay(&[100, 101, 102]));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 8, SELECTED);
    let mut bytes = Vec::new();
    fixture
        .store
        .read(&fixture.key, &section(1), 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    // A plausible in-block cursor must still name this checkpoint's exact hour.
    bytes[28..36].copy_from_slice(&2u64.to_le_bytes());
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    writer.write_at(&section(1), 0, bytes.as_slice()).unwrap();
    writer.commit().unwrap();
    let mut map = fixture.reader(progress, Arc::clone(&replay), SELECTED);
    let mut passed = Vec::new();
    let error = map
        .seek_with(1, 102, |frame| passed.push(frame.hour))
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("cursor differs"));
    assert!(passed.is_empty());
    assert_eq!(map.applied_frames, 0);
    assert_eq!(map.source_reads, 1);
}

#[test]
fn appended_source_restores_accepted_prefix_and_prepares_only_new_blocks() {
    let original = status_replay(&(100..=117).collect::<Vec<_>>());
    let fixture = Fixture::new(&original);
    let accepted = prepare(&fixture, &original, 8, SELECTED);
    let appended = Arc::new(
        original
            .append(&[crate::playback::tests::records(2, &[200, 201])])
            .unwrap(),
    );
    // Canonical append retains all old compressed blocks. Verify those bytes
    // before authorizing the store's explicitly accepted-prefix reader.
    for index in &original.index {
        let start = index.offset as usize;
        let end = (index.offset + index.length) as usize;
        assert_eq!(&original.bytes()[start..end], &appended.bytes()[start..end]);
    }
    std::fs::write(&fixture.path, appended.bytes()).unwrap();
    let key = SourceKey::capture(&fixture.path, &[]).unwrap();
    assert!(
        fixture
            .store
            .read(&fixture.key, "map.resume", accepted.resume_range.unwrap())
            .unwrap()
            .is_none()
    );
    let (state, _) = load_resume_prefix(&fixture.store, &fixture.key, &key, &accepted, SELECTED)
        .unwrap()
        .unwrap();
    assert_eq!((state.branch, state.hour), (1, 117));
    let mut decoder = FrameDecoder::resume_for_tables(SELECTED, state).unwrap();
    let mut builder =
        Builder::new(NonZeroU32::new(8).unwrap(), SELECTED, accepted.clone()).unwrap();
    let mut writer = fixture.store.append(&fixture.key, key.clone()).unwrap();
    let mut new_frames = 0;
    let mut last = None;
    for (block, index) in appended.index.iter().enumerate().skip(original.index.len()) {
        for (offset, frame) in appended
            .read_block_selected(index, SELECTED)
            .unwrap()
            .iter()
            .enumerate()
        {
            let decoded = decoder.apply(frame).unwrap();
            let position = Position {
                block,
                next_frame: offset + 1,
            };
            builder
                .accept(&mut writer, &frame, &decoded, position)
                .unwrap();
            new_frames += 1;
            last = Some((decoded, position));
        }
    }
    let (frame, position) = last.unwrap();
    builder
        .save_resume(&mut writer, &frame, position, SELECTED)
        .unwrap();
    builder.save_gaps(&mut writer, &appended.gaps).unwrap();
    let progress = builder.progress().clone();
    writer.commit().unwrap();
    assert_eq!(new_frames, 2);
    assert_eq!(
        progress.branches[&1].checkpoints,
        accepted.branches[&1].checkpoints
    );
    let mut map = PreparedMap::new(
        Store::local(fixture.id).unwrap(),
        key,
        progress,
        Arc::clone(&appended),
        SELECTED,
    )
    .unwrap();
    assert_parity(&mut map, &appended, 1, 117);
    assert_parity(&mut map, &appended, 2, 201);
}

mod measurement;

#[test]
fn prepared_and_ordinary_hours_build_equal_operative_operation_and_progress_icons() {
    use replay_data::rows::*;
    use replay_frame::frame::FrameEncoder;
    use viewer_raids::map::{
        intel_icons::{EntryKind, IntelAssets, IntelDefines, OperationPriorities, Write, icons},
        raids::test_support::{self, put},
    };
    const HOVER_TABLES: [TableId; 4] = [
        TableId::IntelContributions,
        TableId::IntelSources,
        TableId::IntelNetworkSummaries,
        TableId::PropagandaEfforts,
    ];
    let selected = dashboard_base::prepared_map::selected_tables();
    let mut records = crate::playback::tests::records(1, &[100]);
    records.retain(|record| !matches!(record, Record::Frame(_)));
    let mut encoder = FrameEncoder::new();
    for hour in 100..=102 {
        let mut frame = test_support::positive_frame();
        frame.branch = 1;
        frame.hour = hour;
        // The direct fixture uses a historical absence for this unexercised
        // input. Current-format rows encode an explicit independent refusal.
        let mut counters = frame
            .complete::<tables::CountryCounterInputs>()
            .unwrap()
            .to_vec();
        for row in &mut counters {
            row.forward_visibility = Recorded::Value(None);
        }
        put::<tables::CountryCounterInputs>(&mut frame, counters);
        put::<tables::Campaign>(
            &mut frame,
            vec![CampaignRow {
                id: b"campaign".to_vec(),
            }],
        );
        put::<tables::Clock>(
            &mut frame,
            vec![ClockRow {
                hour,
                ..Default::default()
            }],
        );
        put::<tables::Politics>(
            &mut frame,
            vec![
                PoliticsRow {
                    country: 10,
                    capital: 10,
                    ..Default::default()
                },
                PoliticsRow {
                    country: 11,
                    capital: 20,
                    ..Default::default()
                },
            ],
        );
        put::<tables::OperativeMissions>(
            &mut frame,
            [6, 8, 2]
                .into_iter()
                .enumerate()
                .map(|(i, variant)| OperativeMissionRow {
                    id: i as u64 + 10,
                    ordinal: i as u32,
                    owner_country: 10,
                    roster_country: 10,
                    assignment: 3,
                    variant: Some(variant),
                    target_country: Some(11),
                    target_state: Some(20),
                    ..Default::default()
                })
                .collect(),
        );
        put::<tables::IntelContributions>(
            &mut frame,
            vec![IntelContributionRow {
                owner: 10,
                target: 11,
                kind: 4,
                pool: 17,
                civilian: i64::from(hour),
                ..Default::default()
            }],
        );
        put::<tables::IntelSources>(
            &mut frame,
            vec![IntelSourceRow {
                owner: 10,
                target: 11,
                pool: 19,
                source: 3,
                army: -i64::from(hour),
                ..Default::default()
            }],
        );
        put::<tables::IntelNetworkSummaries>(
            &mut frame,
            vec![IntelNetworkSummaryRow {
                owner_country: 10,
                target_country: 11,
                covered_core_states: hour - 99,
                controlled_strength_sum: i64::from(hour),
                ..Default::default()
            }],
        );
        put::<tables::PropagandaEfforts>(
            &mut frame,
            vec![PropagandaEffortRow {
                slot: 1,
                country: 11,
                sequence: u64::MAX,
                stability: i64::from(hour),
                war_support: -i64::from(hour),
                ..Default::default()
            }],
        );
        for table in HOVER_TABLES {
            frame.metrics.table_ns[table.index()] = 1;
        }
        if hour == 101 {
            for table in HOVER_TABLES {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: format!("synthetic {table:?} acquisition refusal"),
                };
            }
        }
        put::<tables::StrategicOperatives>(
            &mut frame,
            vec![StrategicOperativeRow {
                country: 11,
                slot: 1,
                trade_influence: vec![TradeInfluence {
                    owner: 10,
                    value: 50_000,
                }],
                ..Default::default()
            }],
        );
        put::<tables::CountryStabilityInputs>(
            &mut frame,
            vec![CountryStabilityInputRow {
                country: 11,
                propaganda_stability_q15: 16_384,
                propaganda_war_support_q15: 8_192,
                keys: vec![u32::MAX; CountryStabilityInputRow::KEY_NAMES.len()],
                ..Default::default()
            }],
        );
        put::<tables::Operations>(
            &mut frame,
            vec![OperationRow {
                id: 1,
                owner: 10,
                target: 11,
                start_hour: 100,
                duration_days: 1,
                ..Default::default()
            }],
        );
        put::<tables::AgencyOperationBuffers>(
            &mut frame,
            vec![AgencyOperationBuffersRow {
                running: vec![AgencyBufferedOperation {
                    operation: 1,
                    state: Some(20),
                }],
                ..Default::default()
            }],
        );
        // The direct icon fixture leaves unrelated singletons empty. An encoded
        // frame must distinguish their missing operands from complete empty lists.
        for table in TableId::ALL {
            let view = &mut frame.tables[table.index()];
            if table.key_bits() == 0
                && matches!(&**view, TableView::Complete(rows) if rows.is_empty())
            {
                **view = TableView::Unavailable {
                    reason: "outside the operative checkpoint fixture".into(),
                };
            }
        }
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let replay = Arc::new(Replay::from_records(b"campaign".to_vec(), &[records]).unwrap());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 2, selected);
    let mut prepared = fixture.reader(progress, Arc::clone(&replay), selected);
    let assets = IntelAssets::test(
        IntelDefines {
            cutoff: 100_000_000,
            offset: [0, 0],
            max_stack: 4,
            priorities: OperationPriorities {
                can_start: 1,
                completed: 2,
                in_progress: 3,
                prepared: 4,
                default: 5,
            },
            counter: Err("synthetic shield operands absent".into()),
            activity: Err("synthetic colours absent".into()),
            root_out: Ok([0, 0]),
            control_trade_max: Ok(100_000),
            diplomatic_max: Ok(100_000),
            propaganda_max: Ok([100_000, 100_000]),
        },
        Vec::new(),
        Vec::new(),
        0,
    );
    let raids = test_support::assets();
    let loc = installed_content_localisation::localisation::Localisation::default();
    let build = |frame: &DecodedFrame| icons(&assets, &raids, frame, 2, &loc, "english");
    for hour in [102, 100, 101] {
        let SeekResult::Frame(ordinary) = replay.seek(1, hour).unwrap() else {
            panic!("ordinary hour missing")
        };
        let Some(Current::Frame(actual)) = prepared.seek(1, hour).unwrap() else {
            panic!("prepared hour missing")
        };
        for table in HOVER_TABLES {
            assert_eq!(
                actual.tables[table.index()],
                ordinary.tables[table.index()],
                "{table:?} hour {hour}"
            );
            assert_eq!(
                actual.tables[table.index()].sampled_at(),
                ordinary.tables[table.index()].sampled_at(),
                "{table:?} supplying read at hour {hour}"
            );
        }
        let expected = build(&ordinary);
        let actual = build(&actual);
        let entries: Vec<_> = expected
            .states
            .iter()
            .flat_map(|state| &state.entries)
            .collect();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Operative)
                .count(),
            3
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Operation)
                .count(),
            1
        );
        for progress in ["50%", "25%"] {
            assert!(
                entries
                    .iter()
                    .flat_map(|entry| &entry.writes)
                    .any(|write| matches!(write, Write::Text(_, text) if text.contains(progress)))
            );
        }
        if hour != 101 {
            for detail in [
                "Prior-day dynamic contribution",
                "Static pool 19, source 3",
                "Recorded national network cache",
                "Recorded propaganda history",
            ] {
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.tooltip.body.contains(detail)),
                    "missing {detail} at hour {hour}"
                );
            }
        }
        assert_eq!(actual.states, expected.states, "hour {hour}");
        assert_eq!(actual.report, expected.report, "hour {hour}");
    }
}

#[test]
fn prepared_hours_preserve_native_ordinals_and_market_route_state_endpoints() {
    use replay_data::rows::tables;
    let replay = Arc::new(status_replay(&[100, 101, 102, 103]));
    let fixture = Fixture::new(&replay);
    let selected = dashboard_base::prepared_map::selected_tables();
    let progress = prepare(&fixture, &replay, 2, selected);
    let mut prepared = fixture.reader(progress, Arc::clone(&replay), selected);
    for hour in [103, 100, 102, 101] {
        let SeekResult::Frame(ordinary) = replay.seek_selected(1, hour, Some(selected)).unwrap()
        else {
            panic!("ordinary hour absent")
        };
        let Some(Current::Frame(actual)) = prepared.seek(1, hour).unwrap() else {
            panic!("prepared hour absent")
        };
        assert_eq!(actual.as_ref(), ordinary.as_ref());
        assert_eq!(
            actual.complete::<tables::ResourceOrigins>().unwrap()[0].ordinal,
            hour as u32
        );
        assert_eq!(
            actual.complete::<tables::ResourceRights>().unwrap()[0].ordinal,
            hour as u32
        );
        let route = &actual.complete::<tables::TradeRoutes>().unwrap()[0];
        assert_eq!(
            (route.source_state, route.destination_state),
            (Some(hour as u32), Some(hour as u32 + 1))
        );
        let client = &actual.complete::<tables::ConvoyClients>().unwrap()[0];
        assert_eq!(
            (client.owner_ordinal, client.other_ordinal),
            (Some(hour as u32), Some(hour as u32 + 1))
        );
    }
}
