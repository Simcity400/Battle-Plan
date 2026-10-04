use super::*;
use crate::{cland_combat, cland_combatant, memory::SliceMemory, reference_bucket};

const STATE: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const COUNTRY: usize = 0x3000;
const ROSTER: usize = 0x4000;
const PROVINCES: usize = 0x5000;
const FROM: usize = 0x6000;
const TO: usize = 0x6100;
const GUN: usize = 0x7000;
const ORDINARY: usize = 0x8000;
const ALTERNATE: usize = 0x8100;
const COMBATS: usize = 0x8200;
const COMBAT: usize = 0x9000;
const SIDE: usize = 0x9100;
const SIDE_COUNTRY: usize = 0x9200;
const COMBAT_TWO: usize = 0x9300;
const SIDE_TWO: usize = 0x9400;
const SIDE_TWO_COUNTRY: usize = 0x9500;
const TABLE: usize = 0xa000;
const BUCKETS: usize = 0xa100;
const GROUP_TABLE: usize = 0xb000;
const GROUP_BUCKETS: usize = 0xb100;
const GROUP: usize = 0xc000;
const SUBUNIT: usize = 0xd000;
const TRANSFER: usize = 0xe000;
const RAIL_MANAGER: usize = 0xf000;
const RAIL_SLOTS: usize = 0xf100;
const RAIL_INFO: usize = 0xf200;
const RAIL_BUILDING: usize = 0xf300;
const DEFINITION: usize = 0xf600;
const GUN_HANDLE: u64 = (1_u64 << 32) | 4713;
const GROUP_HANDLE: u64 = (3_u64 << 32) | corders_group::REFERENCE_TYPE as u64;
const TRANSFER_HANDLE: u64 = (4_u64 << 32) | 4713;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address..address + value.len()].copy_from_slice(value);
}

fn put64(bytes: &mut [u8], address: usize, value: u64) {
    put(bytes, address, &value.to_le_bytes());
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}

fn bucket(handle: u64) -> usize {
    bucket_at(handle, BUCKETS)
}

fn bucket_at(handle: u64, data: usize) -> usize {
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(creference_registry::HASH_MULTIPLIER);
    data + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE
}

fn registry(bytes: &mut [u8], table: usize, buckets: usize, entries: &[(u64, usize)]) {
    put64(bytes, table + creference_registry::ENTRIES, buckets as u64);
    put(
        bytes,
        table + creference_registry::COUNT,
        &(entries.len() as u32).to_le_bytes(),
    );
    put(
        bytes,
        table + creference_registry::MASK,
        &7_u32.to_le_bytes(),
    );
    bytes[table + creference_registry::OVERFLOW_DISTANCE] = 5;
    bytes[buckets + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
    for &(handle, object) in entries {
        let cell = bucket_at(handle, buckets);
        bytes[cell + reference_bucket::DISTANCE] = 1;
        put64(bytes, cell + reference_bucket::TYPE, handle);
        put64(bytes, cell + reference_bucket::OBJECT, object as u64);
    }
}

/// One moving, transferring, exiled gun in combat, on a damaged railway.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x12000];
    array(&mut bytes, STATE + cgame_state::COUNTRIES, COUNTRIES, 2);
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, LINKS, 2);
    put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    array(&mut bytes, COUNTRY + ccountry::RAILWAY_UNITS, ROSTER, 1);
    put64(&mut bytes, ROSTER, GUN as u64);

    array(&mut bytes, STATE + cgame_state::PROVINCES, PROVINCES, 3);
    for (id, pointer) in [(1_u32, FROM), (2, TO)] {
        put64(&mut bytes, PROVINCES + id as usize * 8, pointer as u64);
        put64(&mut bytes, pointer, cprovince::VTABLE_RVA as u64);
        put(&mut bytes, pointer + cprovince::ID, &id.to_le_bytes());
    }
    put64(
        &mut bytes,
        STATE + cgame_state::RAIL_MANAGER,
        RAIL_MANAGER as u64,
    );
    put64(
        &mut bytes,
        RAIL_MANAGER,
        crailway_manager::VTABLE_RVA as u64,
    );
    array(
        &mut bytes,
        RAIL_MANAGER + crailway_manager::PROVINCE_INFO_TABLE,
        RAIL_SLOTS,
        3,
    );
    put64(&mut bytes, RAIL_SLOTS + 8, RAIL_INFO as u64);
    put64(&mut bytes, FROM + cprovince::DEFINITION, DEFINITION as u64);
    put(
        &mut bytes,
        DEFINITION + province_definition::ID,
        &1_u32.to_le_bytes(),
    );
    put64(&mut bytes, RAIL_INFO, rail_info::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::PROVINCE_RECORD,
        DEFINITION as u64,
    );
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::BUILDING_STATUS,
        (FROM + cprovince::BUILDING_STATUS) as u64,
    );
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::BUILDING,
        RAIL_BUILDING as u64,
    );
    put64(&mut bytes, RAIL_BUILDING, cbuilding::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        RAIL_BUILDING + cbuilding::BUILDING_STATUS,
        (FROM + cprovince::BUILDING_STATUS) as u64,
    );
    put(
        &mut bytes,
        RAIL_BUILDING + cbuilding::LEVEL,
        &(-3_i16).to_le_bytes(),
    );

    put64(&mut bytes, GUN, crailway_gun::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        GUN + 0x10,
        crailway_gun::SECONDARY_VTABLE_RVA as u64,
    );
    put(&mut bytes, GUN + cunit::CLASS, &13_u32.to_le_bytes());
    put64(&mut bytes, GUN + cunit::HANDLE, GUN_HANDLE);
    put(&mut bytes, GUN + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        GUN + cunit::EXPEDITIONARY_OWNER,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, GUN + cunit::LOGICAL_OWNER, &1_u32.to_le_bytes());
    put64(
        &mut bytes,
        GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP,
        TRANSFER as u64,
    );
    bytes[GUN + cunit::EXILED] = 1;
    bytes[GUN + cunit::RETREAT] = 1;
    bytes[GUN + cunit::STRATEGIC_REDEPLOYMENT] = 1;
    put(
        &mut bytes,
        GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
        &1_i32.to_le_bytes(),
    );
    put64(
        &mut bytes,
        GUN + crailway_gun::ASSIGNEE,
        (GROUP + crailway_gun::ASSIGNEE_BASE) as u64,
    );
    put64(&mut bytes, GUN + cunit::STAT_BLOCK, SUBUNIT as u64);
    put(
        &mut bytes,
        GUN + crailway_gun::CURRENT_STRENGTH,
        &500_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        GUN + cunit::RECEIVED_SUPPLY_RATIO,
        &80_000_i64.to_le_bytes(),
    );
    put64(&mut bytes, SUBUNIT, csub_unit_definition::VTABLE_RVA as u64);
    put(
        &mut bytes,
        SUBUNIT + csub_unit_definition::TOKEN,
        &17_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        SUBUNIT + csub_unit_definition::MAXIMUM_STRENGTH,
        &1_000_000_i64.to_le_bytes(),
    );
    put64(
        &mut bytes,
        TRANSFER,
        cnaval_unit_transfer::VTABLE_RVA as u64,
    );
    put64(
        &mut bytes,
        TRANSFER + cnaval_unit_transfer::HANDLE,
        TRANSFER_HANDLE,
    );
    put(
        &mut bytes,
        TRANSFER + cnaval_unit_transfer::ACTION_STATUS,
        &1_i32.to_le_bytes(),
    );
    put64(&mut bytes, GUN + cunit::LOCATION, FROM as u64);
    put64(&mut bytes, GUN + cunit::MOVEMENT_TARGET, TO as u64);
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, ORDINARY, 2);
    put(&mut bytes, ORDINARY, &2_u32.to_le_bytes());
    put(&mut bytes, ORDINARY + 4, &1_u32.to_le_bytes());
    array(&mut bytes, GUN + cunit::ALTERNATE_ROUTE_DATA, ALTERNATE, 1);
    put(&mut bytes, ALTERNATE, &1_u32.to_le_bytes());
    array(&mut bytes, GUN + cunit::COMBAT_REFERENCE_DATA, COMBATS, 2);
    put64(&mut bytes, COMBATS, COMBAT as u64);
    // The gun attacks in its first combat, so the native walk stops there; a garbage
    // second reference is never touched.
    put64(&mut bytes, COMBATS + 8, u64::MAX);
    put64(&mut bytes, COMBAT, cland_combat::VTABLE_RVA as u64);
    put64(&mut bytes, COMBAT + cland_combat::ATTACKER, SIDE as u64);
    put64(&mut bytes, COMBAT + cland_combat::PROVINCE, TO as u64);
    put64(&mut bytes, SIDE, cland_combatant::VTABLE_RVA as u64);
    put64(&mut bytes, SIDE + cland_combatant::COMBAT, COMBAT as u64);
    put64(
        &mut bytes,
        SIDE + cland_combatant::COUNTRIES,
        SIDE_COUNTRY as u64,
    );
    // The attacking side's country set holds the gun's logical owner, tag 1.
    put(&mut bytes, SIDE_COUNTRY, &1_u32.to_le_bytes());

    registry(
        &mut bytes,
        TABLE,
        BUCKETS,
        &[(GUN_HANDLE, GUN + 0x10), (TRANSFER_HANDLE, TRANSFER)],
    );
    put64(&mut bytes, GROUP, corders_group::VTABLE_RVA as u64);
    put64(&mut bytes, GROUP + corders_group::HANDLE, GROUP_HANDLE);
    put64(
        &mut bytes,
        GROUP + crailway_gun::ASSIGNEE_BASE,
        crailway_gun::ORDERS_GROUP_ASSIGNEE_VTABLE_RVA as u64,
    );
    registry(
        &mut bytes,
        GROUP_TABLE,
        GROUP_BUCKETS,
        &[(GROUP_HANDLE, GROUP)],
    );
    bytes
}

/// The same gun standing still, with no transfer, combat or assignment.
fn standing() -> Vec<u8> {
    let mut bytes = fixture();
    put64(&mut bytes, GUN + cunit::MOVEMENT_TARGET, 0);
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, 0, 0);
    array(&mut bytes, GUN + cunit::ALTERNATE_ROUTE_DATA, 0, 0);
    array(&mut bytes, GUN + cunit::COMBAT_REFERENCE_DATA, 0, 0);
    put64(&mut bytes, GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP, 0);
    bytes[GUN + cunit::EXILED] = 0;
    bytes[GUN + cunit::RETREAT] = 0;
    bytes[GUN + cunit::STRATEGIC_REDEPLOYMENT] = 0;
    put(
        &mut bytes,
        GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
        &0_i32.to_le_bytes(),
    );
    put64(&mut bytes, GUN + crailway_gun::ASSIGNEE, 0);
    bytes
}

struct MemoryFixture {
    bytes: Vec<u8>,
    fault: Option<usize>,
}

impl Memory for MemoryFixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.fault == Some(address) {
            return false;
        }
        if address == creference_registry::HIGH_ROOT_RVA && output.len() == 8 {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
            return true;
        }
        if address
            == creference_registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8
            && output.len() == 8
        {
            output.copy_from_slice(&(GROUP_TABLE as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

fn capture(bytes: Vec<u8>, fault: Option<usize>, rows: &mut [Row], paths: &mut [u32]) -> Walk {
    walk(
        &MemoryFixture { bytes, fault },
        0,
        STATE,
        rows,
        paths,
        &mut cunit::FirstCombatMemo::new(4),
    )
}

fn outcome(bytes: Vec<u8>, fault: Option<usize>) -> Outcome {
    capture(bytes, fault, &mut [Row::default(); 1], &mut [0; 3]).outcome
}

#[test]
fn captures_the_roster_gun_motion_and_first_combat_without_map_entity_state() {
    let mut rows = [Row::default(); 1];
    let mut paths = [0; 3];
    assert_eq!(
        capture(fixture(), None, &mut rows, &mut paths),
        Walk {
            outcome: Outcome::Rows { len: 1 },
            paths_len: 3,
            required_paths: 3,
        }
    );
    assert_eq!(
        rows[0],
        Row {
            gun: GUN_HANDLE,
            roster_owner: 1,
            expeditionary_owner: 2,
            logical_owner: 1,
            province: 1,
            naval_transfer: true,
            transfer_action_status: true,
            exiled: true,
            retreat: true,
            strategic_redeployment: true,
            reference_entry_count: 1,
            assignee_group: GROUP_HANDLE,
            subunit_token: 17,
            current_strength: 500_000,
            maximum_strength: 1_000_000,
            received_supply_ratio: 80_000,
            railway_building_level: Some(-3),
            target: 2,
            combat_role: 1,
            combat_province: 2,
            combat_count: 2,
            attacking: true,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_start: 2,
            alternate_len: 1,
        }
    );
    assert_eq!(paths, [2, 1, 1]);
}

/// Guarded read calls of the hourly walk, with `guns` copies of the fixture's gun in its
/// roster: the fixed hour cost and, as the difference a second gun makes, the cost of each
/// further gun. A second copy shares the first's combat, as guns in one battle do, so its
/// first-combat answer comes from the capture memo.
fn read_calls(mut bytes: Vec<u8>, guns: usize) -> u64 {
    use crate::read_counts::{CountedMemory, ReadCounts};
    array(
        &mut bytes,
        COUNTRY + ccountry::RAILWAY_UNITS,
        ROSTER,
        guns as u32,
    );
    for index in 0..guns {
        put64(&mut bytes, ROSTER + index * 8, GUN as u64);
    }
    let mut rows = vec![Row::default(); guns];
    let mut paths = vec![0; 3 * guns];
    ReadCounts::take_current();
    let result = walk(
        &CountedMemory::new(MemoryFixture { bytes, fault: None }),
        0,
        STATE,
        &mut rows,
        &mut paths,
        &mut cunit::FirstCombatMemo::new(4),
    );
    let reads = ReadCounts::take_current();
    assert_eq!(result.outcome, Outcome::Rows { len: guns });
    reads.copy_calls + reads.scattered_calls + reads.record_calls + reads.plan_calls
}

/// Pins the walk's read calls so a change in its per-gun cost is a reviewed decision. The
/// walk runs in the Ordinary pool job, off the game thread, once an hour.
#[test]
fn read_calls_per_hour_and_per_gun_are_pinned() {
    let cost = |bytes: fn() -> Vec<u8>| {
        let one = read_calls(bytes(), 1);
        (one, read_calls(bytes(), 2) - one)
    };
    let (standing_hour, standing_gun) = cost(standing);
    let (moving_hour, moving_gun) = cost(fixture);
    eprintln!(
        "railway gun inputs: standing {standing_hour} calls for one gun, {standing_gun} per \
         further gun; moving in combat {moving_hour}, {moving_gun} per further gun"
    );
    assert_eq!((standing_hour, standing_gun), (60, 43));
    assert_eq!((moving_hour, moving_gun), (97, 57));
}

#[test]
fn standing_gun_keeps_null_links_as_zero() {
    let mut rows = [Row::default(); 1];
    let walk = capture(standing(), None, &mut rows, &mut []);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let row = rows[0];
    assert_eq!(row.target, 0);
    assert_eq!((row.combat_role, row.combat_province), (0, 0));
    assert_eq!((row.combat_count, row.attacking), (0, false));
    assert_eq!((row.ordinary_len, row.alternate_len), (0, 0));
    assert!(!row.naval_transfer && !row.transfer_action_status);
    assert!(!row.exiled && !row.retreat && !row.strategic_redeployment);
    assert_eq!(row.reference_entry_count, 0);
    assert_eq!(row.assignee_group, 0);
    assert_eq!(row.railway_building_level, Some(-3));
}

#[test]
fn rail_level_preserves_null_links_and_signed_values() {
    for (at, value, expected) in [
        (RAIL_SLOTS + 8, 0_u64.to_le_bytes().to_vec(), None),
        (
            RAIL_INFO + rail_info::BUILDING,
            0_u64.to_le_bytes().to_vec(),
            None,
        ),
        (
            RAIL_BUILDING + cbuilding::LEVEL,
            0_i16.to_le_bytes().to_vec(),
            Some(0),
        ),
        (
            RAIL_BUILDING + cbuilding::LEVEL,
            5_i16.to_le_bytes().to_vec(),
            Some(5),
        ),
    ] {
        let mut bytes = standing();
        put(&mut bytes, at, &value);
        let mut rows = [Row::default(); 1];
        assert_eq!(
            capture(bytes, None, &mut rows, &mut []).outcome,
            Outcome::Rows { len: 1 },
            "mutation at {at:#x}"
        );
        assert_eq!(rows[0].railway_building_level, expected);
    }
}

#[test]
fn rail_faults_and_bad_backlinks_refuse_the_hour() {
    assert_eq!(
        outcome(fixture(), Some(RAIL_BUILDING + cbuilding::LEVEL)),
        Outcome::Unavailable {
            reason: "railway gun rail building level unreadable",
        },
    );
    for (at, reason) in [
        (
            RAIL_BUILDING + cbuilding::BUILDING_STATUS,
            "railway gun rail building identity mismatch",
        ),
        (
            RAIL_INFO + rail_info::PROVINCE_RECORD,
            "railway gun rail-info province backlink mismatch",
        ),
        (RAIL_MANAGER, "railway gun rail manager vtable mismatch"),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn unlocated_gun_needs_no_rail_manager() {
    let mut bytes = standing();
    put64(&mut bytes, GUN + cunit::LOCATION, 0);
    put64(&mut bytes, RAIL_MANAGER, 0);
    let mut rows = [Row::default(); 1];
    assert_eq!(
        capture(bytes, None, &mut rows, &mut []).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        (rows[0].province, rows[0].railway_building_level),
        (0, None)
    );
}

#[test]
fn null_roster_slot_refuses_the_dense_native_census() {
    let mut bytes = fixture();
    put64(&mut bytes, ROSTER, 0);
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "railway gun roster has a null counted slot",
        },
    );
}

#[test]
fn malformed_identity_owner_and_location_fail_instead_of_relabelling() {
    for (at, value, reason) in [
        (
            GUN + cunit::CLASS,
            0_u32.to_le_bytes().to_vec(),
            "counter unit identity invalid",
        ),
        (
            GUN + 0x10,
            0_u64.to_le_bytes().to_vec(),
            "railway gun class or secondary identity mismatch",
        ),
        (
            GUN + cunit::ROSTER_OWNER,
            0_u32.to_le_bytes().to_vec(),
            "country id is zero",
        ),
        (
            PROVINCES + 8,
            0_u64.to_le_bytes().to_vec(),
            "unit province differs from its registered slot",
        ),
        (
            PROVINCES + 16,
            0_u64.to_le_bytes().to_vec(),
            "unit province differs from its registered slot",
        ),
        (
            bucket(GUN_HANDLE) + reference_bucket::OBJECT,
            0_u64.to_le_bytes().to_vec(),
            "counter unit registry identity mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, at, &value);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "mutation at {at:#x}",
        );
    }
}

#[test]
fn short_arenas_truncate_with_demand_and_bad_routes_never_become_stationary() {
    let walk = capture(fixture(), None, &mut [Row::default(); 1], &mut [0; 2]);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(walk.required_paths, 3);
    let walk = capture(fixture(), None, &mut [], &mut [0; 3]);
    assert_eq!(
        (walk.outcome, walk.required_paths),
        (
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            3
        )
    );
    assert_eq!(
        outcome(fixture(), Some(ORDINARY)),
        Outcome::Unavailable {
            reason: "unit ordinary route unreadable",
        },
    );
    let mut bytes = fixture();
    put(
        &mut bytes,
        GUN + cunit::ORDINARY_ROUTE_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster",
        },
    );
    // Three provinces in the roster: a four-hop route cannot be a pathfinder route.
    let mut bytes = fixture();
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, ORDINARY, 4);
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster",
        },
    );
}

#[test]
fn first_combat_side_follows_attacker_membership_and_needs_both_identities() {
    let mut bytes = fixture();
    // Another country on the attacking side of its only combat: the gun defends.
    put(&mut bytes, SIDE_COUNTRY, &0_u32.to_le_bytes());
    put(
        &mut bytes,
        GUN + cunit::COMBAT_REFERENCE_COUNT,
        &1_u32.to_le_bytes(),
    );
    let mut rows = [Row::default(); 1];
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut [0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (2, 2));
    assert_eq!((rows[0].combat_count, rows[0].attacking), (1, false));
    // Defending in the first combat but attacking in the second: the action code's
    // walk finds the second, while the stack kind keeps the first combat's side.
    put(
        &mut bytes,
        GUN + cunit::COMBAT_REFERENCE_COUNT,
        &2_u32.to_le_bytes(),
    );
    put64(&mut bytes, COMBATS + 8, COMBAT_TWO as u64);
    put64(&mut bytes, COMBAT_TWO, cland_combat::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        COMBAT_TWO + cland_combat::ATTACKER,
        SIDE_TWO as u64,
    );
    put64(&mut bytes, COMBAT_TWO + cland_combat::PROVINCE, FROM as u64);
    put64(&mut bytes, SIDE_TWO, cland_combatant::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        SIDE_TWO + cland_combatant::COMBAT,
        COMBAT_TWO as u64,
    );
    put64(
        &mut bytes,
        SIDE_TWO + cland_combatant::COUNTRIES,
        SIDE_TWO_COUNTRY as u64,
    );
    put(&mut bytes, SIDE_TWO_COUNTRY, &1_u32.to_le_bytes());
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut [0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (2, 2));
    assert_eq!((rows[0].combat_count, rows[0].attacking), (2, true));
    // A later combat's province is never read: an unregistered one does not fail the gun.
    put64(&mut bytes, COMBAT_TWO + cland_combat::PROVINCE, 0x10);
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut [0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_province, rows[0].attacking), (2, true));
    // A later combat that cannot be read fails the gun rather than reading as defence.
    put64(&mut bytes, COMBAT_TWO, 0x10);
    assert_eq!(
        capture(bytes, None, &mut rows, &mut [0; 3]).outcome,
        Outcome::Unavailable {
            reason: "division first combat identity mismatch"
        }
    );
    for (at, reason) in [
        (COMBAT, "division first combat identity mismatch"),
        (SIDE, "first combat side identity mismatch"),
        (
            SIDE + cland_combatant::COMBAT,
            "first combat side identity mismatch",
        ),
        (
            COMBAT + cland_combat::PROVINCE,
            "unit province vtable mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0x10);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn the_first_combat_answer_is_shared_through_the_capture_memo() {
    let mut memo = cunit::FirstCombatMemo::new(4);
    let memory = MemoryFixture {
        bytes: fixture(),
        fault: None,
    };
    let mut rows = [Row::default(); 1];
    let walk = walk(&memory, 0, STATE, &mut rows, &mut [0; 3], &mut memo);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(memo.entries.len(), 1);
    // A second unit of the same owner in the same combat is answered from the memo, even
    // with the combat's side no longer readable.
    let mut bytes = fixture();
    put64(&mut bytes, SIDE, 0);
    let memory = MemoryFixture { bytes, fault: None };
    assert_eq!(
        super::walk(&memory, 0, STATE, &mut rows, &mut [0; 3], &mut memo).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (1, 2));
    // A failed answer is never memoised.
    let mut fresh = cunit::FirstCombatMemo::new(4);
    assert!(matches!(
        super::walk(&memory, 0, STATE, &mut rows, &mut [0; 3], &mut fresh).outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(fresh.entries.is_empty());
}

#[test]
fn assignee_must_be_a_registered_orders_group_subobject() {
    for (at, reason) in [
        (
            GROUP + crailway_gun::ASSIGNEE_BASE,
            "railway gun assignee secondary vtable mismatch",
        ),
        (
            GROUP + corders_group::HANDLE,
            "plan group reference type mismatch",
        ),
        (
            bucket_at(GROUP_HANDLE, GROUP_BUCKETS) + reference_bucket::OBJECT,
            "railway gun assignee registry identity mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn unreadable_operands_are_not_defaulted() {
    for (at, reason) in [
        (
            GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP,
            "railway gun naval transfer unreadable",
        ),
        (
            TRANSFER + cnaval_unit_transfer::ACTION_STATUS,
            "railway gun naval transfer action unreadable",
        ),
        (GUN + cunit::EXILED, "railway gun exile byte unreadable"),
        (GUN + cunit::RETREAT, "railway gun retreat byte unreadable"),
        (
            GUN + cunit::STRATEGIC_REDEPLOYMENT,
            "railway gun strategic redeployment unreadable",
        ),
        (
            GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
            "railway gun reference entry count unreadable",
        ),
        (
            GUN + crailway_gun::ASSIGNEE,
            "railway gun assignee pointer unreadable",
        ),
        (
            SUBUNIT + csub_unit_definition::MAXIMUM_STRENGTH,
            "railway gun maximum strength unreadable",
        ),
        (
            GUN + crailway_gun::CURRENT_STRENGTH,
            "railway gun current strength unreadable",
        ),
        (
            GUN + cunit::RECEIVED_SUPPLY_RATIO,
            "railway gun received supply ratio unreadable",
        ),
    ] {
        assert_eq!(
            outcome(fixture(), Some(at)),
            Outcome::Unavailable { reason },
            "fault at {at:#x}",
        );
    }
}

#[test]
fn naval_transfer_and_stat_block_need_their_identities() {
    for (at, reason) in [
        (TRANSFER, "railway gun naval transfer vtable mismatch"),
        (
            TRANSFER + cnaval_unit_transfer::HANDLE,
            "railway gun naval transfer handle invalid",
        ),
        (
            bucket(TRANSFER_HANDLE) + reference_bucket::OBJECT,
            "railway gun naval transfer registry mismatch",
        ),
        (SUBUNIT, "railway gun stat block vtable mismatch"),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}
