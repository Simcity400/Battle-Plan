use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, CountryRow, LexicalTokenRow, ProductionLineOutputRow, Recorded, RowCodec, VariantDefinitionRow};

const DESIGN: u64 = 0x0000_0007_0000_0003;
const OTHER: u64 = 0x0000_0008_0000_0003;
const INFANTRY: u32 = 7;
const TEXTS: &[&[u8]] = &[b"", b"infantry_equipment"];

fn fill<R: RowCodec>(store: &mut Store, hour: i32, table: TableId, rows: &[R], status: Status) {
    store.status(table.index(), status, true, Some(hour), true).unwrap();
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

#[derive(Clone)]
struct World {
    output: Vec<ProductionLineOutputRow>,
    status: Status,
    rocket: Vec<RocketProductionOutputRow>,
    links: Vec<(u32, u32)>,
    definitions: Vec<(u64, u32)>,
}

impl Default for World {
    fn default() -> Self {
        Self { output: Vec::new(), status: Status::Complete, rocket: Vec::new(), links: vec![(10, 1), (20, 2)], definitions: Vec::new() }
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = [(10, 1), (20, 2)].into_iter().map(|(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    fill(store, hour, TableId::Countries, &countries, Status::Complete);
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).collect();
    fill(store, hour, TableId::CountryLinks, &links, Status::Complete);
    fill(store, hour, TableId::ProductionLineOutput, &world.output, world.status);
    fill(store, hour, TableId::RocketProductionOutput, &world.rocket, Status::Complete);
    let definitions: Vec<VariantDefinitionRow> = world.definitions.iter().map(|&(id, archetype_token)| VariantDefinitionRow { id, equipment_token: archetype_token, archetype_token, ..Default::default() }).collect();
    fill(store, hour, TableId::VariantDefinitions, &definitions, Status::Complete);
    fill(store, hour, TableId::LexicalTokens, &[LexicalTokenRow { token: INFANTRY, literal: b"infantry_equipment".to_vec() }], Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Production::default())]);
    writers.start_run();
    writers
}

fn line(line: u64, country: u32, purpose: OutputPurpose, units: i64, spent: Option<i64>) -> ProductionLineOutputRow {
    ProductionLineOutputRow { line, country, variant: DESIGN, units, exact: true, spent_raw: Recorded::Value(spent), purpose: Recorded::Value(purpose.raw()), ..Default::default() }
}

fn total(store: &Store, country: u32, purpose: OutputPurpose) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::ProductionTotals, total_key(country, DESIGN, purpose.raw()))
}

fn country(store: &Store, country: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::ProductionCountryTotals, u128::from(country))
}

fn coverage(store: &Store, country: u32, source: u8) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::ProductionCoverage, coverage_key(country, source))
}

fn run(hours: &[World], mut check: impl FnMut(i32, &Store)) {
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        check(hour as i32, &store);
    }
}

#[test]
fn totals_keep_ended_lines_and_say_what_a_lost_row_leaves_unknown() {
    let mut hours = vec![World::default(); 5];
    hours[0].output = vec![line(1, 10, OutputPurpose::New, 3, Some(300)), line(2, 20, OutputPurpose::Refit, 0, Some(50))];
    hours[1].output = vec![line(1, 10, OutputPurpose::New, 5, Some(500)), line(2, 20, OutputPurpose::Refit, 1, Some(80))];
    hours[2].output = vec![ProductionLineOutputRow { ended: true, ..line(1, 10, OutputPurpose::New, 6, Some(600)) }, line(2, 20, OutputPurpose::Refit, 1, Some(80)), line(3, 10, OutputPurpose::New, 2, Some(100))];
    hours[3].output = vec![line(2, 20, OutputPurpose::Refit, 1, Some(80)), line(3, 10, OutputPurpose::New, 4, Some(200))];
    hours[4].output = vec![line(2, 20, OutputPurpose::Refit, 1, Some(80))];
    let known = |units: i64, spent: i64| Some(vec![Some(units), Some(0), Some(spent), Some(0), Some(0), None, Some(0)]);
    run(&hours, |hour, store| {
        let (new, refit, ten, twenty) = match hour {
            0 => (known(3, 300), known(0, 50), [3, 0, 300, 3, 0, 300], [0, 0, 50, 0, 0, 0]),
            1 => (known(5, 500), known(1, 80), [5, 0, 500, 5, 0, 500], [1, 0, 80, 0, 0, 0]),
            2 => (known(8, 700), known(1, 80), [8, 0, 700, 8, 0, 700], [1, 0, 80, 0, 0, 0]),
            3 => (known(10, 800), known(1, 80), [10, 0, 800, 10, 0, 800], [1, 0, 80, 0, 0, 0]),
            _ => {
                assert_eq!(total(store, 10, OutputPurpose::New), Some(vec![Some(10), Some(1), None, Some(i64::from(Total::LOST_OUTPUT)), None, None, Some(0)]));
                assert_eq!(country(store, 10), Some(vec![Some(10), Some(1), None, Some(10), Some(1), None]));
                return;
            }
        };
        assert_eq!(total(store, 10, OutputPurpose::New), new, "hour {hour}");
        assert_eq!(total(store, 20, OutputPurpose::Refit), refit, "hour {hour}");
        assert_eq!(country(store, 10), Some(ten.map(Some).to_vec()), "hour {hour}");
        assert_eq!(country(store, 20), Some(twenty.map(Some).to_vec()), "hour {hour}");
        assert_eq!(coverage(store, Coverage::RUN, Coverage::LINES), Some(vec![Some(0), None, None, Some(0)]), "hour {hour}");
    });
}

#[test]
fn a_producer_no_link_tells_makes_every_total_unknown_from_its_first_delivery() {
    let mut hours = vec![World::default(); 4];
    hours[0].output = vec![line(1, 99, OutputPurpose::Unknown, 0, Some(0)), line(2, 98, OutputPurpose::New, 0, Some(0))];
    hours[1].output = vec![line(1, 99, OutputPurpose::Unknown, 2, Some(40)), line(2, 98, OutputPurpose::New, 0, Some(0))];
    for world in &mut hours[2..] {
        world.links.extend([(99, 1), (98, 2)]);
    }
    hours[2].output = vec![line(1, 99, OutputPurpose::Unknown, 3, Some(60)), line(2, 98, OutputPurpose::New, 0, Some(0))];
    hours[3].output = vec![line(1, 99, OutputPurpose::Unknown, 4, Some(50)), line(2, 98, OutputPurpose::New, 4, Some(30))];
    run(&hours, |hour, store| {
        let run = coverage(store, Coverage::RUN, Coverage::LINES);
        let tainted = Some(vec![Some(0), Some(1), Some(1), Some(i64::from(Total::MISSED_PUBLICATION))]);
        let zero = Some(vec![Some(0); 6]);
        match hour {
            0 => {
                assert_eq!(run, Some(vec![Some(0), None, None, Some(0)]));
                assert_eq!(total(store, 10, OutputPurpose::Unknown), None);
                assert_eq!(country(store, 10), zero);
            }
            1 | 2 => {
                assert_eq!(run, tainted, "hour {hour}");
                assert_eq!(total(store, 10, OutputPurpose::Unknown), None, "hour {hour}");
                assert_eq!(country(store, 10), zero, "hour {hour}");
            }
            _ => {
                assert_eq!(total(store, 10, OutputPurpose::Unknown), None);
                assert_eq!(country(store, 10), zero);
                assert_eq!(total(store, 20, OutputPurpose::New), Some(vec![Some(4), Some(0), Some(30), Some(0), Some(0), None, Some(0)]));
                assert_eq!(run, tainted);
            }
        }
    });
}

#[test]
fn a_line_counts_for_the_first_producer_it_was_told() {
    let mut hours = vec![World::default(); 3];
    hours[0].links.extend([(97, 1), (96, 1)]);
    hours[1].links.extend([(97, 2), (95, 1)]);
    hours[2].links.extend([(97, 2), (96, 1), (95, 2)]);
    hours[0].output = vec![line(1, 97, OutputPurpose::New, 1, Some(100)), line(2, 96, OutputPurpose::Refit, 1, Some(10)), line(3, 95, OutputPurpose::Converted, 0, Some(0))];
    hours[1].output = vec![line(1, 97, OutputPurpose::New, 3, Some(300)), line(2, 96, OutputPurpose::Refit, 2, Some(20)), line(3, 95, OutputPurpose::Converted, 0, Some(0))];
    hours[2].output = vec![line(1, 97, OutputPurpose::New, 4, Some(400)), line(2, 96, OutputPurpose::Refit, 4, Some(40)), line(3, 95, OutputPurpose::Converted, 2, Some(20))];
    let known = |units: i64, spent: i64| Some(vec![Some(units), Some(0), Some(spent), Some(0), Some(0), None, Some(0)]);
    run(&hours, |hour, store| {
        let (new, refit) = match hour {
            0 => (known(1, 100), known(1, 10)),
            1 => (known(3, 300), known(2, 20)),
            _ => (known(4, 400), known(4, 40)),
        };
        assert_eq!(total(store, 10, OutputPurpose::New), new, "hour {hour}");
        assert_eq!(total(store, 10, OutputPurpose::Refit), refit, "hour {hour}");
        assert_eq!(total(store, 20, OutputPurpose::New), None, "hour {hour}");
        assert_eq!(total(store, 20, OutputPurpose::Converted), None, "hour {hour}");
        assert_eq!(coverage(store, Coverage::RUN, Coverage::LINES), Some(vec![Some(0), None, None, Some(0)]), "hour {hour}");
        if hour == 2 {
            assert_eq!(total(store, 10, OutputPurpose::Converted), known(2, 20));
        }
    });
}

#[test]
fn an_hour_not_read_and_a_refused_spending_are_unknown_from_then_on() {
    let mut hours = vec![World::default(); 4];
    hours[0].output = vec![line(1, 10, OutputPurpose::New, 1, Some(100))];
    hours[1].output = hours[0].output.clone();
    hours[1].status = Status::Unavailable { reason: 0 };
    hours[2].output = vec![line(1, 10, OutputPurpose::New, 4, Some(400))];
    hours[3].output = vec![line(1, 10, OutputPurpose::New, 5, None)];
    run(&hours, |hour, store| {
        let totals = total(store, 10, OutputPurpose::New);
        match hour {
            1 => {
                assert!(matches!(store.view(TableId::ProductionTotals.index()).status(), Status::Unavailable { .. }));
                assert_eq!(totals, Some(vec![Some(1), Some(0), Some(100), Some(0), Some(0), None, Some(0)]));
            }
            2 => {
                assert_eq!(totals, Some(vec![Some(4), Some(0), Some(400), Some(0), Some(0), None, Some(0)]));
                assert_eq!(coverage(store, Coverage::RUN, Coverage::LINES), Some(vec![Some(0), Some(2), Some(2), Some(i64::from(Total::MISSED_PUBLICATION))]));
            }
            3 => {
                assert_eq!(totals, Some(vec![Some(5), Some(0), None, Some(i64::from(Total::SPENDING_UNAVAILABLE)), None, None, Some(0)]));
                assert_eq!(country(store, 10), Some(vec![Some(5), Some(0), None, Some(5), Some(0), None]));
            }
            _ => {}
        }
    });
}

#[test]
fn another_observation_restarts_every_total() {
    let mut hours = vec![World::default(); 3];
    hours[0].output = vec![line(1, 10, OutputPurpose::New, 4, Some(400)), line(2, 20, OutputPurpose::New, 9, Some(900))];
    hours[1].output = vec![line(1, 10, OutputPurpose::New, 4, Some(400))];
    hours[2].output = vec![ProductionLineOutputRow { since_hour: 2, ..line(5, 10, OutputPurpose::New, 1, Some(10)) }];
    run(&hours, |hour, store| match hour {
        1 => {
            assert_eq!(total(store, 20, OutputPurpose::New), Some(vec![Some(9), Some(1), None, Some(i64::from(Total::LOST_OUTPUT)), None, None, Some(0)]));
        }
        2 => {
            assert_eq!(total(store, 10, OutputPurpose::New), Some(vec![Some(1), Some(0), Some(10), Some(0), Some(0), None, Some(0)]));
            assert_eq!(total(store, 20, OutputPurpose::New), None);
            assert_eq!(country(store, 20), Some(vec![Some(0); 6]));
            assert_eq!(coverage(store, Coverage::RUN, Coverage::LINES), Some(vec![Some(2), None, None, Some(0)]));
        }
        _ => {}
    });
}

fn kind(store: &Store, country: u32, archetype: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::ProductionTypeTotals, type_key(country, archetype))
}

#[test]
fn type_totals_follow_the_recorded_archetype_and_move_when_it_is_learnt() {
    let mut hours = vec![World::default(); 2];
    let other = |units: i64, spent: i64| ProductionLineOutputRow { variant: OTHER, ..line(3, 10, OutputPurpose::New, units, Some(spent)) };
    hours[0].output = vec![line(1, 10, OutputPurpose::New, 3, Some(300)), line(2, 20, OutputPurpose::Refit, 2, Some(200)), other(1, 50)];
    hours[0].definitions = vec![(DESIGN, INFANTRY)];
    hours[1].output = vec![line(1, 10, OutputPurpose::New, 3, Some(300)), line(2, 20, OutputPurpose::Refit, 2, Some(200)), other(2, 100)];
    hours[1].definitions = vec![(DESIGN, INFANTRY), (OTHER, INFANTRY)];
    let sums = |units: i64, spent: i64, without: (i64, i64)| Some(vec![Some(units), Some(0), Some(spent), Some(without.0), Some(0), Some(without.1), None]);
    run(&hours, |hour, store| {
        assert_eq!(total(store, 10, OutputPurpose::New).map(|cells| cells[6]), Some(Some(i64::from(INFANTRY))));
        assert_eq!(kind(store, 20, INFANTRY), sums(2, 200, (0, 0)), "hour {hour}");
        match hour {
            0 => {
                assert_eq!(kind(store, 10, INFANTRY), sums(3, 300, (3, 300)));
                assert_eq!(kind(store, 10, TypeTotal::NO_ARCHETYPE), sums(1, 50, (1, 50)));
                assert_eq!(kind(store, TypeTotal::WORLD, INFANTRY), sums(5, 500, (3, 300)));
                assert_eq!(kind(store, TypeTotal::WORLD, TypeTotal::NO_ARCHETYPE), sums(1, 50, (1, 50)));
                let view = store.view(TableId::ProductionTypeTotals.index());
                let slot = view.slot(type_key(10, INFANTRY)).unwrap();
                assert_eq!(view.text(TYPE[6], slot).and_then(|id| view.string(id)), Some(&b"infantry_equipment"[..]));
            }
            _ => {
                assert_eq!(kind(store, 10, INFANTRY), sums(5, 400, (5, 400)));
                assert_eq!(kind(store, 10, TypeTotal::NO_ARCHETYPE), sums(0, 0, (0, 0)));
                assert_eq!(kind(store, TypeTotal::WORLD, INFANTRY), sums(7, 600, (5, 400)));
                assert_eq!(total(store, 10, OutputPurpose::New).map(|cells| cells[0]), Some(Some(3)));
            }
        }
    });
}

fn rocket(country: u32, variant: u64, ticks: u64, deliveries: u64) -> RocketProductionOutputRow {
    let unknown = country == 0 && variant != 0;
    RocketProductionOutputRow {
        country,
        variant,
        spent_raw: if variant == 0 { 0 } else { 100 * ticks as i64 },
        exact: !unknown,
        ticks,
        inexact_ticks: if unknown { ticks } else { 0 },
        unknown_country_ticks: if unknown { ticks } else { 0 },
        units: if variant == 0 { 0 } else { deliveries as i64 },
        units_exact: !unknown || deliveries == 0,
        deliveries,
        inexact_deliveries: if unknown { deliveries } else { 0 },
        ..Default::default()
    }
}

#[test]
fn rocket_rows_that_go_back_or_have_no_producer_are_unknown_from_then() {
    let mut hours = vec![World::default(); 4];
    hours[0].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 2, 1), rocket(20, DESIGN, 1, 1)];
    hours[1].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 1, 1), rocket(20, DESIGN, 1, 1)];
    hours[2].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 1, 1), rocket(20, DESIGN, 1, 1), rocket(0, DESIGN, 1, 0)];
    hours[3].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 1, 1), rocket(0, DESIGN, 1, 0)];
    run(&hours, |hour, store| {
        let run = coverage(store, Coverage::RUN, Coverage::ROCKET);
        let ten = coverage(store, 10, Coverage::ROCKET);
        let twenty = coverage(store, 20, Coverage::ROCKET);
        match hour {
            0 => {
                assert_eq!(run, Some(vec![Some(0), None, None, Some(0)]));
                assert_eq!((ten, twenty), (None, None));
            }
            1 => assert_eq!(ten, Some(vec![None, None, Some(1), Some(i64::from(Total::ROCKET_INPUT_INEXACT))])),
            2 => assert_eq!(run, Some(vec![Some(0), None, Some(2), Some(i64::from(Total::ROCKET_OWNER_UNREADABLE))])),
            _ => assert_eq!(twenty, Some(vec![None, Some(3), Some(3), Some(i64::from(Total::LOST_OUTPUT))])),
        }
    });
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let mut hours = vec![World::default(); 6];
    hours[0].output = vec![line(1, 10, OutputPurpose::New, 1, Some(100)), line(2, 99, OutputPurpose::Converted, 0, Some(0))];
    hours[0].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 1, 1)];
    for world in &mut hours[2..] {
        world.definitions = vec![(DESIGN, INFANTRY)];
    }
    hours[1].output = vec![line(1, 10, OutputPurpose::New, 3, Some(250)), line(2, 99, OutputPurpose::Converted, 0, Some(0))];
    hours[1].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 2, 1)];
    hours[2].output = vec![ProductionLineOutputRow { ended: true, ..line(1, 10, OutputPurpose::New, 4, Some(300)) }, line(2, 99, OutputPurpose::Converted, 2, Some(10))];
    hours[2].rocket = vec![rocket(0, 0, 0, 0), rocket(10, DESIGN, 1, 1)];
    hours[3].output = vec![line(2, 99, OutputPurpose::Converted, 2, Some(10)), line(3, 20, OutputPurpose::Refit, 1, Some(70))];
    hours[3].rocket = hours[2].rocket.clone();
    let settled = hours[2].rocket.clone();
    for world in &mut hours[4..] {
        world.links.push((99, 2));
        world.rocket = settled.clone();
    }
    hours[4].output = vec![line(2, 99, OutputPurpose::Converted, 5, Some(40)), line(3, 20, OutputPurpose::Refit, 2, Some(90))];
    hours[5].output = vec![line(3, 20, OutputPurpose::Refit, 3, Some(110))];
    for (at, world) in hours.iter_mut().enumerate() {
        world.links.push((97, if at < 2 { 1 } else { 2 }));
        if at >= 3 {
            world.links.push((95, if at == 3 { 1 } else { 2 }));
        }
        let units = [1, 1, 2, 2, 3, 3][at];
        world.output.push(line(4, 97, OutputPurpose::Refit, units, Some(10 * units)));
        world.output.push(match at {
            5 => line(5, 95, OutputPurpose::New, 1, Some(5)),
            _ => line(5, 95, OutputPurpose::New, 0, Some(0)),
        });
    }
    let tables = [TableId::ProductionTotals, TableId::ProductionCountryTotals, TableId::ProductionTypeTotals, TableId::ProductionCoverage];
    let unbroken = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in hours.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(tables.map(|table| rows(&store, table)));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(Production::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = unbroken(None);
    for resume in 0..hours.len() - 1 {
        assert_eq!(unbroken(Some(resume)), whole, "resumed after hour {resume}");
    }
}

#[test]
fn keys_are_the_row_codecs() {
    let key = total_key(u32::MAX, u64::MAX, OutputPurpose::Refit.raw());
    assert_eq!(key, Total { country: u32::MAX, variant: u64::MAX, purpose: OutputPurpose::Refit.raw(), ..Default::default() }.key());
    assert_eq!(total_parts(key), (u32::MAX, u64::MAX, OutputPurpose::Refit.raw()));
    assert_eq!(coverage_key(Coverage::RUN, Coverage::ROCKET), Coverage { country: Coverage::RUN, source: Coverage::ROCKET, ..Default::default() }.key());
}
