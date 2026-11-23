use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::store::Store;
use replay_data::rows::{CasualtyTotalRow, CountryLinkRow, CountryRow};
use std::collections::BTreeMap;

#[derive(Clone)]
struct World {
    countries: Vec<(u32, u32)>,
    links: Vec<(u32, u32)>,
    relations: Vec<(u32, u32, i32, [i64; 4])>,
    status: Status,
}

fn world(relations: &[(u32, u32, i32, [i64; 4])]) -> World {
    World {
        countries: vec![(10, 1), (20, 2), (30, 3)],
        links: vec![(10, 1), (20, 2), (30, 3)],
        relations: relations.to_vec(),
        status: Status::Complete,
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = world.countries.iter().map(|&(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    let relations: Vec<WarRelationRow> = world
        .relations
        .iter()
        .map(|&(first, second, start, [a, b, c, d])| WarRelationRow {
            first_country: first,
            second_country: second,
            start_hour: start,
            first_casualties: a,
            second_casualties: b,
            first_unknown_casualties: c,
            second_unknown_casualties: d,
            ..Default::default()
        })
        .collect();
    put(store, hour, TableId::WarRelations, &relations, world.status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Casualties::default())]);
    writers.start_run();
    writers
}

fn run(hours: &[World], resume: Option<usize>) -> Vec<[Vec<(u128, Vec<String>)>; 2]> {
    let mut store = store();
    let mut writers = writers();
    let mut seen = Vec::new();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        seen.push(TABLES.map(|table| rows(&store, table)));
        if resume == Some(hour) {
            let saved = writers.save().unwrap();
            writers = Writers::new(vec![Box::new(Casualties::default())]);
            writers.load(&saved, &mut store).unwrap();
        }
    }
    seen
}

fn totals(store: &Store, (slot, id): Identity) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::CasualtyTotals, holders::key((slot, id)))
}

fn lifetime(store: &Store, a: Identity, b: Identity, start: i32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::CasualtyLifetimes, lifetime_key(a, b, start))
}

const A: Identity = (1, 10);
const B: Identity = (2, 20);
const C: Identity = (3, 30);

fn known(values: [i64; 4]) -> Option<Vec<Option<i64>>> {
    Some(values.into_iter().map(Some).collect())
}

#[derive(Default)]
struct Lifetimes(BTreeMap<u128, (Vec<Option<i64>>, i64)>);

impl Lifetimes {
    fn hour(&mut self, store: &Store) {
        for held in self.0.values_mut() {
            held.1 = 0;
        }
        for (key, _) in rows(store, TableId::CasualtyLifetimes) {
            self.0.insert(key, (cells(store, TableId::CasualtyLifetimes, key).unwrap(), 1));
        }
    }

    fn of(&self, a: Identity, b: Identity, start: i32) -> Option<Vec<Option<i64>>> {
        self.0.get(&lifetime_key(a, b, start)).map(|(counters, live)| counters.iter().copied().chain([Some(*live)]).collect())
    }
}

#[test]
fn casualty_totals_sum_every_lifetime_and_keep_ended_ones() {
    let hours = [
        world(&[(20, 10, 5, [4, 3, 0, 0])]),
        world(&[(20, 10, 5, [6, 3, 0, 0])]),
        world(&[]),
        world(&[(10, 20, 50, [1, 2, 0, 0]), (10, 30, 50, [7, 0, 0, 0])]),
    ];
    let mut store = store();
    let mut writers = writers();
    let mut found = Lifetimes::default();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        found.hour(&store);
        match hour {
            0 => {
                assert_eq!(lifetime(&store, A, B, 5), known([3, 4, 0, 0]));
                assert_eq!(found.of(A, B, 5), Some(vec![Some(3), Some(4), Some(0), Some(0), Some(1)]));
                assert_eq!(totals(&store, A), known([4, 3, 0, 0]));
                assert_eq!(totals(&store, B), known([3, 4, 0, 0]));
                assert_eq!(totals(&store, C), None, "a country never at war has no row");
            }
            1 => assert_eq!(totals(&store, B), known([3, 6, 0, 0])),
            2 => {
                assert_eq!(lifetime(&store, A, B, 5), None, "an ended lifetime's row goes");
                assert_eq!(found.of(A, B, 5), Some(vec![Some(3), Some(6), Some(0), Some(0), Some(0)]), "its last row is its last read");
                assert_eq!(totals(&store, A), known([6, 3, 0, 0]));
            }
            _ => {
                assert_eq!(lifetime(&store, A, B, 50), known([1, 2, 0, 0]));
                assert_eq!(found.of(A, B, 5), Some(vec![Some(3), Some(6), Some(0), Some(0), Some(0)]));
                assert_eq!(totals(&store, A), Some(vec![Some(6 + 2), Some(3 + 1 + 7), None, Some(0)]), "no relation spans its last two reads");
                assert_eq!(totals(&store, B), Some(vec![Some(3 + 1), Some(6 + 2), None, Some(0)]));
                assert_eq!(totals(&store, C), known([7, 0, 0, 0]));
            }
        }
    }
}

#[test]
fn an_unknown_total_needs_a_zero_baseline_and_agreeing_increments() {
    let hours = [
        world(&[(10, 20, 0, [0, 0, 0, 2]), (10, 30, 0, [0, 0, 0, 0])]),
        world(&[(10, 20, 0, [0, 0, 5, 2]), (10, 30, 0, [0, 0, 5, 0])]),
        world(&[(10, 20, 0, [0, 0, 8, 2]), (10, 30, 0, [0, 0, 8, 3]), (20, 30, 1, [0, 0, 0, 3])]),
        world(&[(10, 20, 0, [0, 0, 9, 2]), (10, 30, 0, [0, 0, 8, 3]), (20, 30, 1, [0, 0, 0, 3])]),
        world(&[(10, 20, 0, [0, 0, 10, 2]), (10, 30, 0, [0, 0, 9, 4]), (20, 30, 1, [0, 0, 0, 4])]),
    ];
    let mut store = store();
    let mut writers = writers();
    let unknown = |store: &Store, identity| totals(store, identity).unwrap()[2];
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let expected = match hour {
            0 => [Some(0), None, Some(0)],
            1 => [Some(5), None, Some(0)],
            2 => [Some(8), None, Some(3)],
            3 => [None, None, Some(3)],
            _ => [None, None, Some(4)],
        };
        assert_eq!([unknown(&store, A), unknown(&store, B), unknown(&store, C)], expected, "hour {hour}");
    }
}

#[test]
fn a_gap_loses_unknown_totals_and_marks_lost_lifetimes_partial() {
    let mut hours = vec![
        world(&[(10, 20, 0, [1, 1, 0, 0]), (10, 30, 0, [1, 1, 0, 0])]),
        world(&[(10, 20, 0, [2, 2, 0, 0]), (10, 30, 0, [2, 2, 0, 0])]),
        world(&[(10, 20, 0, [3, 3, 0, 0])]),
        world(&[(10, 20, 0, [4, 4, 0, 0])]),
    ];
    hours[1].status = Status::Truncated { missing: 1 };
    let mut store = store();
    let mut writers = writers();
    let mut found = Lifetimes::default();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::CasualtyTotals.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 1, "hour {hour}");
        found.hour(&store);
    }
    assert_eq!(totals(&store, A), Some(vec![Some(5), Some(5), None, Some(1)]));
    assert_eq!(totals(&store, B), Some(vec![Some(4), Some(4), None, Some(0)]), "its lifetime is still listed");
    assert_eq!(totals(&store, C), Some(vec![Some(1), Some(1), None, Some(1)]));
    assert_eq!(lifetime(&store, A, C, 0), None);
    assert_eq!(found.of(A, C, 0), Some(vec![Some(1), Some(1), Some(0), Some(0), Some(0)]), "the last read before the gap");
}

#[test]
fn an_unresolved_relation_withholds_the_hour_and_a_new_identity_starts_new_lifetimes() {
    let mut hours = vec![
        world(&[(10, 20, 0, [1, 1, 0, 0])]),
        world(&[(10, 20, 0, [1, 1, 0, 0]), (10, 99, 0, [0, 0, 0, 0])]),
        world(&[(10, 20, 0, [2, 2, 0, 0])]),
        world(&[(10, 20, 0, [3, 3, 0, 0])]),
    ];
    hours[3].countries = vec![(11, 1), (20, 2), (30, 3)];
    hours[3].links = vec![(10, 1), (11, 1), (20, 2), (30, 3)];
    let mut store = store();
    let mut writers = writers();
    let mut found = Lifetimes::default();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        found.hour(&store);
        let status = store.view(TableId::CasualtyLifetimes.index()).status();
        assert_eq!(matches!(status, Status::Unavailable { .. }), hour == 1, "hour {hour}");
        match hour {
            1 => assert_eq!(totals(&store, A), known([1, 1, 0, 0]), "rows stay as they were"),
            2 => assert_eq!(totals(&store, A), Some(vec![Some(2), Some(2), None, Some(0)])),
            _ => {}
        }
    }
    let renamed = (1, 11);
    assert_eq!(lifetime(&store, A, B, 0), None);
    assert_eq!(found.of(A, B, 0), Some(vec![Some(2), Some(2), Some(0), Some(0), Some(0)]));
    assert_eq!(lifetime(&store, renamed, B, 0), known([3, 3, 0, 0]));
    assert_eq!(totals(&store, renamed), known([3, 3, 0, 0]));
    assert_eq!(totals(&store, B), Some(vec![Some(5), Some(5), None, Some(0)]));
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let mut hours = vec![
        world(&[(10, 20, 0, [0, 0, 0, 0]), (10, 30, 0, [1, 0, 0, 0])]),
        world(&[(10, 20, 0, [2, 1, 3, 0]), (10, 30, 0, [1, 0, 3, 0])]),
        world(&[(10, 20, 0, [2, 1, 4, 0]), (10, 30, 0, [1, 0, 4, 0])]),
        world(&[(10, 20, 0, [2, 1, 4, 0]), (20, 30, 2, [5, 5, 0, 0])]),
        world(&[(10, 20, 0, [3, 1, 4, 0]), (20, 30, 2, [6, 5, 0, 1])]),
        world(&[(10, 20, 4, [0, 0, 0, 0]), (20, 30, 2, [6, 6, 0, 1])]),
        world(&[(10, 20, 4, [1, 0, 0, 0])]),
    ];
    hours[4].status = Status::Truncated { missing: 1 };
    let whole = run(&hours, None);
    for resume in 0..hours.len() - 1 {
        assert_eq!(run(&hours, Some(resume)), whole, "resumed after hour {resume}");
    }
}

#[test]
fn lifetime_and_total_keys_are_the_row_codecs() {
    let (a, b) = ((CasualtyLifetimeRow::FIELD, 7), (CasualtyLifetimeRow::FIELD, CasualtyLifetimeRow::FIELD));
    for start in [i32::MIN, -1, 0, 1_200_000] {
        let key = lifetime_key(a, b, start);
        let row = CasualtyLifetimeRow { slot_a: a.0, id_a: a.1, slot_b: b.0, id_b: b.1, start_hour: start, ..Default::default() };
        assert_eq!(key, row.key());
        assert_eq!(lifetime_of(key), (a, b));
    }
    let row = CasualtyTotalRow { slot: u32::MAX, id: 3, ..Default::default() };
    assert_eq!(holders::key((u32::MAX, 3)), row.key());
    assert_eq!(holders::of(row.key()), (u32::MAX, 3));
}
