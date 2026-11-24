use super::*;
use crate::crelation::status::bench::{Arena, Counting};

#[test]
fn a_task_force_row_keeps_its_signed_route_count() {
    let mut arena = Arena::new(0x10_0000);
    let forces: Vec<(usize, u32, i32)> = [(1, -2), (2, 0), (3, 5)]
        .into_iter()
        .map(|(slot, route)| {
            let force = arena.alloc(0x380);
            arena.u32(force + cunit::ROSTER_OWNER, slot);
            arena.u32(force + cunit::ROSTER_OWNER + 4, 7);
            arena.u32(force + cunit::ROSTER_OWNER + 8, slot);
            arena.u32(force + MISSION, 4);
            arena.u32(force + cunit::ORDINARY_ROUTE_COUNT, route as u32);
            (force, slot, route)
        })
        .collect();
    let memory = Counting::new();
    let reader = Reader {
        memory: &memory,
        base: arena.base(),
        provinces: Array { data: 0, count: 0 },
    };
    for &(force, slot, route) in &forces {
        let row = reader.row(force, 11, 22).unwrap();
        assert_eq!(
            (row.id, row.fleet, row.route_count, row.mission),
            (11, 22, route, 4)
        );
        assert_eq!(
            (row.roster_owner, row.expeditionary_owner, row.logical_owner),
            (slot, 7, slot)
        );
    }
}

