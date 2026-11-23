use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::CountryRow;

const ONE: i64 = 100_000_000;

fn feed(store: &mut Store, hour: i32, countries: &[u32], nukes: &[(u32, u32, i64, i32)], status: Status) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = countries.iter().map(|&id| CountryRow { id, canonical_slot: id, ..Default::default() }).collect();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let nukes: Vec<NukeRow> = nukes.iter().map(|&(country, nuke_type, amount_raw, ready)| NukeRow { country, nuke_type, amount_raw, ready }).collect();
    put(store, hour, TableId::Nukes, &nukes, status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(NukeTotals::default())]);
    writers.start_run();
    writers
}

fn totals(store: &Store, country: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::CountryNukeTotals, u128::from(country))
}

type Hours = [(&'static [u32], &'static [(u32, u32, i64, i32)], Status)];

const HOURS: &Hours = &[
    (&[1, 2, 3], &[(1, 0, 3 * ONE + ONE / 2, 1), (1, 1, 2 * ONE, 0), (2, 0, ONE, 2)], Status::Complete),
    (&[1, 2, 3, 4], &[(1, 0, 3 * ONE + ONE / 2, 1), (1, 1, 4 * ONE, 1)], Status::Complete),
    (&[1, 2, 3, 4], &[(1, 0, 3 * ONE, 1)], Status::Truncated { missing: 1 }),
    (&[1, 2, 4], &[(1, 0, 3 * ONE + ONE / 2, 1), (1, 1, 4 * ONE, 1), (4, 0, i64::MIN, 0), (5, 0, ONE, 0)], Status::Complete),
];

#[test]
fn a_countrys_total_adds_its_types_as_the_top_bar_does_and_a_country_without_one_holds_zero() {
    let mut store = store();
    let mut writers = writers();
    for (hour, &(countries, nukes, status)) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, countries, nukes, status);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::CountryNukeTotals.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 2, "hour {hour}");
        let expected: [Option<Vec<Option<i64>>>; 5] = match hour {
            0 => [Some(vec![Some(5), Some(1)]), Some(vec![Some(1), Some(2)]), Some(vec![Some(0), Some(0)]), None, None],
            1 | 2 => [Some(vec![Some(7), Some(2)]), Some(vec![Some(0), Some(0)]), Some(vec![Some(0), Some(0)]), Some(vec![Some(0), Some(0)]), None],
            _ => [Some(vec![Some(7), Some(2)]), Some(vec![Some(0), Some(0)]), None, Some(vec![Some(0xa7c5), Some(0)]), Some(vec![Some(1), Some(0)])],
        };
        assert_eq!([1, 2, 3, 4, 5].map(|country| totals(&store, country)), expected, "hour {hour}");
    }
}

#[test]
fn a_stock_cell_not_recorded_leaves_its_countrys_total_not_recorded() {
    let mut store = store();
    let mut writers = writers();
    feed(&mut store, 0, &[1], &[(1, 0, 2 * ONE, 0)], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    store.begin_hour(1, &[]).unwrap();
    store.status(TableId::Nukes.index(), Status::Complete, true, Some(1), true).unwrap();
    recorder_derive_framework::testing::cells(&mut store, TableId::Nukes, 1 << 32 | 1, &[]).unwrap();
    store.settle();
    writers.hour(1, 1, false, &mut store).unwrap();
    assert_eq!(totals(&store, 1), Some(vec![None, None]));
    feed(&mut store, 2, &[1], &[(1, 0, 2 * ONE, 0), (1, 1, ONE, 3)], Status::Complete);
    writers.hour(1, 2, false, &mut store).unwrap();
    assert_eq!(totals(&store, 1), Some(vec![Some(3), Some(3)]));
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, &(countries, nukes, status)) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, countries, nukes, status);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::CountryNukeTotals));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(NukeTotals::default())]);
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
