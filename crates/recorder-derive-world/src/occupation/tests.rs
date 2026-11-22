use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, CountryRow, OccupierTotalRow, StateGarrisonRow};

#[derive(Clone)]
struct World {
    countries: Vec<(u32, u32)>,
    links: Vec<(u32, u32)>,
    states: Vec<(u32, u32)>,
    resistance: Vec<(u32, u32, i64, i64, u8)>,
    garrisons: Vec<(u32, u32)>,
    garrison_status: Status,
}

fn world(states: &[(u32, u32)], resistance: &[(u32, u32, i64, i64, u8)], garrisons: &[(u32, u32)]) -> World {
    World {
        countries: vec![(10, 1), (20, 2), (30, 3)],
        links: vec![(10, 1), (20, 2), (30, 3)],
        states: states.to_vec(),
        resistance: resistance.to_vec(),
        garrisons: garrisons.to_vec(),
        garrison_status: Status::Complete,
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = world.countries.iter().map(|&(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    let states: Vec<StateRow> = world.states.iter().map(|&(id, controller)| StateRow { id, owner: controller, controller, ..Default::default() }).collect();
    put(store, hour, TableId::States, &states, Status::Complete);
    let resistance: Vec<StateResistanceRow> = world
        .resistance
        .iter()
        .map(|&(state, country, level, compliance, active)| StateResistanceRow { state, country, level, compliance, active, ..Default::default() })
        .collect();
    put(store, hour, TableId::StateResistance, &resistance, Status::Complete);
    let garrisons: Vec<StateGarrisonRow> = world.garrisons.iter().map(|&(controller, state)| StateGarrisonRow { controller, state, ..Default::default() }).collect();
    put(store, hour, TableId::StateGarrisons, &garrisons, world.garrison_status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Occupation::default())]);
    writers.start_run();
    writers
}

fn totals(store: &Store, identity: Identity) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::OccupierTotals, holders::key(identity))
}

const A: Identity = (1, 10);
const B: Identity = (2, 20);
const C: Identity = (3, 30);

#[test]
fn occupier_totals_sum_occupied_states_and_unite_held_ones() {
    let states = [(1, 10), (2, 10), (3, 10), (4, 20), (5, 20)];
    let hours = [
        world(&states, &[(1, 20, 50_000, 10_000, 1), (2, 20, 30_000, 20_000, 1), (3, 20, 99, 99, 0), (4, 0, 99, 99, 1)], &[(10, 1), (10, 3)]),
        world(&states, &[(1, 20, 60_000, 10_000, 1), (2, 20, 30_000, 20_000, 1)], &[(10, 1), (10, 2), (10, 3)]),
        world(&[(1, 10), (2, 30), (3, 10), (4, 20), (5, 20)], &[(1, 20, 60_000, 10_000, 1), (2, 20, 30_000, 20_000, 1)], &[(10, 1), (10, 2), (10, 3)]),
        world(&states, &[], &[]),
    ];
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        match hour {
            0 => {
                assert_eq!(totals(&store, A), Some(vec![Some(80_000), Some(30_000), Some(2), Some(3), Some(0)]), "state 2 is not garrisoned");
                assert_eq!(totals(&store, B), None, "an inactive or unoccupied state counts for nobody");
            }
            1 => assert_eq!(totals(&store, A), Some(vec![Some(90_000), Some(30_000), Some(2), Some(3), Some(1)])),
            2 => {
                assert_eq!(totals(&store, A), Some(vec![Some(60_000), Some(10_000), Some(1), Some(3), Some(1)]));
                assert_eq!(totals(&store, C), Some(vec![Some(30_000), Some(20_000), Some(1), Some(1), Some(0)]), "a controller follows the state");
            }
            _ => assert!(rows(&store, TableId::OccupierTotals).is_empty()),
        }
    }
}

#[test]
fn an_unresolved_controller_or_unread_garrisons_say_what_is_not_known() {
    let states = [(1, 10), (2, 20), (3, 99)];
    let resistance = [(1, 20, 5, 6, 1), (2, 10, 7, 8, 1)];
    let mut hours = vec![
        world(&states, &resistance, &[(10, 1), (10, 7), (20, 2)]),
        world(&states, &[(1, 20, 5, 6, 1), (2, 10, 7, 8, 1), (3, 10, 1, 1, 1)], &[(10, 1), (10, 7), (20, 2)]),
        world(&states, &resistance, &[(10, 1), (10, 7), (20, 2)]),
        world(&states, &resistance, &[(10, 1), (10, 7), (20, 2)]),
        world(&states, &resistance, &[(10, 1), (10, 7), (20, 2), (99, 4)]),
    ];
    hours[3].garrison_status = Status::Truncated { missing: 1 };
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let expected = match hour {
            0 | 2 => [Some(vec![Some(5), Some(6), Some(1), Some(2), Some(1)]), Some(vec![Some(7), Some(8), Some(1), Some(1), Some(1)])],
            1 => [Some(vec![None, None, None, None, Some(0)]), Some(vec![None, None, None, None, Some(0)])],
            3 => [Some(vec![Some(5), Some(6), Some(1), Some(1), Some(0)]), Some(vec![Some(7), Some(8), Some(1), Some(1), Some(0)])],
            _ => [Some(vec![Some(5), Some(6), Some(1), Some(2), Some(0)]), Some(vec![Some(7), Some(8), Some(1), Some(1), Some(0)])],
        };
        assert_eq!([totals(&store, A), totals(&store, B)], expected, "hour {hour}");
        assert_eq!(store.view(TableId::OccupierTotals.index()).status(), Status::Complete);
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let states = [(1, 10), (2, 10), (3, 20)];
    let mut hours = vec![
        world(&states, &[(1, 20, 5, 6, 1), (3, 10, 1, 2, 1)], &[(10, 1)]),
        world(&states, &[(1, 20, 6, 6, 1), (2, 30, 3, 3, 1), (3, 10, 1, 2, 1)], &[(10, 1), (20, 3)]),
        world(&[(1, 10), (2, 20), (3, 20)], &[(1, 20, 6, 7, 1), (2, 30, 3, 3, 1), (3, 10, 1, 2, 0)], &[(10, 1), (20, 3)]),
        world(&states, &[(1, 20, 6, 7, 1), (2, 30, 3, 3, 1)], &[(10, 2)]),
        world(&states, &[(1, 20, 7, 7, 1)], &[(10, 2)]),
    ];
    hours[3].links = vec![(10, 1), (20, 2), (30, 3), (40, 3)];
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in hours.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::OccupierTotals));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(Occupation::default())]);
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
fn occupier_keys_are_the_row_codec_s() {
    let row = OccupierTotalRow { slot: 7, id: u32::MAX, ..Default::default() };
    assert_eq!(holders::key((7, u32::MAX)), row.key());
}
