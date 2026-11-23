use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;
const TASK: usize = 0x100;
const STATS: usize = 0x900;
fn put(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn visit() -> cfleet::TaskForceVisit {
    cfleet::TaskForceVisit {
        country: engine_native::ccountry::Country {
            id: 1,
            canonical_slot: 1,
            pointer: 1,
        },
        task: TASK,
        id: 61 | (5_u64 << 32),
        fleet: 1,
        fleet_handle: 61 | (4_u64 << 32),
        leader: 0,
        kind: 1,
    }
}
fn bytes(target: u64) -> Vec<u8> {
    let mut b = vec![0; 0x1200];
    let m = TASK + ctask_force::NAVAL_MISSION;
    put(&mut b, m, cnaval_mission::VTABLE_RVA.0 as u64);
    put(&mut b, m + 8, TASK as u64);
    put(&mut b, m + TARGET, target);
    put(&mut b, m + PROGRESS, (-123_i64) as u64);
    put(&mut b, m + CHANCE, 456);
    put(&mut b, TASK + cunit::STAT_BLOCK, STATS as u64);
    put(&mut b, STATS + SURFACE_DETECTION, 777);
    put(&mut b, STATS + CARRIER_DETECTION, (-88_i64) as u64);
    b
}
struct Trace<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<usize>>,
    deny: Option<usize>,
}
impl Memory for Trace<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(at);
        self.deny != Some(at) && self.memory.copy(at, out)
    }
}
#[test]
fn null_target_does_not_reach_progress_chance_or_resolver() {
    let b = bytes(0);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: None,
    };
    let row = read_with(&memory, 0, visit(), None, None, false, |_| {
        panic!("null target must not resolve")
    });
    assert_eq!(
        row,
        Operands {
            target: Some(0),
            target_live: Some(0),
            surface_detection: Some(777),
            carrier_detection: Some(-88),
            submarine_detection: Some(0),
            carrier_submarine_detection: Some(0),
            ..Default::default()
        }
    );
    let reads = memory.reads.borrow();
    assert!(!reads.contains(&(TASK + ctask_force::NAVAL_MISSION + PROGRESS)));
    assert!(!reads.contains(&(TASK + ctask_force::NAVAL_MISSION + CHANCE)));
}
#[test]
fn independent_leaf_refusal_keeps_target_chance_and_carrier_stat() {
    let b = bytes(99);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: Some(TASK + ctask_force::NAVAL_MISSION + PROGRESS),
    };
    let row = read_with(&memory, 0, visit(), None, None, false, |raw| {
        assert_eq!(raw, 99);
        Ok(Some(0x800))
    });
    assert_eq!(row.target, Some(99));
    assert_eq!(row.target_live, Some(1));
    assert_eq!(row.progress, None);
    assert_eq!(row.chance, Some(456));
    assert_eq!(row.carrier_detection, Some(-88));
}
#[test]
fn original_failed_header_is_never_retried_and_stats_remain_independent() {
    let b = bytes(99);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: None,
    };
    let row = read_with(
        &memory,
        0,
        visit(),
        Some(Err("first header refused")),
        None,
        false,
        |_| panic!("unreached"),
    );
    assert_eq!(row.target, None);
    assert_eq!(row.surface_detection, Some(777));
    assert!(
        !memory
            .reads
            .borrow()
            .contains(&(TASK + ctask_force::NAVAL_MISSION))
    );
}
#[test]
fn unresolved_and_failed_target_resolution_remain_distinct() {
    let b = bytes(99);
    let memory = SliceMemory { base: 0, bytes: &b };
    assert_eq!(
        read_with(&memory, 0, visit(), None, None, false, |_| Ok(None)).target_live,
        Some(0)
    );
    assert_eq!(
        read_with(&memory, 0, visit(), None, None, false, |_| Err(
            "registry failed"
        ))
        .target_live,
        None
    );
}

#[test]
fn convoy_target_demand_reaches_progress_with_no_patrol_target() {
    let b = bytes(0);
    let memory = SliceMemory { base: 0, bytes: &b };
    let row = read_with(&memory, 0, visit(), None, None, true, |_| {
        panic!("no patrol target")
    });
    assert_eq!(row.target, Some(0));
    assert_eq!(row.progress, Some(-123));
    assert_eq!(row.chance, Some(456));
    assert_eq!(row.radar_cache, Some(0));
    assert_eq!(row.air_cache, Some(0));
    assert_eq!(row.surface_visibility, Some(0));
    assert_eq!(row.submarine_visibility, Some(0));
}
#[test]
fn borrowed_stat_selector_is_not_read_again() {
    let b = bytes(0);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: Some(TASK + cunit::STAT_BLOCK),
    };
    let row = read_with(&memory, 0, visit(), None, Some(Ok(STATS)), false, |_| {
        panic!("no patrol target")
    });
    assert_eq!(row.surface_detection, Some(777));
    assert!(!memory.reads.borrow().contains(&(TASK + cunit::STAT_BLOCK)));
}
#[test]
fn null_location_has_no_local_modifier_demand_and_empty_hq_is_known() {
    let b = bytes(0);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: None,
    };
    let context = read_context(&memory, 0, visit(), engine_native_memory::walk::Buffer::Count);
    assert_eq!(context.region, Some(0));
    assert_eq!(context.headquarters, Ok(0));
    assert_eq!(context.local_detection, Some(Default::default()));
    assert!(
        !memory
            .reads
            .borrow()
            .contains(&(TASK + crate::naval_mission_inputs::LOCAL_MODIFIERS))
    );
}

#[test]
fn original_failed_stat_and_location_selectors_are_never_retried() {
    let b = bytes(0);
    let memory = Trace {
        memory: SliceMemory { base: 0, bytes: &b },
        reads: Default::default(),
        deny: None,
    };
    let row = read_with(
        &memory,
        0,
        visit(),
        None,
        Some(Err("first selector refused")),
        false,
        |_| panic!("null target"),
    );
    assert_eq!(row.surface_detection, None);
    assert!(!memory.reads.borrow().contains(&(TASK + cunit::STAT_BLOCK)));
    let context = read_context_with_location(
        &memory,
        0,
        visit(),
        Some(Err("first location refused")),
        engine_native_memory::walk::Buffer::Count,
    );
    assert_eq!(context.region, None);
    assert_eq!(context.local_detection, None);
    assert_eq!(context.headquarters, Ok(0));
    assert!(!memory.reads.borrow().contains(&(TASK + cunit::LOCATION)));
}
