use super::*;
use crate::crelation_status::bench::{Arena, Counting, countries, report};

/// The row reader keeps the stored signed route length, owners and
/// mission of each task force, and refuses a force whose roster owner is
/// not its fleet's country.
#[test]
fn a_task_force_row_keeps_its_signed_route_count() {
    let mut arena = Arena::new(0x10_0000);
    let (game_state, _) = countries(&mut arena, 3, 0x20);
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
    let countries = ccountry::Reader::new(&memory, arena.base(), game_state).unwrap();
    let reader = Reader {
        memory: &memory,
        base: arena.base(),
        countries: &countries,
        provinces: Array { data: 0, count: 0 },
    };
    for &(force, slot, route) in &forces {
        let row = reader.row(force, 11, 22, slot).unwrap();
        assert_eq!(
            (row.id, row.fleet, row.route_count, row.mission),
            (11, 22, route, 4)
        );
        assert_eq!(
            (row.roster_owner, row.expeditionary_owner, row.logical_owner),
            (slot, 7, slot)
        );
    }
    assert!(reader.row(forces[0].0, 11, 22, 2).is_err());
}

/// Two task forces per country, at sea with no port location, each read
/// through the row reader whose scattered copy now carries the route count.
#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_task_force_route_count() {
    for n in [1_500, 5_000] {
        let mut arena = Arena::new(n * (0x20 + 2 * 0x380) + (n + 1) * 16 + 0x10_0000);
        let (game_state, _) = countries(&mut arena, n, 0x20);
        let forces: Vec<(usize, u32)> = (1..=n)
            .flat_map(|slot| [slot, slot])
            .map(|slot| {
                let force = arena.alloc(0x380);
                arena.u32(force + cunit::ROSTER_OWNER, slot as u32);
                arena.u32(force + MISSION, 1);
                arena.u32(force + cunit::ORDINARY_ROUTE_COUNT, (slot % 3) as u32);
                (force, slot as u32)
            })
            .collect();
        let memory = Counting::new();
        let base = arena.base();
        report("TaskForces (route_count in row read)", n, &memory, || {
            let countries = ccountry::Reader::new(&memory, base, game_state).unwrap();
            let reader = Reader {
                memory: &memory,
                base,
                countries: &countries,
                provinces: Array { data: 0, count: 0 },
            };
            let mut moving = 0;
            for &(force, owner) in &forces {
                let row = reader.row(force, 1, 1, owner).unwrap();
                moving += usize::from(row.route_count > 0);
            }
            format!("{} rows, {moving} with a route", forces.len())
        });
    }
}
