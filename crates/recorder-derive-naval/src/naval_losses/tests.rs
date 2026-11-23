use super::*;
use recorder_derive_framework::{
    framework::{Definitions, Writers},
    testing,
};
use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};
use replay_data::rows::{CountryLinkRow, RowCodec, SunkShipsRow};

const TEXTS: &[&[u8]] = &[b"not read"];

const BASE: i32 = game_model::calendar::HOUR_EPOCH + 3 * 365 * 24;
const MONTH: i32 = 36;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(NavalLosses::default())]);
    writers.define(Definitions::read_none().with_define(SHIP_RETENTION, Ok(1)).with_define(CONVOY_RETENTION, Ok(1)));
    writers.start_run();
    writers
}

fn store() -> Store {
    let mut store = testing::store();
    store.begin_run(BASE);
    store
}

fn sunk(name: &[u8], victim: u32, attacker: u32, sink_hour: i32, cause_record: u8) -> SunkShipEntry {
    SunkShipEntry { ship_name: name.to_vec(), victim_country: victim, attacker_country: attacker, sink_hour, cause_record, province: 5, province_present: 1, ..Default::default() }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    At,
    Carried,
    Unread,
}

struct Input {
    countries: bool,
    ships: (Read, Vec<SunkShipEntry>),
    convoys: (Read, Vec<(u32, u32, i32)>),
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, input: &Input) {
    let first = hour == BASE;
    store.begin_hour(hour, if first { TEXTS } else { &[] }).unwrap();
    let countries = if input.countries { Status::Complete } else { Status::Unavailable { reason: 0 } };
    store.status(TableId::Countries.index(), countries, true, Some(hour), true).unwrap();
    store.status(TableId::CountryLinks.index(), Status::Complete, true, Some(hour), true).unwrap();
    if first {
        for (id, slot) in [(1, 10), (2, 20), (3, 10)] {
            testing::cells(store, TableId::Countries, u128::from(id), &[(COUNTRY_SLOT[0], Cell::U32(slot))]).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
        }
    }
    for (table, read) in [(TableId::SunkShips, input.ships.0), (TableId::SunkConvoys, input.convoys.0)] {
        match read {
            Read::At => store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap(),
            Read::Unread => store.status(table.index(), Status::Unavailable { reason: 0 }, true, None, true).unwrap(),
            Read::Carried => {}
        }
    }
    if input.ships.0 == Read::At {
        testing::put(store, TableId::SunkShips, &SunkShipsRow { entries: input.ships.1.clone() }, TEXTS).unwrap();
    }
    if input.convoys.0 == Read::At {
        let rows: Vec<SunkConvoyRow> = input.convoys.1.iter().map(|&(killer, owner, convoys)| SunkConvoyRow { month: MONTH, killer_country: killer, owner_country: owner, convoys, ordinal: 0 }).collect();
        let keys: Vec<u128> = rows.iter().map(RowCodec::key).collect();
        testing::keep_only(store, TableId::SunkConvoys, &keys).unwrap();
        for row in &rows {
            testing::put(store, TableId::SunkConvoys, row, TEXTS).unwrap();
        }
    }
    store.settle();
    writers.hour(1, hour, first, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

const HOURS: [i32; 7] = [0, 1, 2, 3, 4, 5, 8766];

fn script(offset: i32) -> Input {
    let a = sunk(b"Alpha", 1, 2, BASE, 0);
    let b = sunk(b"Bravo", 2, 1, BASE + 1, 1);
    let c = sunk(b"Charlie", 1, 2, BASE + 10, 0);
    let ships = |read, entries: &[&SunkShipEntry]| (read, entries.iter().map(|entry| (*entry).clone()).collect());
    let carried = (Read::Carried, Vec::new());
    let kept = (Read::Carried, Vec::new());
    match offset {
        0 => Input { countries: true, ships: ships(Read::At, &[&a]), convoys: (Read::At, vec![(2, 1, 3)]) },
        1 => Input { countries: true, ships: carried, convoys: kept },
        2 => Input { countries: true, ships: ships(Read::At, &[&a, &a, &b]), convoys: (Read::At, vec![(2, 1, 5), (5, 2, 1)]) },
        3 => Input { countries: true, ships: (Read::Unread, Vec::new()), convoys: kept },
        4 => Input { countries: true, ships: ships(Read::At, &[&a, &a, &b, &c]), convoys: kept },
        5 => Input { countries: false, ships: carried, convoys: kept },
        _ => Input { countries: true, ships: ships(Read::At, &[&a, &a, &b, &c]), convoys: (Read::At, vec![(2, 1, 5), (5, 2, 1)]) },
    }
}

fn totals(store: &Store) -> Vec<((u32, u32, u8, u8), [Option<i64>; 5])> {
    let view = store.view(TableId::NavalLossTotals.index());
    let mut rows: Vec<_> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            (((key >> 48) as u32, (key >> 16) as u32, (key >> 8) as u8, key as u8), [0, 1, 2, 3, 4].map(|column| view.number(column, slot)))
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn total(slot: u32, country: u32, attribution: u8, kind: u8, count: i64, unknown: i64) -> ((u32, u32, u8, u8), [Option<i64>; 5]) {
    ((slot, country, attribution, kind), [Some(count), Some(unknown), None, None, None])
}

fn run_row(kind: u8, counted_from: i32, known: bool, lost_from: Option<i32>) -> ((u32, u32, u8, u8), [Option<i64>; 5]) {
    ((Total::RUN, Total::RUN, Total::LOST, kind), [None, None, Some(i64::from(counted_from)), Some(i64::from(known)), lost_from.map(i64::from)])
}

#[test]
fn sinkings_count_each_rise_of_an_entry_and_each_bucket_change_per_identity_from_the_baseline() {
    let (mut store, mut writers) = (store(), writers());
    let mut seen = Vec::new();
    for offset in HOURS {
        run(&mut store, &mut writers, BASE + offset, &script(offset));
        seen.push(totals(&store));
    }
    let (lost, sunk_by, ships, convoys) = (Total::LOST, Total::SUNK_BY, Total::SHIPS, Total::CONVOYS);
    assert_eq!(seen[0], vec![run_row(ships, BASE, true, None), run_row(convoys, BASE, true, None)], "the first read of each kind is its baseline, never losses");
    assert_eq!(seen[1], seen[0], "an hour no table was read writes nothing");
    let after_two = vec![
        total(10, 1, lost, ships, 1, 0),
        total(10, 1, lost, convoys, 2, 0),
        total(10, 3, lost, ships, 1, 0),
        total(10, 3, lost, convoys, 2, 0),
        total(20, 2, lost, ships, 1, 0),
        total(20, 2, lost, convoys, 1, 0),
        total(20, 2, sunk_by, ships, 1, 0),
        total(20, 2, sunk_by, convoys, 2, 0),
        total(Total::NONE, 5, sunk_by, convoys, 1, 0),
        run_row(ships, BASE, true, None),
        run_row(convoys, BASE, true, None),
    ];
    assert_eq!(
        seen[2], after_two,
        "a second identical entry is a sinking; a cause record sinks by nobody; a loss goes to every identity of its slot; a bucket counts its change, a new one whole; a reference no link resolves counts apart"
    );
    let mut unread = after_two.clone();
    unread[9] = run_row(ships, BASE, false, None);
    assert_eq!(seen[3], unread, "an hour the ships are not read leaves their totals not known");
    let mut refused = after_two.clone();
    refused[0] = total(10, 1, lost, ships, 1, 1);
    refused[2] = total(10, 3, lost, ships, 1, 1);
    refused[6] = total(20, 2, sunk_by, ships, 1, 1);
    assert_eq!(seen[4], refused, "an entry dated after its read is refused, counted as unknown; an hour unread short of the retention loses nothing");
    let mut lower = refused.clone();
    lower[9] = run_row(ships, BASE, true, Some(BASE + 8766));
    lower[10] = run_row(convoys, BASE, true, Some(BASE + 8766));
    assert_eq!(seen[6], lower, "hours the writer did not run reaching the retention make every total a lower bound from the next read");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (store(), writers());
    let mut expected = Vec::new();
    for offset in HOURS {
        run(&mut unbroken, &mut writing, BASE + offset, &script(offset));
        expected.push(testing::rows(&unbroken, TableId::NavalLossTotals));
    }
    for stop in [0, 2, 3] {
        let (mut stopped, mut before) = (store(), writers());
        for &offset in &HOURS[..=stop] {
            run(&mut stopped, &mut before, BASE + offset, &script(offset));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, BASE + HOURS[stop]);
        for (at, &offset) in HOURS.iter().enumerate().skip(stop + 1) {
            run(&mut resumed, &mut after, BASE + offset, &script(offset));
            assert_eq!(testing::rows(&resumed, TableId::NavalLossTotals), expected[at], "stopped after hour {stop}, hour {offset}");
        }
    }
}
