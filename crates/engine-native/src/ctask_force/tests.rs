use super::*;
use crate::{
    ccountry,
    cgame_state,
    cland_combatant,
    cnaval_combat,
    cnaval_combatant,
    cprovince,
};
use engine_native_memory::memory::SliceMemory;

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

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    array(&mut bytes, STATE + cgame_state::COUNTRIES, COUNTRIES, 2);
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, LINKS, 2);
    put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    array(&mut bytes, STATE + cgame_state::PROVINCES, PROVINCES, 5);
    put64(&mut bytes, PROVINCES + 2 * 8, TARGET as u64);
    put64(&mut bytes, TARGET, cprovince::VTABLE_RVA.0 as u64);
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
    put64(&mut bytes, COMBAT, cnaval_combat::VTABLE_RVA.0 as u64);
    put64(&mut bytes, COMBAT + cnaval_combat::ATTACKER, SIDE as u64);
    put64(&mut bytes, COMBAT + cnaval_combat::PROVINCE, TARGET as u64);
    put64(&mut bytes, SIDE, cnaval_combatant::VTABLE_RVA.0 as u64);
    put64(&mut bytes, SIDE + cnaval_combatant::COMBAT, COMBAT as u64);
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
            movement_numerator: Some(0),
            retreat: Some(0),
            withdrawal: Some(0),
            naval_transfer: Some(0),
            combat_links: None,
            fresh: None,
            logical_owner: 1,
            fresh_owner: None,
        }
    );
    let mut defender = bytes.clone();
    put(&mut defender, SIDE_COUNTRY, &0_u32.to_le_bytes());
    assert_eq!(collect(&defender, 1, 3).1[0].combat_role, 2);
    let (walk, _, _) = collect(&bytes, 1, 2);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(walk.required_paths, 3);
    let mut foreign = bytes.clone();
    put64(&mut foreign, COMBAT, 1);
    assert!(matches!(
        collect(&foreign, 1, 3).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut idle = bytes;
    array(&mut idle, TASK + cunit::ORDINARY_ROUTE_DATA, ROUTES, 0);
    array(&mut idle, TASK + cunit::ALTERNATE_ROUTE_DATA, ROUTES, 0);
    array(&mut idle, TASK + cunit::COMBAT_REFERENCE_DATA, COMBATS, 0);
    let (walk, rows, _) = collect(&idle, 1, 0);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].combat_role, 0);
    assert_eq!(rows[0].combat_links, Some([0; 2]));
}

#[test]
fn growing_motion_keeps_the_first_full_routes_beyond_previous_capacity() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let count = 5_000_usize;
    let mut rows = Vec::new();
    let mut paths = Vec::new();
    let mut combats = cunit::FirstCombatMemo::new(0);
    let mut collector =
        MapMotionCollector::new(&memory, 0, STATE, &mut rows, &mut paths, &mut combats);
    for index in 0..count {
        collector.visit(TASK, ((index as u64 + 1) << 32) | 61);
    }
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, Outcome::Rows { len: count });
    assert_eq!(
        (walk.paths_len, walk.required_paths),
        (count * 3, count * 3)
    );
    assert_eq!(rows[count - 1].ordinary_start, (count - 1) * 3);
    assert_eq!(&paths[(count - 1) * 3..count * 3], &[3, 2, 4]);
    assert_eq!(rows[count - 1].combat_role, 1);
}

struct CombatLinksMemory {
    bytes: Vec<u8>,
    registry: engine_native_parts::creference_registry::test_support::Fixture,
    fault: Option<usize>,
    change_header: bool,
    header_reads: std::cell::Cell<usize>,
}

impl Memory for CombatLinksMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.fault == Some(address) && (address != COMBAT || destination.len() == 32) {
            return false;
        }
        if address == TASK + cunit::COMBAT_REFERENCE_DATA && destination.len() == 16 {
            let reads = self.header_reads.get();
            self.header_reads.set(reads + 1);
            if self.change_header && reads != 0 {
                destination.fill(0);
                return true;
            }
        }
        self.registry.copy(address, destination)
            || SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, destination)
    }
}

fn combat_links_fixture() -> (CombatLinksMemory, [u64; 2]) {
    use crate::cland_combat;
    use engine_native_parts::creference_registry::test_support::Fixture;
    const NAVAL: usize = 0x3500;
    let first = (u64::from(u32::MAX) << 32) | 0xfedc_ba98;
    let naval = (0x8000_0000_u64 << 32) | 0xfedc_ba98;
    assert_ne!(Fixture::index(first), Fixture::index(naval));
    let mut bytes = fixture();
    array(&mut bytes, TASK + cunit::COMBAT_REFERENCE_DATA, COMBATS, 3);
    put64(&mut bytes, COMBATS + 8, NAVAL as u64);
    put64(&mut bytes, COMBATS + 16, u64::MAX);
    put64(&mut bytes, COMBAT, cland_combat::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        COMBAT + 0x10,
        cland_combat::BODY_VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, COMBAT + 0x18, first);
    put64(&mut bytes, SIDE, cland_combatant::VTABLE_RVA.0 as u64);
    put64(&mut bytes, NAVAL, cnaval_combat::VTABLE_RVA.0 as u64);
    put64(&mut bytes, NAVAL + 0x10, 0x02a5_6e70);
    put64(&mut bytes, NAVAL + 0x18, naval);
    let mut registry = Fixture::new(first as u32);
    registry.entry(Fixture::index(first), 1, first, (COMBAT + 0x10) as u64);
    registry.entry(Fixture::index(naval), 1, naval, (NAVAL + 0x10) as u64);
    registry.put(
        0x1000 + engine_native_parts::creference_registry::COUNT,
        &2_u32.to_le_bytes(),
    );
    (
        CombatLinksMemory {
            bytes,
            registry,
            fault: None,
            change_header: false,
            header_reads: std::cell::Cell::new(0),
        },
        [first, naval],
    )
}

fn collect_combat_links(
    memory: &impl Memory,
    second: bool,
) -> (MapMotionWalk, Vec<MapMotionRow>, Vec<u32>) {
    let mut rows = vec![MapMotionRow::default(); 2];
    let mut paths = vec![0; 6];
    let mut combats = cunit::FirstCombatMemo::new(2);
    let mut collector =
        MapMotionCollector::new(memory, 0, STATE, &mut rows, &mut paths, &mut combats);
    collector.visit(TASK, 7);
    if second {
        collector.visit(0x3800, 8);
    }
    (collector.finish(None), rows, paths)
}

#[test]
fn combat_links_authenticate_full_identities_and_stop_at_first_native_naval_type() {
    let (mut memory, links) = combat_links_fixture();
    let (walk, rows, _) = collect_combat_links(&memory, false);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].combat_links, Some(links));
    assert_eq!((rows[0].target, rows[0].combat_role), (2, 1));
    assert_eq!(memory.header_reads.get(), 2);
    array(
        &mut memory.bytes,
        TASK + cunit::COMBAT_REFERENCE_DATA,
        COMBATS,
        1,
    );
    assert_eq!(
        collect_combat_links(&memory, false).1[0].combat_links,
        Some([links[0], 0])
    );
    put64(
        &mut memory.bytes,
        COMBAT,
        engine_native_parts::cland_border_war_combat::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut memory.bytes,
        COMBAT + 0x10,
        engine_native_parts::cland_border_war_combat::BODY_VTABLE_RVA.0 as u64,
    );
    put64(
        &mut memory.bytes,
        SIDE,
        engine_native_parts::cland_border_war_combatant::VTABLE_RVA.0 as u64,
    );
    assert_eq!(
        collect_combat_links(&memory, false).1[0].combat_links,
        Some([links[0], 0])
    );
}

#[test]
fn combat_link_suffix_faults_and_changed_headers_preserve_later_motion_prefixes() {
    use engine_native_parts::creference_registry::test_support::Fixture;
    for failure in 0..7 {
        let (mut memory, links) = combat_links_fixture();
        let unit = memory.bytes[TASK..TASK + 0x300].to_vec();
        memory.bytes[0x3800..0x3800 + unit.len()].copy_from_slice(&unit);
        array(
            &mut memory.bytes,
            0x3800 + cunit::COMBAT_REFERENCE_DATA,
            COMBATS,
            0,
        );
        match failure {
            0 => memory.fault = Some(COMBAT),
            1 => memory.fault = Some(COMBATS + 8),
            2 => put64(&mut memory.bytes, 0x3500 + 0x10, 1),
            3 => memory
                .registry
                .entry(Fixture::index(links[1]), 1, links[1], 0x3500),
            4 => put64(&mut memory.bytes, COMBATS + 8, 0),
            5 => put64(&mut memory.bytes, 0x3500, 1),
            _ => memory.change_header = true,
        }
        let (walk, rows, paths) = collect_combat_links(&memory, true);
        assert_eq!(walk.outcome, Outcome::Rows { len: 2 }, "fault {failure}");
        assert_eq!(rows[0].combat_links, None, "fault {failure}");
        assert_eq!(rows[1].combat_links, Some([0; 2]), "fault {failure}");
        assert_eq!(
            (rows[0].target, rows[0].combat_role, rows[1].target),
            (2, 1, 2)
        );
        assert_eq!(paths, [3, 2, 4, 3, 2, 4]);
        assert_eq!(rows[0].movement_numerator, Some(0));
    }
}

#[test]
fn arrow_scalar_faults_preserve_both_motion_prefixes_and_other_raw_scalars() {
    const SECOND: usize = 0x3800;
    struct Fault<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            address != self.at
                && SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
        }
    }
    let mut bytes = fixture();
    let unit = bytes[TASK..TASK + 0x300].to_vec();
    bytes[SECOND..SECOND + unit.len()].copy_from_slice(&unit);
    put(
        &mut bytes,
        TASK + crate::carmy::MOVEMENT_NUMERATOR,
        &(-12_345_i64).to_le_bytes(),
    );
    bytes[TASK + cunit::RETREAT] = 0xfe;
    bytes[TASK + cunit::WITHDRAWAL] = 0xfd;
    put64(&mut bytes, TASK + cunit::NAVAL_TRANSFER_MEMBERSHIP, 1);
    for offset in [
        crate::carmy::MOVEMENT_NUMERATOR,
        cunit::RETREAT,
        cunit::WITHDRAWAL,
        cunit::NAVAL_TRANSFER_MEMBERSHIP,
    ] {
        let memory = Fault {
            bytes: &bytes,
            at: TASK + offset,
        };
        let mut rows = vec![MapMotionRow::default(); 2];
        let mut paths = vec![0; 6];
        let mut combats = cunit::FirstCombatMemo::new(2);
        let mut collector =
            MapMotionCollector::new(&memory, 0, STATE, &mut rows, &mut paths, &mut combats);
        collector.visit(TASK, 7);
        collector.visit(SECOND, 8);
        let walk = collector.finish(None);
        assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
        assert_eq!(paths, [3, 2, 4, 3, 2, 4]);
        assert_eq!(
            (rows[0].target, rows[1].target, rows[1].combat_role),
            (2, 2, 1)
        );
        assert_eq!(
            rows[0].movement_numerator,
            (offset != crate::carmy::MOVEMENT_NUMERATOR).then_some(-12_345)
        );
        assert_eq!(rows[0].retreat, (offset != cunit::RETREAT).then_some(0xfe));
        assert_eq!(
            rows[0].withdrawal,
            (offset != cunit::WITHDRAWAL).then_some(0xfd)
        );
        assert_eq!(
            rows[0].naval_transfer,
            (offset != cunit::NAVAL_TRANSFER_MEMBERSHIP).then_some(1)
        );
        assert_eq!(
            (
                rows[1].movement_numerator,
                rows[1].retreat,
                rows[1].withdrawal,
                rows[1].naval_transfer
            ),
            (Some(0), Some(0), Some(0), Some(0))
        );
    }
}

const REGIONS: usize = 0x3400;
const REGION_ONE: usize = 0x3500;
const REGION_TWO: usize = 0x3600;
const ACCESSIBLE: usize = 0x3700;

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
            region + engine_native_parts::cstrategic_region::ID,
            &id.to_le_bytes(),
        );
    }
    let mission = TASK + NAVAL_MISSION;
    put64(
        &mut bytes,
        mission,
        engine_native_parts::cnaval_mission::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut bytes,
        mission + engine_native_parts::cnaval_mission::TASK_FORCE,
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
        mission + engine_native_parts::cnaval_mission::ACCESSIBLE_REGIONS,
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
fn growing_missions_keep_full_repeated_region_lists_from_the_first_header() {
    let bytes = mission_fixture();
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut rows = Vec::new();
    let mut regions = Vec::new();
    let count = 5_000_usize;
    let mut collector = MissionCollector::new(&memory, 0, STATE, &mut rows, &mut regions);
    for index in 0..count {
        collector.visit(TASK, ((index as u64 + 1) << 32) | 61);
    }
    let walk = collector.finish(None);
    assert_eq!(walk.outcome, Outcome::Rows { len: count });
    assert_eq!(
        (walk.regions_len, walk.required_regions),
        (count * 3, count * 3)
    );
    assert_eq!(rows[count - 1].regions_start, (count - 1) * 3);
    assert_eq!(&regions[(count - 1) * 3..count * 3], &[2, 1, 2]);
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
    let (walk, _, arena) = collect_missions(&bytes, 1, 2);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(walk.required_regions, 3);
    assert_eq!(arena[..3], [2, 1, 2]);
    let mut stray = bytes.clone();
    put(&mut stray, REGIONS + 8, &(REGION_TWO as u64).to_le_bytes());
    assert!(matches!(
        collect_missions(&stray, 1, 3).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut idle = bytes.clone();
    put(
        &mut idle,
        TASK + NAVAL_MISSION + MISSION_CURRENT_REGION,
        &0_u64.to_le_bytes(),
    );
    array(
        &mut idle,
        TASK + NAVAL_MISSION + engine_native_parts::cnaval_mission::ACCESSIBLE_REGIONS,
        ACCESSIBLE,
        0,
    );
    assert_eq!(
        collect_missions(&idle, 1, 3).0.outcome,
        Outcome::Rows { len: 0 }
    );
    for (at, value) in [
        (TASK + NAVAL_MISSION, 1),
        (TASK + NAVAL_MISSION + engine_native_parts::cnaval_mission::TASK_FORCE, 0),
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
