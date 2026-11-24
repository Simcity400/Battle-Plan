use super::*;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x140000000;
const GAME: usize = 0x100;
const STATE: usize = 0x1000;
const PROVINCE: usize = 0x2000;
const BUILDING: usize = 0x3000;
const TEMPLATE: usize = 0x3800;
const STATES: usize = 0x4000;
const PROVINCES: usize = 0x4100;
const INDICES: usize = 0x4200;
const BUILDINGS: usize = 0x4300;
const SOURCE: usize = 0x5000;
const WING: usize = 0x6000;
const POOL: usize = 0x7000;
const OWNER: usize = 0x8000;
const TYPE: u32 = 0xaabbcc55;

fn u64_at(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn array_at(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64_at(bytes, at, data);
    u32_at(bytes, at + 8, count);
    u32_at(bytes, at + 12, count);
}

fn health_at(bytes: &mut [u8], total: i16, healthy: i16, partial: i64) {
    bytes[BUILDING + cbuilding::LEVEL..BUILDING + cbuilding::LEVEL + 2]
        .copy_from_slice(&total.to_le_bytes());
    bytes[BUILDING + cbuilding::HEALTHY_LEVEL..BUILDING + cbuilding::HEALTHY_LEVEL + 2]
        .copy_from_slice(&healthy.to_le_bytes());
    bytes[BUILDING + cbuilding::PARTIAL_HEALTH..BUILDING + cbuilding::PARTIAL_HEALTH + 8]
        .copy_from_slice(&partial.to_le_bytes());
}

fn fixture(province_scope: bool) -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    let (object, offset, id, scope) = if province_scope {
        (
            PROVINCE,
            cprovince::BUILDING_STATUS,
            4,
            cbuilding_status::PROVINCE_FLAG,
        )
    } else {
        (
            STATE,
            cstate::BUILDING_STATUS,
            3,
            cbuilding_status::STATE_FLAG,
        )
    };
    let status = object + offset;
    for (address, value) in [
        (STATE, BASE + cstate::VTABLE_RVA.0),
        (PROVINCE, BASE + cprovince::VTABLE_RVA.0),
        (PROVINCE + cprovince::STATE, STATE),
        (BUILDING, BASE + cbuilding::VTABLE_RVA.0),
        (BUILDING + cbuilding::BUILDING_STATUS, status),
        (BUILDING + cbuilding::TEMPLATE, TEMPLATE),
        (status, BASE + cbuilding_status::VTABLE_RVA.0),
        (STATES + 3 * 8, STATE),
        (PROVINCES + 4 * 8, PROVINCE),
        (BUILDINGS, BUILDING),
        (SOURCE + SOURCE_WING, WING),
        (WING, BASE + engine_native_parts::cair_wing_layout::VTABLE_RVA.0),
        (WING + engine_native_parts::cair_wing_layout::POOL, POOL),
        (POOL, BASE + cair_wing_pool::VTABLE_RVA.0),
        (POOL + cair_wing_pool::OWNER, OWNER),
        (OWNER, BASE + cstrategic_air::VTABLE_RVA.0),
    ] {
        u64_at(&mut bytes, address, value);
    }
    for (address, value) in [
        (STATE + cstate::ID, 3),
        (STATE + cstate::CONTROLLER, 5001),
        (PROVINCE + cprovince::ID, 4),
        (PROVINCE + cprovince::CONTROLLER, 5002),
        (BUILDING + cbuilding::TYPE_ID, TYPE),
        (TEMPLATE + cbuilding_template::TYPE_ID, TYPE),
        (TEMPLATE + cbuilding_template::DATABASE_ORDINAL, 5),
        (status + cbuilding_status::FLAGS, scope),
        (status + cbuilding_status::ID, id),
        (status + cbuilding_status::COUNTRY, 9999),
        (SOURCE, AIR_SOURCE_KIND),
        (WING + cair_wing::ID, 71),
        (OWNER + cstrategic_air::COUNTRY, 6001),
    ] {
        u32_at(&mut bytes, address, value);
    }
    array_at(&mut bytes, GAME + cgame_state::STATES, STATES, 8);
    array_at(&mut bytes, GAME + cgame_state::PROVINCES, PROVINCES, 8);
    array_at(
        &mut bytes,
        status + cbuilding_status::BUILDING_INDICES,
        INDICES,
        9,
    );
    array_at(
        &mut bytes,
        status + cbuilding_status::BUILDINGS,
        BUILDINGS,
        1,
    );
    health_at(&mut bytes, 5, 2, 4_000_000);
    bytes
}

fn capture(bytes: &[u8], caller: usize) -> Result<Snapshot, &'static str> {
    let before = snapshot(
        &SliceMemory { base: 0, bytes },
        BASE,
        GAME,
        BUILDING,
        SOURCE,
        BASE + caller,
        12345,
    )?;
    attribution(&SliceMemory { base: 0, bytes }, BASE, &before)
}

#[test]
fn snapshot_preserves_mod_token_and_credits_controller_in_both_scopes() {
    assert!(std::mem::size_of::<Snapshot>() <= 128);
    for province_scope in [false, true] {
        let bytes = fixture(province_scope);
        let result = capture(&bytes, CALLERS[0].0.0).unwrap();
        assert_eq!(result.building_type, TYPE);
        assert_eq!(result.attacker, Some(6001));
        assert_eq!(result.victim, if province_scope { 5002 } else { 5001 });
        assert_eq!(result.state, 3);
        assert_eq!(result.province, province_scope.then_some(4));
        assert_eq!(result.health_raw, 24_000_000);
        assert_eq!(result.game_hour, 12345);
        assert_eq!(result.native_cause, 1);
        assert_eq!(result.cause, Cause::AirMission);
    }
}

#[test]
fn neutral_controller_is_preserved_without_changing_positive_damage() {
    for province_scope in [false, true] {
        let mut bytes = fixture(province_scope);
        let (object, offset) = if province_scope {
            (PROVINCE, cprovince::CONTROLLER)
        } else {
            (STATE, cstate::CONTROLLER)
        };
        u32_at(&mut bytes, object + offset, 0);
        let before = capture(&bytes, CALLERS[0].0.0).unwrap();
        assert_eq!(before.victim, 0);
        health_at(&mut bytes, 5, 2, 3_000_000);
        assert_eq!(
            applied_damage(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                &before
            ),
            Ok(1_000_000)
        );
    }
}

#[test]
fn effective_country_uses_positive_override_otherwise_pool_owner() {
    for (raw, expected) in [(7_i32, Some(7)), (0, Some(6001)), (-9, Some(6001))] {
        let mut bytes = fixture(false);
        u32_at(&mut bytes, WING + cair_wing::COUNTRY_OVERRIDE, raw as u32);
        assert_eq!(capture(&bytes, CALLERS[1].0.0).unwrap().attacker, expected);
    }
    let mut bytes = fixture(false);
    u32_at(&mut bytes, WING + cair_wing::COUNTRY_OVERRIDE, 7001);
    u64_at(&mut bytes, WING + engine_native_parts::cair_wing_layout::POOL, 0);
    assert_eq!(
        capture(&bytes, CALLERS[1].0.0).unwrap().attacker,
        Some(7001)
    );
    u32_at(&mut bytes, WING + cair_wing::COUNTRY_OVERRIDE, 0);
    assert_eq!(capture(&bytes, CALLERS[1].0.0).unwrap().attacker, None);
}

#[test]
fn null_air_wing_and_nonair_or_unknown_sources_stay_null() {
    let mut bytes = fixture(false);
    u64_at(&mut bytes, SOURCE + SOURCE_WING, 0);
    assert_eq!(capture(&bytes, CALLERS[0].0.0).unwrap().attacker, None);
    for kind in [0, 2, 3, 99] {
        u32_at(&mut bytes, SOURCE, kind);
        u64_at(&mut bytes, SOURCE + SOURCE_WING, usize::MAX);
        for (caller, expected) in CALLERS {
            let result = capture(&bytes, caller.0).unwrap();
            assert_eq!(
                (result.native_cause, result.cause, result.attacker),
                (kind, expected, None)
            );
        }
        let result = capture(&bytes, 0x23456).unwrap();
        assert_eq!(
            (result.native_cause, result.cause, result.attacker),
            (kind, Cause::Unknown, None)
        );
    }
}

#[test]
fn post_read_uses_actual_health_delta_including_partial_level_and_clamp() {
    let mut bytes = fixture(false);
    let before = capture(&bytes, CALLERS[0].0.0).unwrap();
    health_at(&mut bytes, 5, 1, 1_000_000);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Ok(13_000_000)
    );
    health_at(&mut bytes, 5, 0, 0);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Ok(24_000_000)
    );
    health_at(&mut bytes, 5, 2, 4_000_000);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Ok(0)
    );
    health_at(&mut bytes, 5, 3, 0);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Ok(-6_000_000)
    );
    health_at(&mut bytes, 6, 2, 0);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Err("building damage total level changed during call")
    );
    health_at(&mut bytes, 5, 1, 0);
    u32_at(&mut bytes, BUILDING + cbuilding::TYPE_ID, TYPE + 1);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            BASE,
            &before
        ),
        Err("building damage identity changed during call")
    );
}

#[test]
fn capture_refuses_identity_health_bounds_and_short_reads_by_first_failure() {
    let status = STATE + cstate::BUILDING_STATUS;
    let mut cases = Vec::new();
    for (province, address, value, reason) in [
        (
            false,
            BUILDING,
            0_u64.to_le_bytes().to_vec(),
            "building damage vtable mismatch",
        ),
        (
            false,
            status,
            0_u64.to_le_bytes().to_vec(),
            "building damage status vtable mismatch",
        ),
        (
            false,
            status + cbuilding_status::FLAGS,
            0_u32.to_le_bytes().to_vec(),
            "building damage status scope unsupported",
        ),
        (
            false,
            STATE,
            0_u64.to_le_bytes().to_vec(),
            "building damage location vtable mismatch",
        ),
        (
            false,
            STATE + cstate::ID,
            0_u32.to_le_bytes().to_vec(),
            "building damage location identity mismatch",
        ),
        (
            false,
            TEMPLATE + cbuilding_template::TYPE_ID,
            0_u32.to_le_bytes().to_vec(),
            "building damage template type mismatch",
        ),
        (
            false,
            BUILDINGS,
            0_u64.to_le_bytes().to_vec(),
            "building damage instance differs from status slot",
        ),
        (
            false,
            TEMPLATE + cbuilding_template::DATABASE_ORDINAL,
            99_u32.to_le_bytes().to_vec(),
            "building damage template outside indices",
        ),
        (
            false,
            status + cbuilding_status::BUILDING_INDICES + 12,
            5_u32.to_le_bytes().to_vec(),
            "building damage template outside indices",
        ),
        (
            false,
            INDICES + 5 * 4,
            u32::MAX.to_le_bytes().to_vec(),
            "building damage index outside vector",
        ),
        (
            true,
            STATES + 3 * 8,
            0_u64.to_le_bytes().to_vec(),
            "building damage province state identity mismatch",
        ),
    ] {
        let mut bytes = fixture(province);
        bytes[address..address + value.len()].copy_from_slice(&value);
        cases.push((bytes, Some(reason)));
    }
    for (total, healthy, partial) in [(1, 2, 0), (1, 0, -1), (-1, 0, 0)] {
        let mut bytes = fixture(false);
        health_at(&mut bytes, total, healthy, partial);
        cases.push((bytes, None));
    }
    for end in [
        BUILDING + cbuilding::PARTIAL_HEALTH + 7,
        TEMPLATE + cbuilding_template::DATABASE_ORDINAL + 3,
        SOURCE + 3,
    ] {
        let mut bytes = fixture(false);
        bytes.truncate(end);
        cases.push((bytes, None));
    }
    for (bytes, reason) in cases {
        let actual = capture(&bytes, CALLERS[0].0.0).unwrap_err();
        if let Some(reason) = reason {
            assert_eq!(actual, reason);
        }
    }
}

#[test]
fn negative_modified_damage_preserves_health_above_a_full_level() {
    let mut bytes = fixture(false);
    health_at(&mut bytes, 1, 0, 10_000_000);
    let before = capture(&bytes, CALLERS[0].0.0).unwrap();
    health_at(&mut bytes, 1, 0, 11_000_000);
    assert_eq!(
        applied_damage(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            BASE,
            &before,
        ),
        Ok(-1_000_000)
    );
    assert_eq!(
        capture(&bytes, CALLERS[0].0.0).unwrap().health_raw,
        11_000_000
    );
}

#[test]
fn attribution_read_failures_are_distinct_from_native_absence() {
    for address in [WING, WING + cair_wing::ID, POOL, OWNER] {
        let mut bytes = fixture(false);
        bytes[address..address + 8].fill(0);
        let row = capture(&bytes, CALLERS[0].0.0).unwrap();
        assert_eq!(row.attacker, None);
        assert!(row.attribution_failed);
    }
    for (address, value) in [(SOURCE, 0_u64), (SOURCE + SOURCE_WING, 0)] {
        let mut bytes = fixture(false);
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        let row = capture(&bytes, CALLERS[0].0.0).unwrap();
        assert_eq!(row.attacker, None);
        assert!(!row.attribution_failed);
    }
    assert!(
        !capture(&fixture(false), CALLERS[0].0.0)
            .unwrap()
            .attribution_failed
    );
}

#[test]
fn native_zero_ordinal_and_absent_province_state_are_valid() {
    let mut bytes = fixture(false);
    u32_at(
        &mut bytes,
        TEMPLATE + cbuilding_template::DATABASE_ORDINAL,
        0,
    );
    assert!(capture(&bytes, CALLERS[0].0.0).is_ok());
    let mut bytes = fixture(true);
    u64_at(&mut bytes, PROVINCE + cprovince::STATE, 0);
    assert_eq!(capture(&bytes, CALLERS[0].0.0).unwrap().state, 0);
}

struct CensusMemory<'a> {
    bytes: &'a [u8],
    game_state: usize,
}

impl Memory for CensusMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == BASE + cgame_state::SINGLETON_RVA.0 && destination.len() == 8 {
            destination.copy_from_slice(&(self.game_state as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

#[test]
fn capacity_census_sizes_from_every_live_location_and_refuses_missing() {
    let mut bytes = fixture(false);
    let status = PROVINCE + cprovince::BUILDING_STATUS;
    u64_at(&mut bytes, status, BASE + cbuilding_status::VTABLE_RVA.0);
    u32_at(
        &mut bytes,
        status + cbuilding_status::FLAGS,
        cbuilding_status::PROVINCE_FLAG,
    );
    u32_at(&mut bytes, status + cbuilding_status::ID, 4);
    array_at(&mut bytes, status + cbuilding_status::BUILDINGS, 0, 0);
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes,
                game_state: GAME
            },
            BASE
        ),
        Some(1)
    );
    array_at(
        &mut bytes,
        status + cbuilding_status::BUILDINGS,
        BUILDINGS,
        7,
    );
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes,
                game_state: GAME
            },
            BASE
        ),
        Some(8)
    );
    u64_at(&mut bytes, BUILDINGS, usize::MAX);
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes,
                game_state: GAME
            },
            BASE
        ),
        Some(8)
    );
    for (at, width) in [
        (STATE, 8),
        (STATE + cstate::ID, 4),
        (status, 8),
        (status + cbuilding_status::FLAGS, 4),
    ] {
        let mut broken = bytes.clone();
        broken[at..at + width].fill(0);
        assert_eq!(
            building_count(
                &CensusMemory {
                    bytes: &broken,
                    game_state: GAME
                },
                BASE
            ),
            None
        );
    }
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes,
                game_state: 0
            },
            BASE
        ),
        None
    );
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes[..STATE],
                game_state: GAME
            },
            BASE
        ),
        None
    );
    u32_at(&mut bytes, status + cbuilding_status::BUILDINGS + 8, 6);
    assert_eq!(
        building_count(
            &CensusMemory {
                bytes: &bytes,
                game_state: GAME
            },
            BASE
        ),
        None
    );
}

#[test]
fn exhausted_health_skips_source_reads_but_positive_health_reads_source() {
    let mut bytes = fixture(false);
    health_at(&mut bytes, 5, 0, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        snapshot(
            &memory,
            BASE,
            GAME,
            BUILDING,
            usize::MAX,
            BASE + CALLERS[0].0.0,
            12345
        )
        .unwrap()
        .health_raw,
        0
    );
    health_at(&mut bytes, 5, 0, 1);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        snapshot(
            &memory,
            BASE,
            GAME,
            BUILDING,
            SOURCE,
            BASE + CALLERS[0].0.0,
            12345
        )
        .unwrap()
        .health_raw,
        1
    );
}

#[test]
fn post_attribution_uses_controller_at_return_and_noop_skips_location() {
    let mut bytes = fixture(false);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let before = snapshot(
        &memory,
        BASE,
        GAME,
        BUILDING,
        SOURCE,
        BASE + CALLERS[0].0.0,
        12345,
    )
    .unwrap();
    u32_at(&mut bytes, STATE + cstate::CONTROLLER, 7001);
    health_at(&mut bytes, 5, 1, 0);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(applied_damage(&memory, BASE, &before), Ok(14_000_000));
    assert_eq!(attribution(&memory, BASE, &before).unwrap().victim, 7001);
    u64_at(&mut bytes, STATE, 0);
    health_at(&mut bytes, 5, 2, 4_000_000);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(applied_damage(&memory, BASE, &before), Ok(0));
    assert!(attribution(&memory, BASE, &before).is_err());
}
