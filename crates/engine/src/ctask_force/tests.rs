use super::*;
use crate::{
    ccountry, cgame_state, cland_combatant, cnaval_combat, cnaval_combatant, cprovince,
    memory::SliceMemory,
};

const STATE: usize = 0x100;
const COUNTRIES: usize = 0x200;
const LINKS: usize = 0x300;
const COUNTRY: usize = 0x400;
const PROVINCES: usize = 0x800;
const TARGET: usize = 0x1000;
const TASK: usize = 0x2000;
const ROUTES: usize = 0x3000;
const COMBATS: usize = 0x3100;
const COMBAT: usize = 0x3200;
const SIDE: usize = 0x3300;
const SIDE_COUNTRY: usize = 0x3400;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    put(bytes, at, &value.to_le_bytes());
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}

/// A task force of country 1 at sea moving to province 2 through 3
/// (alternate route 4), attacking in a naval combat.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    array(&mut bytes, STATE + cgame_state::COUNTRIES, COUNTRIES, 2);
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, LINKS, 2);
    put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    array(&mut bytes, STATE + cgame_state::PROVINCES, PROVINCES, 5);
    put64(&mut bytes, PROVINCES + 2 * 8, TARGET as u64);
    put64(&mut bytes, TARGET, cprovince::VTABLE_RVA as u64);
    put(&mut bytes, TARGET + cprovince::ID, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        TASK + cunit::LOGICAL_OWNER,
        &1_u32.to_le_bytes(),
    );
    put64(&mut bytes, TASK + cunit::MOVEMENT_TARGET, TARGET as u64);
    array(&mut bytes, TASK + cunit::ORDINARY_ROUTE_DATA, ROUTES, 2);
    array(
        &mut bytes,
        TASK + cunit::ALTERNATE_ROUTE_DATA,
        ROUTES + 8,
        1,
    );
    for (index, id) in [3_u32, 2, 4].into_iter().enumerate() {
        put(&mut bytes, ROUTES + index * 4, &id.to_le_bytes());
    }
    array(&mut bytes, TASK + cunit::COMBAT_REFERENCE_DATA, COMBATS, 1);
    put64(&mut bytes, COMBATS, COMBAT as u64);
    put64(&mut bytes, COMBAT, cnaval_combat::VTABLE_RVA as u64);
    put64(&mut bytes, COMBAT + cnaval_combat::ATTACKER, SIDE as u64);
    put64(&mut bytes, COMBAT + cnaval_combat::PROVINCE, TARGET as u64);
    put64(&mut bytes, SIDE, cnaval_combatant::VTABLE_RVA as u64);
    put64(&mut bytes, SIDE + cnaval_combatant::COMBAT, COMBAT as u64);
    // The naval side keeps the shared combatant country set; it holds tag 1.
    put64(
        &mut bytes,
        SIDE + cland_combatant::COUNTRIES,
        SIDE_COUNTRY as u64,
    );
    put(&mut bytes, SIDE_COUNTRY, &1_u32.to_le_bytes());
    bytes
}

fn collect(
    bytes: &[u8],
    rows: usize,
    cells: usize,
) -> (MapMotionWalk, Vec<MapMotionRow>, Vec<u32>) {
    let memory = SliceMemory { base: 0, bytes };
    let mut out = vec![MapMotionRow::default(); rows];
    let mut paths = vec![0_u32; cells];
    let mut combats = cunit::FirstCombatMemo::new(rows);
    let mut collector =
        MapMotionCollector::new(&memory, 0, STATE, &mut out, &mut paths, &mut combats);
    collector.visit(TASK, (7 << 32) | 61);
    let walk = collector.finish(None);
    (walk, out, paths)
}

#[test]
fn motion_rows_carry_routes_and_the_naval_combat_role() {
    let bytes = fixture();
    let (walk, rows, paths) = collect(&bytes, 1, 3);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!((walk.paths_len, walk.required_paths), (3, 3));
    assert_eq!(paths, [3, 2, 4]);
    assert_eq!(
        rows[0],
        MapMotionRow {
            id: (7 << 32) | 61,
            target: 2,
            combat_role: 1,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_len: 1,
        }
    );
    // An owner outside the attacking side's set defends.
    let mut defender = bytes.clone();
    put(&mut defender, SIDE_COUNTRY, &0_u32.to_le_bytes());
    assert_eq!(collect(&defender, 1, 3).1[0].combat_role, 2);
    // A short arena truncates and reports the demand the next hour grows to.
    let (walk, _, _) = collect(&bytes, 1, 2);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(walk.required_paths, 3);
    // A first combat that is no combat object refuses the census.
    let mut foreign = bytes.clone();
    put64(&mut foreign, COMBAT, 1);
    assert!(matches!(
        collect(&foreign, 1, 3).0.outcome,
        Outcome::Unavailable { .. }
    ));
    // Without routes or combats the row stands with role zero.
    let mut idle = bytes;
    array(&mut idle, TASK + cunit::ORDINARY_ROUTE_DATA, ROUTES, 0);
    array(&mut idle, TASK + cunit::ALTERNATE_ROUTE_DATA, ROUTES, 0);
    array(&mut idle, TASK + cunit::COMBAT_REFERENCE_DATA, COMBATS, 0);
    let (walk, rows, _) = collect(&idle, 1, 0);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].combat_role, 0);
}

const REGIONS: usize = 0x3400;
const REGION_ONE: usize = 0x3500;
const REGION_TWO: usize = 0x3600;
const ACCESSIBLE: usize = 0x3700;

/// A mission of country 42 patrolling region 2, with accessible regions 2, 1, 2.
fn mission_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    array(
        &mut bytes,
        STATE + cgame_state::STRATEGIC_REGIONS,
        REGIONS,
        3,
    );
    for (id, region) in [(1_u32, REGION_ONE), (2, REGION_TWO)] {
        put(
            &mut bytes,
            REGIONS + id as usize * 8,
            &(region as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            region + crate::cstrategic_region::ID,
            &id.to_le_bytes(),
        );
    }
    let mission = TASK + NAVAL_MISSION;
    put64(
        &mut bytes,
        mission,
        crate::cnaval_mission::VTABLE_RVA as u64,
    );
    put64(
        &mut bytes,
        mission + crate::cnaval_mission::TASK_FORCE,
        TASK as u64,
    );
    put(&mut bytes, mission + MISSION_OWNER, &42_u32.to_le_bytes());
    put(
        &mut bytes,
        mission + MISSION_CURRENT_REGION,
        &(REGION_TWO as u64).to_le_bytes(),
    );
    array(
        &mut bytes,
        mission + crate::cnaval_mission::ACCESSIBLE_REGIONS,
        ACCESSIBLE,
        3,
    );
    for (index, region) in [REGION_TWO, REGION_ONE, REGION_TWO].into_iter().enumerate() {
        put(
            &mut bytes,
            ACCESSIBLE + index * 8,
            &(region as u64).to_le_bytes(),
        );
    }
    bytes
}

fn collect_missions(
    bytes: &[u8],
    rows: usize,
    cells: usize,
) -> (MissionWalk, Vec<MissionRow>, Vec<u32>) {
    let memory = SliceMemory { base: 0, bytes };
    let mut out = vec![MissionRow::default(); rows];
    let mut arena = vec![0_u32; cells];
    let mut collector = MissionCollector::new(&memory, 0, STATE, &mut out, &mut arena);
    collector.visit(TASK, (7 << 32) | 61);
    let walk = collector.finish(None);
    (walk, out, arena)
}

#[test]
fn mission_rows_keep_accessible_regions_in_order_with_repeats() {
    let bytes = mission_fixture();
    let (walk, rows, arena) = collect_missions(&bytes, 1, 3);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!((walk.regions_len, walk.required_regions), (3, 3));
    assert_eq!(arena, [2, 1, 2]);
    assert_eq!(
        rows[0],
        MissionRow {
            id: (7 << 32) | 61,
            owner: 42,
            current_region: 2,
            regions_start: 0,
            regions_len: 3,
        }
    );
    // A short arena truncates and reports the demand the next hour grows to.
    let (walk, _, _) = collect_missions(&bytes, 1, 2);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(walk.required_regions, 3);
    // A region that is not the one registered at its id refuses the census.
    let mut stray = bytes.clone();
    put(&mut stray, REGIONS + 8, &(REGION_TWO as u64).to_le_bytes());
    assert!(matches!(
        collect_missions(&stray, 1, 3).0.outcome,
        Outcome::Unavailable { .. }
    ));
    // A mission with no current and no accessible region has no row.
    let mut idle = bytes.clone();
    put(
        &mut idle,
        TASK + NAVAL_MISSION + MISSION_CURRENT_REGION,
        &0_u64.to_le_bytes(),
    );
    array(
        &mut idle,
        TASK + NAVAL_MISSION + crate::cnaval_mission::ACCESSIBLE_REGIONS,
        ACCESSIBLE,
        0,
    );
    assert_eq!(
        collect_missions(&idle, 1, 3).0.outcome,
        Outcome::Rows { len: 0 }
    );
    // A foreign mission vtable or task-force back-pointer refuses the census whether or not
    // a row is kept: with room, with the row buffer exhausted, and for a mission with no
    // region.
    for (at, value) in [
        (TASK + NAVAL_MISSION, 1),
        (TASK + NAVAL_MISSION + crate::cnaval_mission::TASK_FORCE, 0),
    ] {
        for (fixture, rows) in [(&bytes, 1), (&bytes, 0), (&idle, 1)] {
            let mut forged = fixture.clone();
            put64(&mut forged, at, value);
            assert_eq!(
                collect_missions(&forged, rows, 3).0.outcome,
                Outcome::Unavailable {
                    reason: "naval mission identity mismatch"
                },
                "{at:#x} {rows}"
            );
        }
    }
}
