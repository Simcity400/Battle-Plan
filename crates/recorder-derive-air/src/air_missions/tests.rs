use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::store::Store;
use replay_data::rows::{AirMissionWingRow, CountryLinkRow};

#[derive(Clone)]
struct World {
    links: Vec<(u32, u32)>,
    wings: Vec<(u64, u32, Option<u32>, u32, i32)>,
    registrations: Vec<(u64, u32)>,
    status: Status,
}

fn world(wings: &[(u64, u32, Option<u32>, u32, i32)], registrations: &[(u64, u32)]) -> World {
    World { links: vec![(10, 1), (20, 2)], wings: wings.to_vec(), registrations: registrations.to_vec(), status: Status::Complete }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
    put(store, hour, TableId::CountryLinks, &links, Status::Complete);
    let wings: Vec<AirWingRow> = world
        .wings
        .iter()
        .map(|&(id, owner, mission_region, mission_mask, planes)| AirWingRow { id, owner, mission_region, mission_mask, planes, ..Default::default() })
        .collect();
    put(store, hour, TableId::AirWings, &wings, Status::Complete);
    let registrations: Vec<AirRegionalMissionRow> = world.registrations.iter().map(|&(wing, ordinal)| AirRegionalMissionRow { wing, ordinal, active_order: None }).collect();
    put(store, hour, TableId::AirRegionalMissions, &registrations, world.status);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(AirMissions::default())]);
    writers.start_run();
    writers
}

fn counts(store: &Store, slot: u32, region: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::AirMissionCounts, group_key((slot, region)))
}

fn expect(wings: i64, missions: &[(usize, i64)]) -> Option<Vec<Option<i64>>> {
    let mut cells = vec![None; 1 + AirMissionCountRow::MISSIONS];
    cells[0] = Some(wings);
    for &(bit, total) in missions {
        cells[1 + bit] = Some(total);
    }
    Some(cells)
}

const NEGATIVE: u32 = 0x8000_0002;

#[test]
fn mission_counts_follow_registration_order_and_the_map_s_arithmetic() {
    let base = [(1, 10, Some(5), 0b101, 4), (2, 10, Some(5), 0b001, 3), (3, 10, Some(5), NEGATIVE, 9), (4, 20, Some(5), 0b010, 7), (5, 10, Some(6), 0, 2), (6, 10, Some(5), 1, 100)];
    let registrations = [(1, 2), (2, 1), (3, 0), (4, 0), (5, 0)];
    let mut wrapped = base;
    wrapped[0].4 = 1;
    wrapped[1].4 = i32::MAX;
    let mut moved = wrapped.to_vec();
    moved.push((7, 10, Some(5), 1, 2));
    moved[3].2 = Some(8);
    let hours = [world(&base, &registrations), world(&wrapped, &registrations), world(&moved, &[(1, 2), (2, 1), (3, 0), (4, 0), (7, 3)])];
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        match hour {
            0 => {
                assert_eq!(counts(&store, 1, 5), expect(3, &[(0, 7), (2, 4)]), "wing 3's negative mask selects nothing; wing 6 is not registered");
                assert_eq!(counts(&store, 2, 5), expect(1, &[(1, 7)]));
                assert_eq!(counts(&store, 1, 6), expect(1, &[]));
            }
            1 => assert_eq!(counts(&store, 1, 5), expect(3, &[(0, i64::from(i32::MIN)), (2, 1)])),
            _ => {
                assert_eq!(counts(&store, 1, 5), expect(4, &[(0, 2), (2, 1)]), "a negative total restarts");
                assert_eq!((counts(&store, 2, 5), counts(&store, 1, 6)), (None, None));
                assert_eq!(counts(&store, 2, 8), expect(1, &[(1, 7)]));
            }
        }
    }
}

#[test]
fn a_registration_that_does_not_join_withholds_the_table() {
    let wings = [(1, 10, Some(5), 1, 4), (2, 20, Some(5), 1, 3)];
    let mut changed = wings;
    changed[0].4 = 6;
    let mut unresolved = changed;
    unresolved[1].1 = 30;
    let mut regionless = changed;
    regionless[1].2 = None;
    let hours = [
        world(&wings, &[(1, 0), (2, 0)]),
        world(&changed, &[(1, 0), (2, 0), (99, 1)]),
        world(&unresolved, &[(1, 0), (2, 0)]),
        world(&regionless, &[(1, 0), (2, 0)]),
        world(&changed, &[(1, 0), (2, 0)]),
    ];
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let view = store.view(TableId::AirMissionCounts.index());
        let reason = match view.status() {
            Status::Unavailable { reason } => Some(String::from_utf8(view.string(reason).unwrap().to_vec()).unwrap()),
            _ => None,
        };
        let expected = match hour {
            1 => Some(REASONS[NO_WING]),
            2 => Some(REASONS[UNRESOLVED]),
            3 => Some(REASONS[NO_REGION]),
            _ => None,
        };
        assert_eq!(reason.as_deref(), expected, "hour {hour}");
        assert_eq!(store.view(TableId::AirMissionWings.index()).status(), view.status(), "the wings follow the counts' status, hour {hour}");
        let planes = if hour == 0 || (1..4).contains(&hour) { 4 } else { 6 };
        assert_eq!(counts(&store, 1, 5), expect(1, &[(0, planes)]), "hour {hour}");
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let wings = [(1, 10, Some(5), 0b11, 4), (2, 10, Some(5), 0b01, 3), (3, 20, Some(6), 0b10, 1)];
    let mut later = wings.to_vec();
    later[1].2 = Some(6);
    later.push((4, 20, Some(5), 0b100, 8));
    let mut relinked = world(&later, &[(1, 1), (2, 0), (3, 0), (4, 2)]);
    relinked.links = vec![(10, 1), (20, 3)];
    let mut withheld = world(&later, &[(1, 1), (2, 0), (3, 0), (4, 2), (9, 3)]);
    withheld.wings[0].4 = 2;
    let hours = [
        world(&wings, &[(1, 0), (2, 1), (3, 0)]),
        world(&later, &[(1, 1), (2, 0), (3, 0), (4, 2)]),
        relinked,
        withheld,
        world(&later, &[(1, 1), (3, 0)]),
    ];
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in hours.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push((rows(&store, TableId::AirMissionCounts), rows(&store, TableId::AirMissionWings)));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(AirMissions::default())]);
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
fn group_keys_are_the_row_codec_s() {
    let row = AirMissionCountRow { slot: u32::MAX, region: 7, ..Default::default() };
    assert_eq!(group_key((u32::MAX, 7)), row.key());
}

fn region_wings(store: &Store, slot: u32, region: u32) -> Vec<u64> {
    let view = store.view(TableId::AirMissionWings.index());
    let mut wings: Vec<(i64, u64)> = view
        .rows()
        .filter_map(|row| {
            let key = view.key(row)?;
            ((key >> 96) as u32 == slot && (key >> 64) as u32 == region).then(|| (view.number(0, row).unwrap(), key as u64))
        })
        .collect();
    wings.sort_unstable();
    wings.into_iter().map(|(_, wing)| wing).collect()
}

#[test]
fn a_region_s_wings_are_the_map_s_roster_in_registration_order() {
    let base = [(1, 10, Some(5), 0b101, 4), (2, 10, Some(5), 0b001, 3), (3, 10, Some(5), NEGATIVE, 9), (4, 20, Some(5), 0b010, 7), (5, 10, Some(6), 0, 2), (6, 10, Some(5), 1, 100)];
    let mut moved = base.to_vec();
    moved[0].2 = Some(6);
    moved[3].1 = 10;
    let mut relinked = world(&moved, &[(1, 0), (2, 4), (4, 1), (6, 2)]);
    relinked.links = vec![(10, 2), (20, 1)];
    let hours = [
        world(&base, &[(1, 2), (2, 1), (3, 0), (4, 0), (5, 0)]),
        world(&base, &[(1, 0), (2, 1), (3, 2), (4, 0), (5, 0)]),
        world(&moved, &[(1, 0), (2, 4), (4, 1), (6, 2)]),
        relinked,
    ];
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let wings: Vec<AirWingRow> = world.wings.iter().map(|&(id, owner, mission_region, mission_mask, planes)| AirWingRow { id, owner, mission_region, mission_mask, planes, ..Default::default() }).collect();
        let registrations: Vec<AirRegionalMissionRow> = world.registrations.iter().map(|&(wing, ordinal)| AirRegionalMissionRow { wing, ordinal, active_order: None }).collect();
        let links: HashMap<u32, u32> = world.links.iter().copied().collect();
        let mut held = 0;
        for slot in [1, 2] {
            let (_, rosters) = replay_derive::air_mission_counts::regional_counts(&registrations, &wings, slot, |raw| links.get(&raw).copied()).unwrap();
            for region in [5, 6] {
                let expected: Vec<u64> = rosters.get(&region).into_iter().flatten().map(|wing| wing.id).collect();
                held += expected.len();
                assert_eq!(region_wings(&store, slot, region), expected, "hour {hour}, slot {slot}, region {region}");
            }
        }
        assert_eq!(store.view(TableId::AirMissionWings.index()).rows().count(), held, "hour {hour}: no row but the rosters'");
    }
}

#[test]
fn wing_keys_are_the_row_codec_s() {
    let row = AirMissionWingRow { slot: u32::MAX, region: 7, wing: u64::MAX - 1, ordinal: 3 };
    assert_eq!(wing_key((u32::MAX, 7), u64::MAX - 1), row.key());
}
