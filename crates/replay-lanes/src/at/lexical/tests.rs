use super::*;
use replay_data::{format::FormatVersion, rows::{LexicalTokenRow, StateRow}};
use replay_journal::{fixtures, synthetic::{Input, Sample}};

const START: i32 = 100;
const RUNS: [(u64, i32, u32); 2] = [(1, START, 2), (2, START + 10_000, 1)];

fn fixture() -> Vec<u8> {
    fixtures::written_at(recorder_derive::writers_within, FormatVersion::CURRENT.version(), b"exact lexical", &RUNS, &[TABLE, TableId::States], |synthetic, _, ordinal| {
        let mut rows = vec![LexicalTokenRow { token: 1, literal: if ordinal < 51 { b"one" } else { b"uno" }.to_vec() }];
        if ordinal >= 51 { rows.push(LexicalTokenRow { token: 2, literal: b"two".to_vec() }); }
        if ordinal >= 75 { rows.push(LexicalTokenRow { token: 3, literal: b"three".to_vec() }); }
        if ordinal >= 100 { rows.insert(0, LexicalTokenRow { token: 0, literal: b"zero".to_vec() }); }
        let input = if ordinal == 123 {
            Input::Unavailable { reason: "lexical reader refused the sample", read: true, sample: Sample::Reset }
        } else { Input::Snapshot {
            rows: &rows, partial: (ordinal == 120).then_some(1),
            read: ordinal % 3 == 0, sample: Sample::FromReadBit,
        } };
        synthetic.table(TABLE, input).unwrap();
        synthetic.table(TableId::States, Input::Snapshot {
            rows: &[StateRow { id: 1, owner: if ordinal < 51 { 7 } else { 8 }, ..Default::default() }],
            partial: (ordinal == 120).then_some(1), read: true, sample: Sample::FromReadBit,
        }).unwrap();
    }).0
}

#[test]
fn complete_table_cursor_matches_exact_reads_and_skips_unchanged_rows() {
    let recording = Recording::open(fixture()).unwrap();
    let mut cursor = CompleteCursor::default();
    for (hour, rows) in [(START, 1), (START + 48, 0), (START + 72, 1)] {
        let got = cursor.read(&recording, 0, hour, TableId::States, RETAINED_BYTES).unwrap();
        let expected = super::super::frame(&recording, 0, hour, &[super::super::Read::all(TableId::States)]).unwrap();
        assert_eq!(got.0, expected.tables[TableId::States.index()]);
        assert_eq!(got.0.sampled_at(), Some(hour));
        assert!(got.1);
        assert_eq!(got.2, rows);
        assert_eq!(cursor.held.as_ref().unwrap().reader.restores(), 1);
    }
    assert!(cursor.read(&recording, 0, START + 120, TableId::States, RETAINED_BYTES).is_none());
    assert_eq!(cursor.retained_bytes(), 0);
}

fn compared(cache: &mut LexicalCursor, recording: &Recording, run: u32, hour: i32) -> (SharedView, u64) {
    let got = cache.read(recording, run, hour).expect("complete lexical table");
    let expected = super::super::frame(recording, run, hour, &[super::super::Read::all(TABLE)]).unwrap();
    let view = &expected.tables[TABLE.index()];
    assert_eq!(&got.0, view);
    assert_eq!(got.0.sampled_at(), view.sampled_at());
    assert_eq!(got.1, expected.metrics.table_ns[TABLE.index()] != 0);
    (got.0, got.2)
}

#[test]
fn lexical_playback_reuses_the_reader_and_unchanged_views() {
    let recording = Recording::open(fixture()).unwrap();
    for step in [1, 3, 24] {
        let mut cache = LexicalCursor::default();
        let (first, rows) = compared(&mut cache, &recording, 0, START);
        assert_eq!(rows, 1);
        for n in 1..=72 / step {
            let offset = n * step;
            let (view, rows) = compared(&mut cache, &recording, 0, START + offset);
            assert_eq!(SharedView::ptr_eq(&first, &view), offset < 51);
            assert_eq!(rows, if offset - step < 51 && offset >= 51 { 2 } else { 0 });
            assert_eq!(cache.0.held.as_ref().unwrap().reader.restores(), 1);
        }
        let (_, rows) = compared(&mut cache, &recording, 0, START + 72 + step.max(3));
        assert_eq!(rows, 1);
        let held = cache.0.held.as_ref().unwrap();
        assert_eq!(held.view_bytes, view_heap_bytes(held.view.held()) + size_of::<TableView>() + 2 * size_of::<usize>());
        let (_, rows) = compared(&mut cache, &recording, 0, START + 100);
        assert_eq!(rows, 4);
        assert_eq!(cache.0.held.as_ref().unwrap().reader.restores(), 1);
    }
}

#[test]
fn lexical_playback_releases_its_view_on_a_seek_or_inexact_read() {
    let recording = Recording::open(fixture()).unwrap();
    let mut cache = LexicalCursor::default();
    let (first, _) = compared(&mut cache, &recording, 0, START + 3);
    let (back, _) = compared(&mut cache, &recording, 0, START);
    assert!(!SharedView::ptr_eq(&first, &back));
    assert!(cache.read(&recording, 0, START + 120).is_none());
    assert!(cache.0.held.is_none());
    assert!(cache.read(&recording, 0, START + 123).is_none());
    assert!(cache.0.held.is_none());
    let (before, _) = compared(&mut cache, &recording, 0, START + BLOCK as i32 - 1);
    let (after, _) = compared(&mut cache, &recording, 0, START + BLOCK as i32);
    assert!(!SharedView::ptr_eq(&before, &after));
    assert!(cache.read(&recording, 0, START + (RUNS[0].2 * BLOCK) as i32).is_none());
    assert!(cache.0.held.is_none());
    let (other, _) = compared(&mut cache, &recording, 1, START + 10_000);
    assert!(!SharedView::ptr_eq(&after, &other));
    assert!(cache.0.read(&recording, 1, START + 10_001, TABLE, 0).is_some());
    assert!(cache.0.held.is_none());
}

#[test]
fn the_same_recordings_projection_cannot_use_a_full_lexical_view() {
    let mut recording = Recording::open(fixture()).unwrap();
    let mut cache = LexicalCursor::default();
    let _ = compared(&mut cache, &recording, 0, START);
    recording.project(TABLE, &[]).unwrap();
    assert!(cache.read(&recording, 0, START).is_none());
    assert!(cache.0.held.is_none());
}

#[test]
fn a_damaged_checkpoint_cannot_be_bypassed_by_a_lexical_reader() {
    let mut bytes = fixture();
    let metadata = Recording::open(bytes.clone()).unwrap();
    let page = metadata.table_entry(0, 0, TABLE.index()).unwrap();
    let restore = page.unit.restore.as_ref().unwrap();
    let frame = &restore.streams[0].1[0].frame;
    bytes[(frame.offset + u64::from(frame.stored) / 2) as usize] ^= 0x40;
    let recording = Recording::open(bytes).unwrap();
    let mut cache = LexicalCursor::default();
    if let Some(proof) = LexicalCursor::prepare(&recording, 0) { cache.use_anchors(proof); }
    let _ = compared(&mut cache, &recording, 0, START + BLOCK as i32 - 1);
    let target = START + BLOCK as i32;
    let exact = super::super::frame(&recording, 0, target, &[super::super::Read::all(TABLE)]).unwrap();
    assert!(matches!(exact.tables[TABLE.index()].held(), TableView::Unavailable { .. }));
    assert!(cache.read(&recording, 0, target).is_none());
    assert!(cache.0.held.is_none());
}

fn constant_recording(literal: &[u8]) -> Recording {
    let bytes = fixtures::written_at(recorder_derive::writers_within, FormatVersion::CURRENT.version(),
        b"verified lexical", &[(1, START, 2)], &[TABLE], |synthetic, _, ordinal| {
            synthetic.table(TABLE, Input::Snapshot {
                rows: &[LexicalTokenRow { token: 1, literal: literal.to_vec() }],
                partial: None, read: ordinal % 3 == 0, sample: Sample::FromReadBit,
            }).unwrap();
        }).0;
    Recording::open(bytes).unwrap()
}

#[test]
fn a_verified_checkpoint_reuses_a_forward_view_but_a_rewind_restores() {
    let recording = constant_recording(b"one");
    let mut cache = LexicalCursor::default();
    cache.use_anchors(LexicalCursor::prepare(&recording, 0).unwrap());
    let boundary = START + BLOCK as i32;
    let (before, decoded) = compared(&mut cache, &recording, 0, boundary - 24);
    assert_eq!(decoded, 1);
    let (after, decoded) = compared(&mut cache, &recording, 0, boundary);
    assert_eq!(decoded, 0);
    assert!(SharedView::ptr_eq(&before, &after));
    assert_eq!(cache.0.held.as_ref().unwrap().reader.restores(), 1);
    let (back, decoded) = compared(&mut cache, &recording, 0, boundary - 1);
    assert_eq!(decoded, 1);
    assert!(!SharedView::ptr_eq(&after, &back));
}

#[test]
fn a_decodable_checkpoint_with_different_literals_cannot_prove_reuse() {
    let recording = constant_recording(b"one");
    let different = constant_recording(b"uno");
    let mut reader = TableReader::new(&recording, 0, TABLE, None).unwrap();
    reader.seek(&recording, BLOCK - 1).unwrap();
    let mut restored = TableReader::new(&different, 0, TABLE, None).unwrap();
    restored.seek(&different, BLOCK).unwrap();
    assert_eq!(reader.cells().key(0), restored.cells().key(0));
    assert_eq!(reader.cells().value(0, 0), restored.cells().value(0, 0));
    assert!(!reader.verify_restore(&different, 0).unwrap());
}
