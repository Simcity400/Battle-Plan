use super::*;
use crate::columnar_frames::Pool;
use replay_frame::frame::{DecodedFrame, HourInput, SharedView, TableChanges, TableView};
use replay_data::{rows::{CountryRow, DivisionRow, DivisionRowAt141, Recorded, StateRow, StateRowAt141, TableId, TableRows}, scope::CaptureMode};
use replay_journal::{fixtures, synthetic::{Input, Sample, Synthetic}};
use replay_columnar::envelope::{self, UNIT_ORDINALS};
use replay_data::rows::compatibility::CAPTURE_ADDITIONS_FORMAT;
use std::sync::Arc;

const UNIT: i32 = UNIT_ORDINALS as i32;

const RUNS: [(u64, i32, u32); 3] = [(1, 100, 3), (1, 100 + 4 * UNIT, 2), (2, 50_000, 2)];
const TABLES: [TableId; 2] = [TableId::Countries, TableId::WorldTension];

fn alone(bytes: Vec<u8>) -> Arc<Natives> {
    Arc::new(Natives::of(Recording::open(bytes).unwrap()).unwrap())
}

fn hours(runs: &[(u64, i32, u32)]) -> Vec<(u64, i32)> {
    runs.iter().flat_map(|&(branch, first, units)| (0..units as i32 * UNIT).map(move |at| (branch, first + at))).collect()
}

fn same(frame: &DecodedFrame, written: &fixtures::Seen) -> bool {
    let sampled: Vec<Option<i32>> = frame.tables.iter().map(SharedView::sampled_at).collect();
    frame.branch == written.0.branch && frame.hour == written.0.hour && frame.tables == written.0.tables && sampled == written.1
}

#[test]
fn frames_read_from_the_recording_are_the_writer_s_at_every_hour() {
    let (bytes, expected) = fixtures::churned(recorder_derive::writers(), &RUNS);
    let natives = alone(bytes.clone());
    let hours = hours(&RUNS);
    assert_eq!(hours.len(), expected.len());
    let pool = Arc::new(
        Pool::new(3, "cursor worker", || {
            assert!(
                std::thread::current().name().is_some_and(|name| name.starts_with("cursor worker ")),
                "unexpected worker name {:?}",
                std::thread::current().name()
            );
        })
        .unwrap(),
    );
    for use_pool in [false, true] {
        let mut cursor = Cursor::new(Arc::clone(&natives), &TABLES).unwrap();
        if use_pool {
            cursor.set_pool(Some(Arc::clone(&pool)));
        }
        assert_eq!(cursor.tables(), &TABLES);
        let before = pool.entered();
        assert_eq!(cursor.build().unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(pool.entered(), before, "build before any move does not enter the pool");
        let check = |at: usize, cursor: &mut Cursor| {
            let (branch, hour) = hours[at];
            let before = pool.entered();
            let frame = cursor.frame(branch, branch, hour).unwrap().unwrap();
            assert_eq!(pool.entered() - before, if use_pool { 2 } else { 0 }, "branch {branch} hour {hour}");
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
                let (branch, hour) = hours[(at + 1) % hours.len()];
                let before = pool.entered();
                assert!(cursor.move_to(branch, branch, hour, &|| false).unwrap());
                assert_eq!(pool.entered() - before, usize::from(use_pool));
            }
            if step % 5 == 0 {
                let (branch, hour) = hours[at];
                let before = pool.entered();
                let error = cursor.move_to(branch, branch, hour, &|| true).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
                assert_eq!(pool.entered(), before, "a cancelled move does not enter the pool");
            }
            check(at, &mut cursor);
        }
        for at in (0..hours.len()).rev().step_by(7) {
            check(at, &mut cursor);
        }
        for (branch, hour) in [(1, 99), (1, 100 + 3 * UNIT), (1, 100 + 4 * UNIT - 1), (1, 100 + 6 * UNIT), (2, 49_999), (3, 100)] {
            let before = pool.entered();
            assert!(cursor.frame(branch, branch, hour).unwrap().is_none(), "branch {branch} hour {hour}");
            assert!(!cursor.move_to(branch, branch, hour, &|| false).unwrap(), "branch {branch} hour {hour}");
            assert_eq!(pool.entered(), before, "missing hour does not enter the pool");
        }
        if use_pool {
            cursor.set_pool(None);
            let before = pool.entered();
            let (branch, hour) = hours[0];
            assert!(same(&cursor.frame(branch, branch, hour).unwrap().unwrap(), &expected[0]));
            assert_eq!(pool.entered(), before, "clearing the pool stops scheduling on it");
        }
    }

    let clean = Recording::open(bytes.clone()).unwrap();
    let entry = clean.table_entry(0, 1, TableId::Countries.index()).unwrap();
    let part = &entry.unit.parts[0].frames[0];
    let mut damaged = bytes;
    damaged[(part.offset + u64::from(part.stored) / 2) as usize] ^= 0x40;
    let mut cursor = Cursor::new(alone(damaged), &TABLES).unwrap();
    let first_run = 3 * UNIT_ORDINALS as usize;
    for at in (0..hours.len()).step_by(13).chain([UNIT_ORDINALS as usize - 1, UNIT_ORDINALS as usize, first_run]) {
        let (branch, hour) = hours[at];
        let frame = cursor.frame(branch, branch, hour).unwrap().unwrap();
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

#[test]
fn a_cursor_moved_forward_by_days_builds_what_a_seek_builds() {
    let (bytes, expected) = fixtures::churned(recorder_derive::writers(), &RUNS);
    let natives = alone(bytes);
    let hours = hours(&RUNS);
    let mut cursor = Cursor::new(Arc::clone(&natives), &TABLES).unwrap();
    for at in (3..hours.len()).step_by(24) {
        let (branch, hour) = hours[at];
        let frame = cursor.frame(branch, branch, hour).unwrap().unwrap();
        assert!(same(&frame, &expected[at]), "branch {branch} hour {hour}");
        let mut fresh = Cursor::new(Arc::clone(&natives), &TABLES).unwrap();
        let sought = fresh.frame(branch, branch, hour).unwrap().unwrap();
        assert_eq!(frame.tables, sought.tables, "branch {branch} hour {hour}");
    }
}

fn mode_recording(capture_mode: u8) -> Vec<u8> {
    let mut file = fixtures::writer(recorder_derive::writers(), fixtures::identity(b"cursor mode", capture_mode), |_| {});
    file.start_run(1, envelope::epoch::START, 100).unwrap();
    let mut synthetic = Synthetic::new(1, 100).unwrap();
    for ordinal in 0..2 {
        let rows = [CountryRow { id: 7, tag: b"GER".to_vec(), stability_raw: 10 + i64::from(ordinal), ..Default::default() }];
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections)).unwrap().unwrap();
    }
    file.finish().unwrap().finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner()
}

#[test]
fn a_cursor_reads_format_and_capture_mode_from_the_recording_metadata() {
    let format = CAPTURE_ADDITIONS_FORMAT - 1;
    let added = [TableId::LandUnitOrder, TableId::StrategicRegionWeights, TableId::StateBuildingInputs, TableId::CombatEngagementRefreshes, TableId::LandCombatEdges, TableId::FuelAwardRecipients, TableId::ScriptVariableNames, TableId::ScriptVariableValues, TableId::LandCombatResultObservers];
    let (bytes, _) = fixtures::written_at(recorder_derive::writers_within, format, b"cursor format 141", &[(1, 100, 1)], &[TableId::States, TableId::Divisions], |synthetic, _, ordinal| {
        let states = [StateRowAt141 { id: 4, owner: 7 + ordinal, controller: 9, custom_name: b"Bohemia".to_vec(), continent: Some(2).into(), ..Default::default() }];
        let divisions = [DivisionRowAt141 { id: (3_u64 << 32) | 1, roster_owner: 7, province: 11 + ordinal, hit_points: 500_000, organisation: 250_000, ..Default::default() }];
        synthetic.table(TableId::States, Input::Snapshot { rows: &states, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let mut tables = vec![TableId::States, TableId::Divisions];
    tables.extend(added);
    let mut cursor = Cursor::new(alone(bytes), &tables).unwrap().with_change_facts();
    let first = cursor.frame(1, 1, 100).unwrap().unwrap();
    let second = cursor.frame(1, 1, 101).unwrap().unwrap();
    for (ordinal, frame) in [(0_u32, &first), (1_u32, &second)] {
        assert_eq!(
            *frame.tables[TableId::States.index()],
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 4,
                owner: 7 + ordinal,
                controller: 9,
                custom_name: b"Bohemia".to_vec(),
                continent: Some(2).into(),
                marked_owners: Recorded::NotRecorded,
                ..Default::default()
            }]))
        );
        assert_eq!(
            *frame.tables[TableId::Divisions.index()],
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: (3_u64 << 32) | 1,
                roster_owner: 7,
                province: 11 + ordinal,
                hit_points: 500_000,
                organisation: 250_000,
                theatre: Recorded::NotRecorded,
                ..Default::default()
            }]))
        );
        for table in added {
            assert_eq!(*frame.tables[table.index()], TableView::NotRecorded { mode: CaptureMode::Full }, "{table:?}");
        }
    }
    for table in added {
        assert!(!cursor.complete(table).unwrap(), "{table:?}");
    }
    let marked_owners_bit = 1_u64 << TableId::States.last_change().fields(TableId::States, format);
    assert_eq!(TableId::States.field_name(TableId::States.last_change().fields(TableId::States, format)), Some("marked_owners"));
    let TableChanges::Rows(facts) = HourInput::new(&second).changes().since(TableId::States, Some(&first.tables[TableId::States.index()].identity()), u64::MAX) else {
        panic!("stepped earlier-format frame carries change facts");
    };
    assert_eq!(facts.changed().len(), 1);
    assert_ne!(facts.changed()[0].fields & marked_owners_bit, 0, "unrecorded fields after the recording's format are marked");

    let map_natives = alone(mode_recording(CaptureMode::Map.byte()));
    let mut map_cursor = Cursor::new(map_natives, &TABLES).unwrap();
    let frame = map_cursor.frame(1, 1, 100).unwrap().unwrap();
    assert!(matches!(&*frame.tables[TableId::Countries.index()], TableView::Complete(_)));
    assert_eq!(*frame.tables[TableId::WorldTension.index()], TableView::NotRecorded { mode: CaptureMode::Map });
    assert!(!map_cursor.complete(TableId::WorldTension).unwrap());

    let bad_mode = alone(mode_recording(99));
    let error = Cursor::new(bad_mode, &[TableId::Countries]).err().expect("invalid capture mode is refused");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}
