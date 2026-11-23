use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};

fn feed(store: &mut Store, hour: i32, wings: &[(u64, u32, Option<u32>)], status: Status) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let wings: Vec<AirWingRow> = wings.iter().map(|&(id, owner, mission_region)| AirWingRow { id, owner, mission_region, ..Default::default() }).collect();
    put(store, hour, TableId::AirWings, &wings, status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirRegionWings::default())]);
    writers.start_run();
    writers
}

fn wings(store: &Store, region: u32) -> Option<i64> {
    cells(store, TableId::AirRegionWings, u128::from(region)).map(|cells| cells[0].unwrap())
}

type Hours = [(&'static [(u64, u32, Option<u32>)], Status)];

const HOURS: &Hours = &[
    (&[(1, 10, Some(5)), (2, 20, Some(5)), (3, 10, Some(6)), (4, 10, None)], Status::Complete),
    (&[(1, 10, Some(5)), (2, 20, Some(6)), (3, 10, Some(6)), (4, 10, Some(7))], Status::Complete),
    (&[(1, 10, Some(5)), (2, 20, Some(6)), (4, 10, Some(7))], Status::Truncated { missing: 1 }),
    (&[(2, 20, Some(6)), (4, 10, None)], Status::Complete),
];

#[test]
fn region_wings_count_every_owner_s_wings_by_mission_region() {
    let mut store = store();
    let mut writers = writers();
    for (hour, &(world, status)) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, world, status);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::AirRegionWings.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 2, "hour {hour}");
        let expected = match hour {
            0 => [Some(2), Some(1), None],
            1 | 2 => [Some(1), Some(2), Some(1)],
            _ => [None, Some(1), None],
        };
        assert_eq!([wings(&store, 5), wings(&store, 6), wings(&store, 7)], expected, "hour {hour}");
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, &(world, status)) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, world, status);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::AirRegionWings));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirRegionWings::default())]);
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
