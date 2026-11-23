use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, CountryRow, NavyClassRow, ShipCounterInputs};

#[derive(Clone)]
struct World {
    forces: Vec<(u64, u32, u32)>,
    ships: Vec<(u64, u64, i64, i32)>,
    models: Vec<(u64, Option<i64>)>,
    status: Status,
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = [(10, 1), (20, 2), (30, 3)].into_iter().map(|(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let links: Vec<CountryLinkRow> = [(10, 1), (20, 2), (30, 3)].into_iter().map(|(id, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    let forces: Vec<TaskForceRow> = world
        .forces
        .iter()
        .map(|&(id, roster_owner, logical_owner)| TaskForceRow { id, roster_owner, logical_owner, ..Default::default() })
        .collect();
    put(store, hour, TableId::TaskForces, &forces, Status::Complete);
    let ships: Vec<ShipRow> = world
        .ships
        .iter()
        .map(|&(id, task_force, strength, class_id)| ShipRow { id, task_force, strength, class_id, ..Default::default() })
        .collect();
    put(store, hour, TableId::Ships, &ships, Status::Complete);
    let models: Vec<ShipModelRow> = world
        .models
        .iter()
        .map(|&(ship, maximum)| ShipModelRow {
            ship,
            counter: maximum.map(|maximum_strength| ShipCounterInputs { maximum_strength, ..Default::default() }),
            ..Default::default()
        })
        .collect();
    put(store, hour, TableId::ShipModels, &models, world.status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Navy::default())]);
    writers.start_run();
    writers
}

fn base() -> World {
    World {
        forces: vec![(100, 10, 20), (200, 20, 20)],
        ships: vec![(1, 100, 500, 7), (2, 100, 300, 8), (3, 200, 200, 7)],
        models: vec![(1, Some(1000)), (2, Some(50)), (3, Some(400))],
        status: Status::Complete,
    }
}

fn totals(store: &Store, slot: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::NavyTotals, u128::from(slot))
}

fn class(store: &Store, slot: u32, side: usize, class: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::NavyClasses, class_key(slot, side, class))
}

#[test]
fn navy_totals_follow_task_forces_and_say_what_is_not_known() {
    let mut hours = vec![base(); 6];
    hours[1].forces[0].1 = 30;
    hours[1].ships[2].2 = -5;
    for world in &mut hours[2..] {
        world.forces[0].1 = 30;
        world.models[1].1 = None;
    }
    hours[3].status = Status::Unavailable { reason: 0 };
    hours[4].ships.push((4, 999, 10, 9));
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let unknown = Some(vec![None; 6]);
        let expected: [(u32, Option<Vec<Option<i64>>>); 3] = match hour {
            0 => [
                (1, Some(vec![Some(2), Some(800), Some(1100), Some(0), Some(0), Some(0)])),
                (2, Some(vec![Some(1), Some(200), Some(400), Some(3), Some(1000), Some(1500)])),
                (3, Some(vec![Some(0); 6])),
            ],
            1 => [
                (1, Some(vec![Some(0); 6])),
                (2, Some(vec![Some(1), None, Some(400), Some(3), None, Some(1500)])),
                (3, Some(vec![Some(2), Some(800), Some(1100), Some(0), Some(0), Some(0)])),
            ],
            2 | 5 => [
                (1, Some(vec![Some(0); 6])),
                (2, Some(vec![Some(1), Some(200), Some(400), Some(3), Some(1000), None])),
                (3, Some(vec![Some(2), Some(800), None, Some(0), Some(0), Some(0)])),
            ],
            3 => [
                (1, Some(vec![Some(0), Some(0), None, Some(0), Some(0), None])),
                (2, Some(vec![Some(1), Some(200), None, Some(3), Some(1000), None])),
                (3, Some(vec![Some(2), Some(800), None, Some(0), Some(0), None])),
            ],
            _ => [(1, unknown.clone()), (2, unknown.clone()), (3, unknown.clone())],
        };
        for (slot, cells) in expected {
            assert_eq!(totals(&store, slot), cells, "hour {hour}, slot {slot}");
        }
        match hour {
            0 => {
                assert_eq!(class(&store, 2, 1, 7), Some(vec![Some(2), Some(700)]));
                assert_eq!(class(&store, 2, 1, 8), Some(vec![Some(1), Some(300)]));
                assert_eq!(class(&store, 1, 0, 8), Some(vec![Some(1), Some(300)]));
            }
            1 => {
                assert_eq!(class(&store, 2, 1, 7), Some(vec![Some(2), None]));
                assert_eq!(class(&store, 1, 0, 7), None, "the task force's ships left slot 1's roster");
                assert_eq!(class(&store, 3, 0, 7), Some(vec![Some(1), Some(500)]));
            }
            4 => assert!(rows(&store, TableId::NavyClasses).is_empty()),
            _ => assert_eq!(class(&store, 2, 1, 7), Some(vec![Some(2), Some(700)]), "hour {hour}"),
        }
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let mut hours = vec![base(); 6];
    hours[1].ships.push((4, 200, 50, 9));
    hours[2].forces[1].2 = 30;
    hours[2].ships[0].2 = 450;
    hours[3].models.remove(0);
    hours[3].forces.remove(1);
    hours[4].status = Status::Unavailable { reason: 0 };
    hours[5].ships.remove(1);
    let tables = [TableId::NavyTotals, TableId::NavyClasses];
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in hours.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(tables.map(|table| rows(&store, table)));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(Navy::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in 0..hours.len() - 1 {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}

#[test]
fn class_keys_are_the_row_codec_s() {
    let key = class_key(u32::MAX, 1, u32::MAX);
    assert_eq!(key, NavyClassRow { slot: u32::MAX, ownership: 1, class: u32::MAX, ..Default::default() }.key());
    assert_eq!(class_of(key), (u32::MAX, 1, u32::MAX));
}
