use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::{Recorded, RowCodec};

const TEXTS: &[&[u8]] = &[b"AAA", b"tech_a", b"tech_b", b"BBB"];
const ACTIVE_CLASSIFIER: u32 = 5;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Research::default())]);
    writers.start_run();
    writers
}

struct World {
    slots: Vec<(u8, usize, u32, i32, i32)>,
    inventory: Vec<(i32, u32, i32, u8)>,
    read: bool,
    complete: bool,
}

fn sync<R: RowCodec>(store: &mut Store, table: TableId, rows: &[R]) {
    let keep: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (key, _) in testing::rows(store, table) {
        if !keep.contains(&key) {
            testing::remove(store, table, key).unwrap();
        }
    }
    for row in rows {
        testing::put(store, table, row, TEXTS).unwrap();
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, world: &World) {
    run_tagged(store, writers, hour, world, 0);
}

fn run_tagged(store: &mut Store, writers: &mut Writers, hour: i32, world: &World, tag: usize) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for read in READS {
        let status = if read.table == TableId::ResearchInputs && !world.complete { Status::Unavailable { reason: 0 } } else { Status::Complete };
        store.status(read.table.index(), status, true, Some(hour), true).unwrap();
    }
    sync(store, TableId::Countries, &[CountryRow { id: 1, canonical_slot: 1, tag: TEXTS[tag].to_vec(), ..Default::default() }]);
    sync(store, TableId::CountryLinks, &[CountryLinkRow { id: 1, canonical_slot: 1 }]);
    if world.read {
        let inputs: Vec<ResearchInputRow> = world
            .slots
            .iter()
            .enumerate()
            .map(|(index, &(present, token, classifier, level, occurrence))| ResearchInputRow {
                country: 1,
                index: index as u32,
                technology_handle: occurrence as u32 + 100,
                technology_present: present,
                token: TEXTS[token].to_vec(),
                classifier,
                level,
                progress_raw: i64::from(hour),
                database_index: Recorded::Value(occurrence),
                ..Default::default()
            })
            .collect();
        sync(store, TableId::ResearchInputs, &inputs);
        sync(store, TableId::SliceReads, &[SliceReadRow { table: INPUTS, owner: 1, sample_hour: hour }]);
    }
    let inventory: Vec<ResearchTechnologyRow> = world
        .inventory
        .iter()
        .map(|&(database_index, classifier, level, researched)| ResearchTechnologyRow { country: 1, technology_handle: database_index as u32 + 100, database_index, classifier, level, researched, ..Default::default() })
        .collect();
    sync(store, TableId::ResearchTechnologies, &inventory);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn switching(hour: i32) -> World {
    let slots = if hour < 10 { vec![(1, 1, ACTIVE_CLASSIFIER, 0, 7), (0, 0, 0, 0, 0)] } else { vec![(1, 2, ACTIVE_CLASSIFIER, 0, 8), (0, 0, 0, 0, 0)] };
    let level = if hour < 15 { 0 } else { 1 };
    World { slots, inventory: vec![(7, ACTIVE_CLASSIFIER, level, 0), (8, ACTIVE_CLASSIFIER, 0, 0)], read: matches!(hour, 0 | 10 | 15 | 45), complete: true }
}

type Row = (u32, u16, i32, i32, i32, Option<i32>, Option<i32>, Option<i32>, u8, Option<i32>, u8);

fn assignments(store: &Store) -> Vec<Row> {
    let view = store.view(TableId::ResearchAssignments.index());
    let mut rows: Vec<Row> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            let number = |at: usize| view.number(ASSIGNMENT[at], slot);
            let cell = |at: usize| number(at).unwrap();
            let hour = |at: usize| number(at).map(|hour| hour as i32);
            ((key >> 16) as u32, key as u16, cell(1) as i32, cell(4) as i32, cell(5) as i32, hour(6), hour(7), hour(8), cell(9) as u8, hour(10), cell(11) as u8)
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn coverage(store: &Store) -> Vec<(u32, i32)> {
    let view = store.view(TableId::ResearchCoverage.index());
    let mut rows: Vec<(u32, i32)> = view.rows().map(|slot| (view.key(slot).unwrap() as u32, view.number(SINCE[0], slot).unwrap() as i32)).collect();
    rows.sort_unstable();
    rows
}

const HOLDER: u32 = ResearchCoverageRow::HOLDER;

#[test]
fn an_assignment_ends_switched_awaits_its_completion_in_a_lane_of_its_own_and_a_late_read_breaks_continuity() {
    let (mut store, mut writers) = (testing::store(), writers());
    let (mut seen, mut covered) = (Vec::new(), Vec::new());
    for at in 0..=46 {
        run(&mut store, &mut writers, at, &switching(at));
        seen.push(assignments(&store));
        covered.push(coverage(&store));
    }
    let (active, completed, switched, unknown) = (ResearchAssignmentRow::ACTIVE, ResearchAssignmentRow::COMPLETED, ResearchAssignmentRow::SWITCHED, ResearchAssignmentRow::UNKNOWN);
    assert_eq!(seen[0], [(0, 0, 7, 0, 0, None, None, None, active, None, 0)], "the run's first read starts an assignment after nothing");
    assert_eq!(seen[9], seen[0], "hours between reads change nothing");
    assert_eq!(
        seen[10],
        [(0, 0, 8, 0, 10, Some(0), None, None, active, None, 0), (0, 1, 7, 0, 0, None, Some(0), Some(10), switched, None, 1)],
        "the switched assignment awaits evidence in lane 1; the new one starts after the read before"
    );
    assert_eq!(
        seen[15],
        [(0, 0, 8, 0, 10, Some(0), None, None, active, None, 0), (0, 1, 7, 0, 0, None, Some(0), Some(10), completed, Some(15), 0)],
        "the inventory's risen level proves it completed at the next read"
    );
    assert_eq!(seen[16], [(0, 0, 8, 0, 10, Some(0), None, None, active, None, 0)], "a wait ended in another lane goes the hour after");
    assert_eq!(
        seen[45],
        [(0, 0, 8, 45, 45, None, None, None, active, None, 0), (0, 1, 8, 0, 10, Some(0), Some(15), Some(45), unknown, None, 0)],
        "a read more than a period after the cover ended is not continuous: the active assignment ends unknown, and the slot's starts afresh"
    );
    assert_eq!(seen[46], [(0, 0, 8, 45, 45, None, None, None, active, None, 0)]);
    let read_at = |since: i32| vec![(0, since), (1, since), (HOLDER, since)];
    assert_eq!(covered[0], read_at(0));
    assert_eq!(covered[38], read_at(0), "a read covers through the hours before the next is due, and continuous reads extend it");
    assert_eq!(covered[39], Vec::<(u32, i32)>::new(), "no read due within a period: the cover ends");
    assert_eq!(covered[45], read_at(45));
}

#[test]
fn a_holder_closed_and_started_again_in_one_hour_moves_its_ended_assignment_to_a_lane_of_its_own() {
    let (mut store, mut writers) = (testing::store(), writers());
    let world = |hour: i32| World { slots: vec![(1, 1, ACTIVE_CLASSIFIER, 0, 7)], inventory: vec![(7, ACTIVE_CLASSIFIER, 0, 0)], read: hour < 2, complete: true };
    run(&mut store, &mut writers, 0, &world(0));
    run_tagged(&mut store, &mut writers, 1, &world(1), 3);
    assert_eq!(store.view(TableId::ResearchAssignments.index()).status(), Status::Complete);
    let rows = assignments(&store);
    let lanes: Vec<(u16, i32, u8, Option<i32>)> = rows.iter().map(|row| (row.1, row.2, row.8, row.7)).collect();
    assert_eq!(lanes, [(0, 7, ResearchAssignmentRow::ACTIVE, None), (1, 7, ResearchAssignmentRow::UNKNOWN, Some(1))]);
    assert_eq!(rows[0].4, 1, "the new holder's assignment is first observed at the hour");
    run_tagged(&mut store, &mut writers, 2, &world(2), 3);
    assert_eq!(assignments(&store).iter().map(|row| row.1).collect::<Vec<_>>(), [0], "the ended one's lane goes the hour after");
}

#[test]
fn an_hour_a_table_is_not_complete_ends_the_cover_and_continuity() {
    let (mut store, mut writers) = (testing::store(), writers());
    let world = |hour: i32| World { slots: vec![(1, 1, ACTIVE_CLASSIFIER, 0, 7)], inventory: vec![(7, ACTIVE_CLASSIFIER, 0, 0)], read: matches!(hour, 0 | 10), complete: hour != 5 };
    let mut covered = Vec::new();
    for at in 0..=10 {
        run(&mut store, &mut writers, at, &world(at));
        covered.push(coverage(&store));
    }
    assert_eq!(covered[4], [(0, 0), (HOLDER, 0)]);
    assert_eq!(covered[5], Vec::<(u32, i32)>::new(), "ResearchInputs not complete: nothing is covered");
    assert_eq!(covered[10], [(0, 10), (HOLDER, 10)]);
    let (active, unknown) = (ResearchAssignmentRow::ACTIVE, ResearchAssignmentRow::UNKNOWN);
    assert_eq!(
        assignments(&store),
        [(0, 0, 7, 10, 10, None, None, None, active, None, 0), (0, 1, 7, 0, 0, None, Some(0), Some(10), unknown, None, 0)],
        "an unobservable hour between two reads breaks continuity: the assignment ends unknown and starts again"
    );
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for at in 0..=46 {
        run(&mut unbroken, &mut writing, at, &switching(at));
        expected.push((testing::rows(&unbroken, TableId::ResearchAssignments), testing::rows(&unbroken, TableId::ResearchCoverage)));
    }
    for stop in [12, 15] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for at in 0..=stop {
            run(&mut stopped, &mut before, at, &switching(at));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for at in stop + 1..=46 {
            run(&mut resumed, &mut after, at, &switching(at));
            let got = (testing::rows(&resumed, TableId::ResearchAssignments), testing::rows(&resumed, TableId::ResearchCoverage));
            assert_eq!(got, expected[at as usize], "stopped at {stop}, hour {at}");
        }
    }
}
