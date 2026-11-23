use super::{Checked, branch_id, differ, differ_rows, log_rows, log_table, played, read_rows};
use crate::{
    Piece, Timeline,
    changes::Changes,
    lane::{
        Lane,
        log::tests::{LOG, record_log},
        tests::{Memory, Recording},
    },
    pages::{Pages, Run, Span, WorldPages},
};
use replay::campaign::{Replay, timelines};
use replay_capture_spool::spool::Record;
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableState},
    unit::{BLOCK, Encoder, Settings},
};
use replay_data::{
    BranchId, Hour,
    identity::{CaptureMode, Identity, SampleBoundary, SessionStart, Verification},
    rows::TableId,
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{Frame, Metrics, TableUpdate, TableView, current_absence},
    rows::{CampaignRow, ClockRow, TableRows},
};
use std::{io, sync::Arc};

fn one_row(value: i64, unrecorded: bool) -> TableState {
    let mut store = Store::new(vec![vec![Kind::I64]], vec![false]);
    store.begin_hour(0, &[]).unwrap();
    let mut table = store.table(0).unwrap();
    table.insert_at(0, 1).unwrap();
    table.write(0, 0, Cell::I64(value)).unwrap();
    if unrecorded {
        table.unrecord(0, 0).unwrap();
    }
    store.table_state(0).unwrap().clone()
}

#[test]
fn two_unrecorded_cells_compare_equal_whatever_they_held_and_an_unrecorded_cell_differs_from_a_recorded_one() {
    let (five, seven, recorded) = (one_row(5, true), one_row(7, true), one_row(5, false));
    assert_ne!(five.column(0).unwrap().held(0), seven.column(0).unwrap().held(0), "the cells hold other values under their unrecorded marks");
    let mut first = None;
    assert_eq!(differ(&five, &seven, &[0], &mut first), 0, "two unrecorded cells are the same missing reading: {first:?}");
    assert_eq!(differ(&five, &recorded, &[0], &mut None), 1, "an unrecorded cell is never a recorded one");
    assert_eq!(differ(&recorded, &five, &[0], &mut None), 1, "nor a recorded cell an unrecorded one");
}

fn logged(hours: u32) -> (Arc<dyn Pages>, Vec<(Hour, bool, u64)>) {
    let (bytes, run, done) = record_log(Settings { cap: 48, ..Settings::at(3) }, 0, hours);
    (Arc::new(Recording { bytes, runs: vec![run], kinds: vec![LOG.to_vec()] }), done)
}

fn passed(checked: &Checked) {
    assert_eq!([checked.a, checked.b, checked.refused, checked.c, checked.d], [0; 5], "{checked:?}");
}

fn events(done: &[(Hour, bool, u64)], (start, end): (u32, u32)) -> u64 {
    done.iter().filter(|(hour, ..)| (start as Hour + 1..=end as Hour).contains(hour)).count() as u64
}

#[test]
fn a_log_table_is_checked_against_the_log_reader_through_every_block_and_the_window_end() {
    let hours = 2 * BLOCK + 50;
    let (pages, done) = logged(hours);
    for window in [(0, hours - 1), (10, 300), (0, BLOCK + 100), (BLOCK - 1, BLOCK + 99)] {
        let checked = log_table(&pages, 0, window, 24, 8).unwrap_or_else(|error| panic!("{window:?}: a log table is refused: {error}"));
        passed(&checked);
        let wanted = events(&done, window);
        assert!(wanted > 0, "{window:?}: the window holds rows");
        assert_eq!(checked.read_rows, wanted, "{window:?}: every appended and forgotten row of the window is compared with the reader: {checked:?}");
        assert_eq!(checked.narrow_rows, [wanted; 2], "{window:?}: and with each lane of fewer columns, the lane of none too: {checked:?}");
    }
}

#[test]
fn a_log_window_of_one_block_end_hour_checks_that_blocks_restore() {
    let (pages, _) = logged(2 * BLOCK + 50);
    let checked = log_table(&pages, 0, (BLOCK - 1, BLOCK - 1), 24, 8).unwrap();
    passed(&checked);
    assert_eq!(checked.block_ends, 1, "{checked:?}");
}

#[test]
fn a_log_row_handed_without_its_key_differs_from_the_row_the_reader_keys_zero() {
    let kinds = vec![vec![Kind::I64]];
    let mut store = Store::new(kinds.clone(), vec![false]);
    store.declare_logs(&[true]).unwrap();
    store.attach(Encoder::new(&kinds, Settings::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for hour in 0..40 {
        store.begin_hour(hour, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(hour), true).unwrap();
        store.table(0).unwrap().append(0, &[Some(Cell::I64(i64::from(hour)))]).unwrap();
        store.end_hour(&mut memory).unwrap();
    }
    store.end_run(&mut memory).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds });
    let handed = |keyed: bool| {
        let mut lane = Lane::new(Arc::clone(&pages), 0, 0, &[0], keyed).unwrap();
        lane.seek(0).unwrap();
        lane.fetch_through(39).unwrap();
        let (mut changes, mut rows) = (Changes::default(), Vec::new());
        lane.step(39, &mut changes).unwrap();
        log_rows(&changes, &[0], &mut rows);
        rows
    };
    let wanted = read_rows(&*pages, 0, &[0], 1..=39).unwrap();
    assert_eq!(wanted.len(), 39);
    assert_eq!(differ_rows(handed(true), wanted.clone(), "keyed", &mut None), 0, "a keyed lane hands every key, zero too");
    assert_eq!(differ_rows(handed(false), wanted, "unkeyed", &mut None), 78, "a row without its key is not the row keyed zero");
}

#[test]
fn a_branch_id_is_read_whole_and_exact() {
    assert_eq!(branch_id("18446744073709551615").unwrap(), u64::MAX);
    assert_eq!(branch_id("2").unwrap(), 2);
    for refused in ["1.9", "-1", "two", ""] {
        let error = branch_id(refused).err().unwrap_or_else(|| panic!("{refused:?} is taken as a branch"));
        assert!(error.to_string().contains("--branch takes a branch id"), "{error}");
    }
}

const CAMPAIGN: &[u8] = b"bench branches";

fn frame(branch: BranchId, hour: Hour) -> Frame {
    Frame {
        branch,
        hour,
        keyframe: true,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            let rows = match TableId::ALL[index] {
                TableId::Clock => TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]),
                TableId::Campaign => TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.to_vec() }]),
                table => TableRows::empty(table),
            };
            match current_absence(rows.table()) {
                Some(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
                Some(TableView::Unavailable { reason }) => TableUpdate::Unavailable { reason },
                None => TableUpdate::Snapshot(rows),
                _ => unreachable!("absence is an omission or a named refusal"),
            }
        }),
    }
}

fn segment(id: BranchId, hours: std::ops::RangeInclusive<Hour>) -> Vec<Record> {
    let session = SessionStart {
        id,
        process_id: id as u32,
        process_start_time: id,
        qpc_frequency: 100,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    };
    let first = *hours.start();
    let branch = BranchStart { id, session: id, loaded_hour: Some(first - 1), first_hour: first, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let mut records = vec![Record::SessionStart(session), Record::BranchStart(branch)];
    records.extend(hours.map(|hour| Record::Frame(frame(id, hour))));
    records
}

fn reloaded() -> (Replay, WorldPages) {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.to_vec(), &[segment(1, 1_000..=1_005), segment(2, 1_003..=1_006)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages = WorldPages::over(replay.recording().selecting(&TableId::ALL).unwrap(), Span::Whole).unwrap();
    (replay, pages)
}

#[test]
fn a_recording_loaded_back_in_time_plays_its_branch_timeline_not_its_overlapping_runs() {
    let (replay, pages) = reloaded();
    assert!(pages.pieces()[1].first <= pages.pieces()[0].last, "the runs overlap: {:?}", pages.pieces());
    let main = played(&replay, &pages, Span::Whole, None).unwrap_or_else(|error| panic!("the main branch is refused: {error}"));
    assert_eq!(main.pieces(), [Piece { run: 0, first: 1_000, last: 1_002 }, Piece { run: 1, first: 1_003, last: 1_006 }]);
    let first = played(&replay, &pages, Span::Whole, Some(1)).unwrap();
    assert_eq!(first.pieces(), [Piece { run: 0, first: 1_000, last: 1_005 }]);
    let refused = played(&replay, &pages, Span::Whole, Some(9)).err().expect("a branch the recording does not hold");
    assert!(refused.to_string().contains("no branch 9"), "{refused}");
}

#[test]
fn a_branch_holding_no_readable_hour_is_refused_as_before() {
    let (_, pages) = reloaded();
    let elsewhere = [timelines::Piece { segment: 7, first_hour: 1_000, last_hour: 1_006, declared: 1_000 }];
    let refused = Timeline::branch(&pages, pages.branches(), 7, &elsewhere).err().expect("no run holds segment 7");
    assert_eq!(refused.kind(), io::ErrorKind::Other, "{refused}");
    assert_eq!(refused.to_string(), "Branch 7 holds no readable hour of the recording");
}
