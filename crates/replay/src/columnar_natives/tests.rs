use super::*;
use crate::{
    campaign::Parallel,
    columnar_frames::fixtures,
    format::FormatVersion,
    frame::{DecodedFrame, SharedView, TableView},
    rows::TableId,
    scope::CaptureMode,
};
use replay_columnar::envelope::UNIT_ORDINALS;
use std::sync::Arc;

const UNIT: i32 = UNIT_ORDINALS as i32;

/// Runs (branch, first hour, units): two of one branch with a block's hours between them, and
/// another branch.
const RUNS: [(u64, i32, u32); 3] = [(1, 100, 3), (1, 100 + 4 * UNIT, 2), (2, 50_000, 2)];
const TABLES: [TableId; 2] = [TableId::Countries, TableId::WorldTension];

fn alone(bytes: Vec<u8>) -> Arc<Natives> {
    Arc::new(Natives::of(Recording::open(bytes).unwrap()).unwrap())
}

/// Every hour of `runs`, in their order, with the branch it is of.
fn hours(runs: &[(u64, i32, u32)]) -> Vec<(u64, i32)> {
    runs.iter().flat_map(|&(branch, first, units)| (0..units as i32 * UNIT).map(move |at| (branch, first + at))).collect()
}

/// Each table's view and sample hour, the metrics aside: the writer's frames carry none.
fn same(frame: &DecodedFrame, written: &fixtures::Seen) -> bool {
    let sampled: Vec<Option<i32>> = frame.tables.iter().map(SharedView::sampled_at).collect();
    frame.branch == written.0.branch && frame.hour == written.0.hour && frame.tables == written.0.tables && sampled == written.1
}

/// Frames read by a cursor at every hour in order, at hours at random (some moved to without
/// building and built later) and backwards, alone and with views built on several workers, are
/// the writer's: rows, strings, statuses, reasons and sample hours. An hour no run holds is no
/// frame and no error. A damaged table is unavailable by name from its damage on, its hour
/// still a frame, and the other table reads on.
#[test]
fn frames_read_from_the_recording_are_the_writer_s_at_every_hour() {
    let (bytes, expected) = fixtures::churned(&RUNS);
    let natives = alone(bytes.clone());
    let hours = hours(&RUNS);
    assert_eq!(hours.len(), expected.len());
    let on_worker = || {};
    let parallel = Parallel { workers: 3, on_worker: &on_worker };
    for parallel in [None, Some(&parallel)] {
        let mut cursor = Cursor::new(Arc::clone(&natives), &TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        assert_eq!(cursor.tables(), &TABLES);
        let check = |at: usize, cursor: &mut Cursor| {
            let (branch, hour) = hours[at];
            let frame = cursor.frame(branch, branch, hour, parallel).unwrap().unwrap();
            assert!(same(&frame, &expected[at]), "branch {branch} hour {hour}");
            let complete = matches!(&*frame.tables[TableId::WorldTension.index()], TableView::Complete(_));
            assert_eq!(cursor.complete(TableId::WorldTension).unwrap(), complete, "branch {branch} hour {hour}");
        };
        for at in 0..hours.len() {
            check(at, &mut cursor);
        }
        let mut seed = 9_u64;
        for step in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let at = (seed >> 33) as usize % hours.len();
            if step % 3 == 0 {
                // A move taken without a frame is built into the next one.
                let (branch, hour) = hours[(at + 1) % hours.len()];
                assert!(cursor.move_to(branch, branch, hour, &|| false).unwrap());
            }
            if step % 5 == 0 {
                // A move stopped before it starts leaves the cursor to be moved again.
                let (branch, hour) = hours[at];
                let error = cursor.move_to(branch, branch, hour, &|| true).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
            }
            check(at, &mut cursor);
        }
        for at in (0..hours.len()).rev().step_by(7) {
            check(at, &mut cursor);
        }
        for (branch, hour) in [(1, 99), (1, 100 + 3 * UNIT), (1, 100 + 4 * UNIT - 1), (1, 100 + 6 * UNIT), (2, 49_999), (3, 100)] {
            assert!(cursor.frame(branch, branch, hour, parallel).unwrap().is_none(), "branch {branch} hour {hour}");
            assert!(!cursor.move_to(branch, branch, hour, &|| false).unwrap(), "branch {branch} hour {hour}");
        }
    }

    let clean = Recording::open(bytes.clone()).unwrap();
    let entry = clean.table_entry(0, 1, TableId::Countries.index()).unwrap();
    let part = &entry.unit.parts[0].frames[0];
    let mut damaged = bytes;
    damaged[(part.offset + u64::from(part.stored) / 2) as usize] ^= 0x40;
    let mut cursor = Cursor::new(alone(damaged), &TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let first_run = 3 * UNIT_ORDINALS as usize;
    for at in (0..hours.len()).step_by(13).chain([UNIT_ORDINALS as usize - 1, UNIT_ORDINALS as usize, first_run]) {
        let (branch, hour) = hours[at];
        let frame = cursor.frame(branch, branch, hour, None).unwrap().unwrap();
        let tension = TableId::WorldTension.index();
        assert_eq!(frame.tables[tension], expected[at].0.tables[tension], "hour {hour}'s world tension");
        let countries = &*frame.tables[TableId::Countries.index()];
        if at < UNIT_ORDINALS as usize || at >= first_run {
            assert_eq!(countries, &*expected[at].0.tables[TableId::Countries.index()], "hour {hour}");
        } else {
            let TableView::Unavailable { reason } = countries else { panic!("hour {hour}'s countries read past their damage") };
            assert!(reason.contains("Countries of run 0 in block 1"), "{reason}");
            assert!(!cursor.complete(TableId::Countries).unwrap());
        }
    }
}

/// The map's cursor moved forward a day at a time, across block ends and from one run to the
/// next, builds at every stop the writer's frame and the one a cursor moved there alone builds.
#[test]
fn a_cursor_moved_forward_by_days_builds_what_a_seek_builds() {
    let (bytes, expected) = fixtures::churned(&RUNS);
    let natives = alone(bytes);
    let hours = hours(&RUNS);
    let mut cursor = Cursor::new(Arc::clone(&natives), &TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    for at in (3..hours.len()).step_by(24) {
        let (branch, hour) = hours[at];
        let frame = cursor.frame(branch, branch, hour, None).unwrap().unwrap();
        assert!(same(&frame, &expected[at]), "branch {branch} hour {hour}");
        let mut fresh = Cursor::new(Arc::clone(&natives), &TABLES, FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        let sought = fresh.frame(branch, branch, hour, None).unwrap().unwrap();
        assert_eq!(frame.tables, sought.tables, "branch {branch} hour {hour}");
    }
}
