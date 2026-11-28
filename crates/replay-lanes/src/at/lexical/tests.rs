use super::*;
use replay_data::{format::FormatVersion, rows::LexicalTokenRow};
use replay_journal::{fixtures, synthetic::{Input, Sample}};

const START: i32 = 100;
const RUNS: [(u64, i32, u32); 2] = [(1, START, 2), (2, START + 10_000, 1)];

fn fixture() -> Vec<u8> {
    fixtures::written_at(recorder_derive::writers_within, FormatVersion::CURRENT.version(), b"exact lexical", &RUNS, &[TABLE], |synthetic, _, ordinal| {
        let mut rows = vec![LexicalTokenRow { token: 1, literal: if ordinal < 51 { b"one" } else { b"uno" }.to_vec() }];
        if ordinal >= 51 { rows.push(LexicalTokenRow { token: 2, literal: b"two".to_vec() }); }
        let input = if ordinal == 123 {
            Input::Unavailable { reason: "lexical reader refused the sample", read: true, sample: Sample::Reset }
        } else { Input::Snapshot {
            rows: &rows, partial: (ordinal == 120).then_some(1),
            read: ordinal % 3 == 0, sample: Sample::FromReadBit,
        } };
        synthetic.table(TABLE, input).unwrap();
    }).0
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
            assert_eq!(cache.held.as_ref().unwrap().reader.restores(), 1);
        }
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
    assert!(cache.held.is_none());
    assert!(cache.read(&recording, 0, START + 123).is_none());
    assert!(cache.held.is_none());
    let (before, _) = compared(&mut cache, &recording, 0, START + BLOCK as i32 - 1);
    let (after, _) = compared(&mut cache, &recording, 0, START + BLOCK as i32);
    assert!(!SharedView::ptr_eq(&before, &after));
    assert!(cache.read(&recording, 0, START + (RUNS[0].2 * BLOCK) as i32).is_none());
    assert!(cache.held.is_none());
    let (other, _) = compared(&mut cache, &recording, 1, START + 10_000);
    assert!(!SharedView::ptr_eq(&after, &other));
    assert!(cache.read_bounded(&recording, 1, START + 10_001, 0).is_some());
    assert!(cache.held.is_none());
}

#[test]
fn the_same_recordings_projection_cannot_use_a_full_lexical_view() {
    let mut recording = Recording::open(fixture()).unwrap();
    let mut cache = LexicalCursor::default();
    let _ = compared(&mut cache, &recording, 0, START);
    recording.project(TABLE, &[]).unwrap();
    assert!(cache.read(&recording, 0, START).is_none());
    assert!(cache.held.is_none());
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
    let _ = compared(&mut cache, &recording, 0, START + BLOCK as i32 - 1);
    let target = START + BLOCK as i32;
    let exact = super::super::frame(&recording, 0, target, &[super::super::Read::all(TABLE)]).unwrap();
    assert!(matches!(exact.tables[TABLE.index()].held(), TableView::Unavailable { .. }));
    assert!(cache.read(&recording, 0, target).is_none());
    assert!(cache.held.is_none());
}
