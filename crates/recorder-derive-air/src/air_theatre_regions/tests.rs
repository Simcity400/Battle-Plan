use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{AirTheatreGroupRow, AirTheatreMemberRow, AirWingRow};
use std::collections::BTreeMap;

struct World {
    groups: &'static [(u64, u64)],
    members: &'static [(u64, u64)],
    wings: &'static [(u64, Option<u32>)],
    status: Status,
}

const HOURS: &[World] = &[
    World { groups: &[(10, 1), (20, 2)], members: &[(10, 100), (10, 101), (20, 200)], wings: &[(100, Some(5)), (101, Some(6)), (200, Some(5)), (300, Some(7))], status: Status::Complete },
    World { groups: &[(10, 1), (20, 2)], members: &[(10, 100), (10, 101), (20, 200)], wings: &[(100, Some(5)), (101, Some(5)), (200, Some(5)), (300, Some(8))], status: Status::Complete },
    World { groups: &[(10, 1), (20, 1)], members: &[(10, 100), (10, 101), (20, 200), (20, 300)], wings: &[(100, None), (101, Some(5)), (200, Some(5)), (300, Some(8))], status: Status::Complete },
    World { groups: &[(10, 1), (20, 1)], members: &[(10, 101)], wings: &[(100, None), (101, Some(5)), (200, Some(5)), (300, Some(8))], status: Status::Truncated { missing: 1 } },
    World { groups: &[(20, 1)], members: &[(20, 300)], wings: &[(100, None), (101, Some(5)), (200, Some(5))], status: Status::Complete },
];

fn expected(hour: usize) -> BTreeMap<(u64, u32), i64> {
    match hour {
        0 => [((1, 5), 1), ((1, 6), 1), ((2, 5), 1)].into(),
        1 => [((1, 5), 2), ((2, 5), 1)].into(),
        2 | 3 => [((1, 5), 2), ((1, 8), 1)].into(),
        _ => BTreeMap::new(),
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let groups: Vec<AirTheatreGroupRow> = world.groups.iter().map(|&(id, theatre)| AirTheatreGroupRow { id, theatre, ..Default::default() }).collect();
    let members: Vec<AirTheatreMemberRow> = world.members.iter().map(|&(group, wing)| AirTheatreMemberRow { group, wing, ordinal: 1 }).collect();
    let wings: Vec<AirWingRow> = world.wings.iter().map(|&(id, mission_region)| AirWingRow { id, owner: 1, mission_region, ..Default::default() }).collect();
    put(store, hour, TableId::AirTheatreGroups, &groups, Status::Complete);
    put(store, hour, TableId::AirTheatreMembers, &members, world.status);
    put(store, hour, TableId::AirWings, &wings, Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirTheatreRegions::default())]);
    writers.start_run();
    writers
}

fn held(store: &Store) -> BTreeMap<(u64, u32), i64> {
    let mut out = BTreeMap::new();
    for theatre in [1u64, 2] {
        for region in 5..=8u32 {
            if let Some(cells) = cells(store, TableId::AirTheatreRegions, key(theatre, region)) {
                out.insert((theatre, region), cells[0].unwrap());
            }
        }
    }
    out
}

#[test]
fn a_theatres_regions_are_its_member_wings_mission_regions() {
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let withheld = matches!(store.view(TableId::AirTheatreRegions.index()).status(), Status::Unavailable { .. });
        assert_eq!(withheld, hour == 3, "hour {hour}");
        if hour != 3 {
            assert_eq!(held(&store), expected(hour), "hour {hour}");
        }
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::AirTheatreRegions));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirTheatreRegions::default())]);
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
