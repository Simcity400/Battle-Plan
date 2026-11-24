use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, store},
    framework::Writers,
};
use crate::production::Production;
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, CountryRow, LexicalTokenRow, OutputPurpose, ProductionLineOutputRow, Recorded, RowCodec};

const INFANTRY: u32 = 7;
const ARTILLERY: u32 = 8;
const KNOWN: Totals = Totals { units: 0, units_inexact: 0, spent: Some(0), units_without_refits: 0, units_without_refits_inexact: 0, spent_without_refits: Some(0) };

fn totals(units: i64, spent: Option<i64>) -> Totals {
    Totals { units, spent, units_without_refits: units, spent_without_refits: spent, ..KNOWN }
}

fn rocket(units: i64, spent: i64) -> RocketRow {
    RocketRow { units, units_exact: true, spent }
}

const RUN: Run = Run { lines: None, rocket: None, rocket_world: true };

#[test]
fn every_type_is_the_country_row_and_rocket_output_an_archetype_its_rows_and_designs() {
    let producer = Producer {
        country: totals(10, Some(500)),
        types: [(INFANTRY, totals(6, Some(300))), (ARTILLERY, totals(4, Some(200)))].into_iter().collect(),
        rocket: vec![(INFANTRY, rocket(2, 40)), (ARTILLERY, rocket(1, 10))],
        own_rocket: None,
    };
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::Units, 5), Amount::Known(13));
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::Ic, 5), Amount::Known(550));
    assert_eq!(total(&producer, &RUN, Part::Archetype(INFANTRY), Count::Units, 5), Amount::Known(8));
    assert_eq!(total(&producer, &RUN, Part::Archetype(ARTILLERY), Count::Ic, 5), Amount::Known(210));
    assert_eq!(total(&producer, &RUN, Part::Rest, Count::Units, 5), Amount::Known(0));
    assert_eq!(parts(&producer).into_iter().collect::<Vec<_>>(), vec![INFANTRY, ARTILLERY]);
    let set = [INFANTRY, ARTILLERY].map(|token| total(&producer, &RUN, Part::Archetype(token), Count::Units, 5));
    assert_eq!(set[0].plus(set[1]), total(&producer, &RUN, Part::Whole, Count::Units, 5));
}

#[test]
fn designs_whose_type_was_not_recorded_leave_every_archetype_unknown_while_they_hold_something() {
    let mut producer = Producer {
        country: totals(10, Some(500)),
        types: [(INFANTRY, totals(6, Some(300))), (TypeTotal::NO_ARCHETYPE, totals(0, Some(0)))].into_iter().collect(),
        rocket: vec![(TypeTotal::NO_ARCHETYPE, rocket(0, 0))],
        own_rocket: None,
    };
    assert_eq!(total(&producer, &RUN, Part::Archetype(INFANTRY), Count::Units, 5), Amount::Known(6), "untyped designs holding nothing veto nothing");
    producer.types.insert(TypeTotal::NO_ARCHETYPE, totals(3, Some(30)));
    for part in [Part::Archetype(INFANTRY), Part::Rest] {
        assert_eq!(total(&producer, &RUN, part, Count::Units, 5), Amount::Unknown(Row::TYPE_NOT_RECORDED), "{part:?}: untyped lines hold something");
    }
    assert_eq!(total(&producer, &RUN, Part::Archetype(TypeTotal::NO_ARCHETYPE), Count::Units, 5), Amount::Known(3), "the untyped designs themselves are known");
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::Units, 5), Amount::Known(10), "every type is known whatever the types");
    producer.types.insert(TypeTotal::NO_ARCHETYPE, totals(0, Some(0)));
    producer.rocket = vec![(TypeTotal::NO_ARCHETYPE, rocket(1, 5))];
    assert_eq!(total(&producer, &RUN, Part::Archetype(INFANTRY), Count::Ic, 5), Amount::Unknown(Row::TYPE_NOT_RECORDED), "a Rocket design no definition types");
    assert_eq!(total(&producer, &RUN, Part::Archetype(TypeTotal::NO_ARCHETYPE), Count::Ic, 5), Amount::Known(5));
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::Ic, 5), Amount::Known(505));
}

#[test]
fn coverage_inexact_counts_and_an_unread_rocket_table_are_never_counted() {
    let mut country = totals(10, Some(500));
    country.units_inexact = 1;
    country.units_without_refits_inexact = 1;
    let producer = Producer { country, types: [(INFANTRY, totals(6, None))].into_iter().collect(), rocket: Vec::new(), own_rocket: None };
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::Units, 5), Amount::Unknown(Row::UNITS_INEXACT));
    assert_eq!(total(&producer, &RUN, Part::Whole, Count::UnitsWithoutRefits, 5), Amount::Unknown(Row::UNITS_INEXACT));
    assert_eq!(total(&producer, &RUN, Part::Archetype(INFANTRY), Count::Ic, 5), Amount::Unknown(Row::SPENDING_UNAVAILABLE));
    let lost = Run { lines: Some(Covered { observation: Some(0), units_unknown_from: None, spending_unknown_from: Some(6), reason: Total::LOST_OUTPUT }), ..RUN };
    assert_eq!(total(&producer, &lost, Part::Rest, Count::Ic, 5), Amount::Known(0));
    assert_eq!(total(&producer, &lost, Part::Rest, Count::Ic, 6), Amount::Unknown(Row::LOST_OUTPUT), "an archetype it has no row of is unknown with the lines");
    let unread = Run { rocket_world: false, ..RUN };
    assert_eq!(total(&producer, &unread, Part::Rest, Count::Units, 5), Amount::Unknown(Row::ROCKET_TABLE_UNAVAILABLE));
    let own = Producer { own_rocket: Some(Covered { units_unknown_from: Some(3), reason: Total::ROCKET_INPUT_INEXACT, ..Covered::default() }), ..producer };
    assert_eq!(total(&own, &RUN, Part::Rest, Count::Units, 5), Amount::Unknown(Row::ROCKET_INPUT_INEXACT));
    let big = Producer { country: totals(i64::MAX, Some(0)), rocket: vec![(INFANTRY, rocket(1, 0))], ..Producer::default() };
    let cells = row(&big, &RUN, Part::Whole, 5);
    assert_eq!((cells[0], cells[1]), (Value::Unrecorded, Value::U8(Row::OUT_OF_RANGE)), "past 64 bits: not recorded, never wrapped");
}

const TEXTS: &[&[u8]] = &[b"", b"infantry_equipment"];
const DESIGN: u64 = 0x0000_0007_0000_0003;

fn fill<R: RowCodec>(store: &mut Store, hour: i32, table: TableId, rows: &[R]) {
    store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    let keys: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (key, _) in recorder_derive_framework::testing::rows(store, table) {
        if !keys.contains(&key) {
            recorder_derive_framework::testing::remove(store, table, key).unwrap();
        }
    }
    for row in rows {
        recorder_derive_framework::testing::put(store, table, row, TEXTS).unwrap();
    }
}

fn feed(store: &mut Store, hour: i32, units: Option<i64>) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    fill(store, hour, TableId::Countries, &[CountryRow { id: 10, canonical_slot: 1, ..Default::default() }]);
    fill(store, hour, TableId::CountryLinks, &[CountryLinkRow { id: 10, canonical_slot: 1 }]);
    let output: Vec<ProductionLineOutputRow> = units
        .map(|units| ProductionLineOutputRow { line: 1, country: 10, variant: DESIGN, units, exact: true, spent_raw: Recorded::Value(Some(units * 100)), purpose: Recorded::Value(OutputPurpose::Unknown.raw()), ..Default::default() })
        .into_iter()
        .collect();
    fill(store, hour, TableId::ProductionLineOutput, &output);
    fill(store, hour, TableId::RocketProductionOutput, &[RocketProductionOutputRow { exact: true, units_exact: true, ..Default::default() }]);
    fill(store, hour, TableId::VariantDefinitions, &[VariantDefinitionRow { id: DESIGN, equipment_token: INFANTRY, archetype_token: INFANTRY, ..Default::default() }]);
    fill(store, hour, TableId::LexicalTokens, &[LexicalTokenRow { token: INFANTRY, literal: b"infantry_equipment".to_vec() }]);
    store.settle();
}

#[test]
fn a_producers_rows_follow_production_and_the_clock_dates_them() {
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(Production::default()), Box::new(ProducedValues::default())]);
    writers.start_run();
    let hours = [Some(0), Some(2), Some(2), Some(5), None];
    let producer = |store: &Store, archetype: u32| cells(store, TableId::ProducedValues, Row::key_of(10, archetype));
    for (hour, units) in hours.into_iter().enumerate() {
        let hour = hour as i32;
        feed(&mut store, hour, units);
        writers.hour(1, hour, hour == 0, &mut store).unwrap();
        let clock = cells(&store, TableId::ProducedValues, Row::clock()).expect("the clock row lives");
        assert_eq!(clock[9], Some(i64::from(hour)), "hour {hour}: production was read this hour");
        assert_eq!(clock[8], Some(0), "hour {hour}: the totals count from the observation's first hour");
        let Some(units) = units else { continue };
        for archetype in [Row::WHOLE, INFANTRY] {
            let row = producer(&store, archetype).unwrap_or_else(|| panic!("hour {hour}: part {archetype}"));
            assert_eq!((row[0], row[1], row[4], row[5]), (Some(units), Some(0), Some(units * 100), Some(0)), "hour {hour}: part {archetype}");
        }
        assert_eq!(producer(&store, Row::REST).map(|row| (row[0], row[4])), Some((Some(0), Some(0))), "hour {hour}: an archetype without a row counts nothing");
    }
}
