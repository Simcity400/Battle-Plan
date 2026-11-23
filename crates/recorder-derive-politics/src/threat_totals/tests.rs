use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};

fn feed(store: &mut Store, hour: i32, aggregates: &[(u32, i64)], status: Status) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let rows: Vec<ThreatCountryAggregateRow> =
        aggregates.iter().map(|&(country_slot, total_raw)| ThreatCountryAggregateRow { country_slot, total_raw }).collect();
    put(store, hour, TableId::ThreatCountryAggregates, &rows, status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(ThreatTotals::default())]);
    writers.start_run();
    writers
}

fn sum(store: &Store) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::ThreatTotals, 0)
}

type Hours = [(&'static [(u32, i64)], Status)];

const HOURS: &Hours = &[
    (&[(1, 400_000), (2, 250_000)], Status::Complete),
    (&[(1, 400_000), (2, 300_000), (7, -50_000)], Status::Complete),
    (&[(1, 400_000)], Status::Truncated { missing: 2 }),
    (&[(2, 1_200_000), (7, 9_000_000_000)], Status::Complete),
    (&[], Status::Complete),
];

#[test]
fn the_sum_adds_every_contribution_as_rows_come_change_and_go() {
    let mut store = store();
    let mut writers = writers();
    for (hour, &(aggregates, status)) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, aggregates, status);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::ThreatTotals.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 2, "hour {hour}");
        let expected = [650_000, 650_000, 0, 9_001_200_000, 0][hour];
        if hour != 2 {
            assert_eq!(sum(&store), Some(vec![Some(expected)]), "hour {hour}");
        }
    }
}

#[test]
fn a_contribution_not_recorded_leaves_the_sum_not_recorded() {
    let mut store = store();
    let mut writers = writers();
    feed(&mut store, 0, &[(1, 100), (2, 200)], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    store.begin_hour(1, &[]).unwrap();
    store.status(TableId::ThreatCountryAggregates.index(), Status::Complete, true, Some(1), true).unwrap();
    {
        let mut changes = store.table(TableId::ThreatCountryAggregates.index()).unwrap();
        let slot = changes.slot(2).unwrap();
        changes.unrecord(slot, TOTAL[0]).unwrap();
    }
    store.settle();
    writers.hour(1, 1, false, &mut store).unwrap();
    assert_eq!(sum(&store), Some(vec![None]));
    feed(&mut store, 2, &[(1, 100), (2, 300)], Status::Complete);
    writers.hour(1, 2, false, &mut store).unwrap();
    assert_eq!(sum(&store), Some(vec![Some(400)]));
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, &(aggregates, status)) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, aggregates, status);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::ThreatTotals));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(ThreatTotals::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in 0..HOURS.len() - 1 {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}
