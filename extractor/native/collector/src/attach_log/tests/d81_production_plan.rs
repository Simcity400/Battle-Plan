use super::*;

use crate::attach_log::d81_copy_bracket::{
    domain81_copy_plan, CountingSnapshotMemory, D81PlanCounters, Domain81PlanRoots, D81_CONSUMER,
};
use crate::attach_log::d81_vector_store::{D81Family, VectorAnswer, VectorImage};
use crate::attach_log::{d81_observe_copy_plan_with, D81PlanObservation};
use crate::snapshot_recipes::executor::validate;
use crate::snapshot_recipes::synthetic::SyntheticMemory;
use crate::snapshot_recipes::view::ViewRefusal;
use crate::snapshot_recipes::{HourStamp, Span};

const PROVINCES: u32 = 512;
const RAIL_EVERY: u32 = 3;
const PAGE: u64 = crate::snapshot_recipes::SCOPED_VOID_GRANULARITY_BYTES;

const BASE: u64 = 0x0000_0200_0000_0000;

const GAMESTATE: u64 = BASE;
const IDLER_SLOT: u64 = BASE + 0x0001_0000;
const IDLER_SLOT_B: u64 = BASE + 0x0001_8000;
const IDLER: u64 = BASE + 0x0002_0000;
const OWNER: u64 = BASE + 0x0003_0000;
const CONTAINER: u64 = BASE + 0x0004_0000;
const RAIL_MANAGER: u64 = BASE + 0x0005_0000;
const RAIL_TABLE: u64 = BASE + 0x0006_0000;
const ACTIVE_ROOT: u64 = BASE + 0x0010_0000;
const ACTIVE_TABLE: u64 = BASE + 0x0011_0000;
const SELECTOR_MAP: u64 = BASE + 0x0020_0000;
const SELECTOR_TABLE: u64 = BASE + 0x0021_0000;
const SOURCE_ARRAY: u64 = BASE + 0x0030_0000;
const MAP_ARRAY: u64 = BASE + 0x0031_0000;

const RAIL_INFO: u64 = BASE + 0x1000_0000;
const PAIR_BLOB: u64 = BASE + 0x2000_0000;
const MAP_PROVINCE: u64 = BASE + 0x3000_0000;
const RECORD: u64 = BASE + 0x4000_0000;
const SOURCE_PROVINCE: u64 = BASE + 0x5000_0000;
const SCATTER_STRIDE: u64 = 0x0002_0000;

const SELECTOR_MASK: u32 = PROVINCES - 1;
const SELECTOR_MAX_DISTANCE: u8 = 2;
const SELECTOR_SLOTS: u32 = SELECTOR_MASK + 1 + SELECTOR_MAX_DISTANCE as u32 + 1;

const REJECTED_PROVINCE: u32 = 9;
const VOID_PAGE_START: u64 = PAGE;

const MAP_PROVINCE_LEN: usize = crate::attach_log::D81_ANCHOR_SECONDARY_OFF + 12 + 0x20;

fn railed(province: u32) -> bool {
    province % RAIL_EVERY == 0
}

fn rail_info_of(province: u32) -> u64 {
    RAIL_INFO + u64::from(province) * SCATTER_STRIDE
}
fn pair_blob_of(province: u32) -> u64 {
    PAIR_BLOB + u64::from(province) * SCATTER_STRIDE
}
fn map_province_of(province: u32) -> u64 {
    MAP_PROVINCE + u64::from(province) * SCATTER_STRIDE
}
fn record_of(province: u32) -> u64 {
    RECORD + u64::from(province) * SCATTER_STRIDE
}
fn source_province_of(province: u32) -> u64 {
    SOURCE_PROVINCE + u64::from(province) * SCATTER_STRIDE
}

fn voided(province: u32) -> bool {
    let at = u64::from(province) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64;
    (VOID_PAGE_START..VOID_PAGE_START + PAGE).contains(&at)
}

fn selector_slot_of(key: u32, mask: u32) -> u32 {
    let mut hash = (key ^ (key >> 16)).wrapping_mul(0x045d_9f3b);
    hash = (hash ^ (hash >> 16)).wrapping_mul(0x045d_9f3b);
    hash ^= hash >> 16;
    hash & mask
}

fn selector_key_of(province: u32) -> u32 {
    province
}

fn selector_value_of(province: u32) -> u64 {
    0xA000_0000 + u64::from(province)
}

fn selector_placement() -> Vec<Option<u32>> {
    let mut occupied = vec![false; (SELECTOR_SLOTS + 8) as usize];
    let mut placement = vec![None; PROVINCES as usize];
    for province in 0..PROVINCES {
        let start = selector_slot_of(selector_key_of(province), SELECTOR_MASK);
        for step in 0..=u32::from(SELECTOR_MAX_DISTANCE) {
            let slot = start + step;
            if !occupied[slot as usize] {
                occupied[slot as usize] = true;
                placement[province as usize] = Some(slot);
                break;
            }
        }
    }
    placement
}

fn anchor_of(province: u32, secondary: bool) -> [u32; 3] {
    let bias = if secondary { 0x1000 } else { 0 };
    [
        province + bias,
        province + 100 + bias,
        province + 200 + bias,
    ]
}

fn stage_world() -> SyntheticMemory {
    use crate::attach_log::*;
    let mut memory = SyntheticMemory::new();

    memory.map_zeroed(GAMESTATE, D81_GS_ANCHOR_SELECTOR_MAP_OFF + 8);
    memory.map_zeroed(IDLER_SLOT, 8);
    memory.map_zeroed(IDLER_SLOT_B, 8);
    memory.map_zeroed(IDLER, D81_IDLER_OWNER_OFF + 8);
    memory.map_zeroed(OWNER, D81_OWNER_CONTAINER_OFF + 8);
    memory.map_zeroed(CONTAINER, D81_CONTAINER_PROVINCE_COUNT_OFF + 4);
    memory.map_zeroed(RAIL_MANAGER, D81_RAIL_MANAGER_TABLE_OFF + 8);
    memory.map_zeroed(RAIL_TABLE, PROVINCES as usize * 8);
    memory.map_zeroed(ACTIVE_ROOT, D81_ACTIVE_ROOT_TABLE_OFF + 8);
    let active_bytes = PROVINCES as u64 * D81_ACTIVE_ENTRY_STRIDE as u64;
    assert!(
        active_bytes >= 3 * PAGE,
        "the fixture needs a readable page either side of the voided one",
    );
    memory.map_zeroed(ACTIVE_TABLE, VOID_PAGE_START as usize);
    memory.map_zeroed(ACTIVE_TABLE + VOID_PAGE_START, PAGE as usize);
    memory.map_zeroed(
        ACTIVE_TABLE + VOID_PAGE_START + PAGE,
        (active_bytes - VOID_PAGE_START - PAGE) as usize,
    );
    memory.map_zeroed(SELECTOR_MAP, D81_SELECTOR_MAP_MAX_DISTANCE_OFF + 1);
    memory.map_zeroed(
        SELECTOR_TABLE,
        (SELECTOR_SLOTS as usize + 8) * D81_SELECTOR_ENTRY_STRIDE,
    );
    memory.map_zeroed(SOURCE_ARRAY, PROVINCES as usize * 8);
    memory.map_zeroed(MAP_ARRAY, PROVINCES as usize * 8);

    memory.write_u64(GAMESTATE + D81_GS_RAIL_MANAGER_OFF as u64, RAIL_MANAGER);
    memory.write_u64(GAMESTATE + CGAMESTATE_CPROVINCE_ARRAY as u64, SOURCE_ARRAY);
    memory.write_u32(GAMESTATE + CGAMESTATE_CPROVINCE_COUNT as u64, PROVINCES);
    memory.write_u64(
        GAMESTATE + D81_GS_ACTIVE_PROVINCE_ROOT_OFF as u64,
        ACTIVE_ROOT,
    );
    memory.write_u64(
        GAMESTATE + D81_GS_ANCHOR_SELECTOR_MAP_OFF as u64,
        SELECTOR_MAP,
    );
    memory.write_u64(RAIL_MANAGER + D81_RAIL_MANAGER_TABLE_OFF as u64, RAIL_TABLE);
    memory.write_u64(ACTIVE_ROOT + D81_ACTIVE_ROOT_TABLE_OFF as u64, ACTIVE_TABLE);
    memory.write_u64(
        SELECTOR_MAP + D81_SELECTOR_MAP_TABLE_OFF as u64,
        SELECTOR_TABLE,
    );
    memory.write_u32(
        SELECTOR_MAP + D81_SELECTOR_MAP_MASK_OFF as u64,
        SELECTOR_MASK,
    );
    memory.write(
        SELECTOR_MAP + D81_SELECTOR_MAP_MAX_DISTANCE_OFF as u64,
        &[SELECTOR_MAX_DISTANCE],
    );

    memory.write_u64(IDLER_SLOT, IDLER);
    memory.write_u64(IDLER_SLOT_B, IDLER);
    memory.write_u64(IDLER + D81_IDLER_OWNER_OFF as u64, OWNER);
    memory.write_u64(OWNER + D81_OWNER_CONTAINER_OFF as u64, CONTAINER);
    memory.write_u64(
        CONTAINER + D81_CONTAINER_PROVINCE_ARRAY_OFF as u64,
        MAP_ARRAY,
    );
    memory.write_u32(
        CONTAINER + D81_CONTAINER_PROVINCE_COUNT_OFF as u64,
        PROVINCES,
    );

    for slot in 0..SELECTOR_SLOTS + 8 {
        let entry = SELECTOR_TABLE + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
        memory.write(
            entry + D81_SELECTOR_MARKER_OFF as u64,
            &[SELECTOR_MAX_DISTANCE + 1],
        );
        memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, u32::MAX);
    }

    let placement = selector_placement();

    for province in 0..PROVINCES {
        let index = u64::from(province);
        memory.write(
            ACTIVE_TABLE + index * D81_ACTIVE_ENTRY_STRIDE as u64 + D81_ACTIVE_WINDOW_OFFSET as u64,
            &active_window_of(province),
        );
        let source = source_province_of(province);
        memory.map_zeroed(source, crate::attach_log::CPROVINCE_OFF_ID + 0x20);
        memory.write_u32(
            source + crate::attach_log::CPROVINCE_OFF_ID as u64,
            province,
        );
        memory.write_u64(SOURCE_ARRAY + index * 8, source);
        let map = map_province_of(province);
        memory.map_zeroed(map, MAP_PROVINCE_LEN);
        memory.write_u64(MAP_ARRAY + index * 8, map);
        let record = record_of(province);
        memory.map_zeroed(record, D81_RECORD_SELECTOR_KEY_OFF + 0x10);
        memory.write_u64(map + D81_MAP_PROVINCE_RECORD_OFF as u64, record);
        memory.write_u32(
            record + D81_RECORD_SELECTOR_KEY_OFF as u64,
            selector_key_of(province),
        );
        for (axis, value) in anchor_of(province, false).iter().enumerate() {
            memory.write_u32(
                map + D81_ANCHOR_PRIMARY_OFF as u64 + axis as u64 * 4,
                *value,
            );
        }
        for (axis, value) in anchor_of(province, true).iter().enumerate() {
            memory.write_u32(
                map + D81_ANCHOR_SECONDARY_OFF as u64 + axis as u64 * 4,
                *value,
            );
        }
        if let Some(slot) = placement[province as usize] {
            let entry = SELECTOR_TABLE + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
            memory.write_u32(
                entry + D81_SELECTOR_KEY_OFF as u64,
                selector_key_of(province),
            );
            memory.write_u64(
                entry + D81_SELECTOR_VALUE_OFF as u64,
                selector_value_of(province),
            );
        }

        if !railed(province) {
            continue;
        }
        let info = rail_info_of(province);
        memory.map_zeroed(
            info,
            RAIL_INFO_PAIRS_PTR_OFF + RAIL_HEADER_WINDOW_LEN + 0x20,
        );
        let blob = pair_blob_of(province);
        memory.map_zeroed(blob, 0x40);
        memory.write_u32(blob, (province + 1) % PROVINCES);
        memory.write_u32(blob + 4, 1);
        memory.write_u32(blob + 8, (province + 2) % PROVINCES);
        memory.write_u32(blob + 12, 2);
        memory.write_u64(info + RAIL_INFO_PAIRS_PTR_OFF as u64, blob);
        memory.write_u32(info + RAIL_INFO_PAIR_COUNT_OFF as u64, 2);
        memory.write_u32(
            info + RAIL_INFO_PAIRS_PTR_OFF as u64 + 8,
            0xDEAD_0000 + province,
        );

        let pointer = if province == REJECTED_PROVINCE {
            0x1000u64
        } else {
            info
        };
        memory.write_u64(RAIL_TABLE + index * 8, pointer);
    }

    memory.set_readable(ACTIVE_TABLE + VOID_PAGE_START, false);
    memory
}

fn active_window_of(province: u32) -> [u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN] {
    let mut window = [0u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN];
    window[0] = (province & 0xFF) as u8;
    window[1] = ((province >> 8) & 0xFF) as u8;
    window[2] = 0x5A;
    window[3] = 0xC3;
    window[4] = (province & 0x0F) as u8;
    window[5] = 0x11;
    window[6] = 0x22;
    window
}

fn plan_roots() -> Domain81PlanRoots {
    Domain81PlanRoots {
        game_state: GAMESTATE,
        idler_slot: IDLER_SLOT,
        idler_slot_b: IDLER_SLOT_B,
        container: CONTAINER,
        province_slots: PROVINCES,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    NoRail,
    RailPointerRejected {
        raw: u64,
    },
    Unreadable {
        what: &'static str,
    },
    Row {
        neighbors: Vec<(u32, u32)>,
        active: [u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN],
        source_province: u64,
        selector_key: u32,
        selector_value: Option<u64>,
        primary: [u32; 3],
        secondary: [u32; 3],
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RowOutcome {
    province: u32,
    outcome: Outcome,
}

fn reference_walk(memory: &SyntheticMemory) -> Vec<RowOutcome> {
    let read = |addr: u64, len: usize| -> Option<Vec<u8>> {
        let mut out = vec![0u8; len];
        crate::snapshot_recipes::SnapshotMemory::copy_into(memory, addr, &mut out)
            .ok()
            .map(|()| out)
    };
    walk("live", &mut |addr, len| read(addr, len))
}

fn replay_walk(observation: &D81PlanObservation, hour: HourStamp) -> Vec<RowOutcome> {
    let view = observation
        .closed
        .snapshot
        .view_for(D81_CONSUMER, hour)
        .expect("the plan captured, so its consumer has a view");
    let vector = observation
        .vector
        .as_ref()
        .expect("the plan captured, so the vector pipeline ran");
    walk("copy", &mut |addr, len| {
        match vector.read(addr, len as u32, VectorImage::Opening) {
            VectorAnswer::Served(bytes) => return Some(bytes.to_vec()),
            VectorAnswer::NotCovered => {}
            VectorAnswer::Unreadable | VectorAnswer::SampleAbsent | VectorAnswer::Aliased => {
                return None
            }
            VectorAnswer::NeverObserved => {
                panic!("an opening read of a captured span must never be NeverObserved")
            }
        }
        let mut out = vec![0u8; len];
        match view.read_into(addr, &mut out) {
            Ok(()) => Some(out),
            Err(ViewRefusal::Unreadable { .. }) | Err(ViewRefusal::NotCaptured { .. }) => None,
            Err(other) => panic!("view refused {addr:#x}+{len}: {other:?}"),
        }
    })
}

fn walk(arm: &str, read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Vec<RowOutcome> {
    use crate::attach_log::*;
    let u32_at = |read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, addr: u64| {
        read(addr, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let u64_at = |read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, addr: u64| {
        read(addr, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    };

    let mut rows = Vec::new();
    for province in 0..PROVINCES {
        let index = u64::from(province);
        let slot = u64_at(read, RAIL_TABLE + index * 8).expect("rail slot");
        if slot == 0 {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::NoRail,
            });
            continue;
        }
        if !(HEAP_LO as u64..HEAP_HI as u64).contains(&slot) || slot % 8 != 0 {
            rows.push(RowOutcome {
                province,
                outcome: Outcome::RailPointerRejected { raw: slot },
            });
            continue;
        }
        let header = read(
            slot + RAIL_INFO_PAIRS_PTR_OFF as u64,
            RAIL_HEADER_WINDOW_LEN,
        )
        .unwrap_or_else(|| {
            panic!("{arm}: rail header window for province {province} at {slot:#x}")
        });
        let pair_ptr = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let pair_count = u32::from_le_bytes(
            header[RAIL_HEADER_COUNT_IN_WINDOW..RAIL_HEADER_COUNT_IN_WINDOW + 4]
                .try_into()
                .unwrap(),
        );
        let blob = read(pair_ptr, pair_count as usize * 8).expect("pair blob");
        let neighbors: Vec<(u32, u32)> = blob
            .chunks_exact(8)
            .map(|bytes| {
                (
                    u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                )
            })
            .collect();
        let active_addr =
            ACTIVE_TABLE + index * D81_ACTIVE_ENTRY_STRIDE as u64 + D81_ACTIVE_WINDOW_OFFSET as u64;
        let active = match read(active_addr, D81_ACTIVE_WINDOW_LEN) {
            Some(bytes) => {
                let mut window = [0u8; D81_ACTIVE_WINDOW_LEN];
                window.copy_from_slice(&bytes);
                window
            }
            None => {
                rows.push(RowOutcome {
                    province,
                    outcome: Outcome::Unreadable { what: "active" },
                });
                continue;
            }
        };
        let source_province = u64_at(read, SOURCE_ARRAY + index * 8).expect("gamestate slot");
        let map = u64_at(read, MAP_ARRAY + index * 8).expect("map slot");
        let record = u64_at(read, map + D81_MAP_PROVINCE_RECORD_OFF as u64).expect("record ptr");
        let selector_key =
            u32_at(read, record + D81_RECORD_SELECTOR_KEY_OFF as u64).expect("selector key");
        let selector_value = probe_selector(selector_key, read);
        let axes = |read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>, base: u64| {
            [
                u32_at(read, base).expect("axis 0"),
                u32_at(read, base + 4).expect("axis 1"),
                u32_at(read, base + 8).expect("axis 2"),
            ]
        };
        let primary = axes(read, map + D81_ANCHOR_PRIMARY_OFF as u64);
        let secondary = axes(read, map + D81_ANCHOR_SECONDARY_OFF as u64);
        rows.push(RowOutcome {
            province,
            outcome: Outcome::Row {
                neighbors,
                active,
                source_province,
                selector_key,
                selector_value,
                primary,
                secondary,
            },
        });
    }
    rows
}

fn probe_selector(key: u32, read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>) -> Option<u64> {
    use crate::attach_log::*;
    let mut entry = SELECTOR_TABLE
        + u64::from(selector_slot_of(key, SELECTOR_MASK)) * D81_SELECTOR_ENTRY_STRIDE as u64;
    let mut distance = 1u16;
    loop {
        let marker = read(entry + D81_SELECTOR_MARKER_OFF as u64, 1)?[0];
        if marker == 0
            || distance > u16::from(marker)
            || distance > u16::from(SELECTOR_MAX_DISTANCE).saturating_add(1)
        {
            return None;
        }
        let held = u32::from_le_bytes(
            read(entry + D81_SELECTOR_KEY_OFF as u64, 4)?
                .try_into()
                .ok()?,
        );
        if held == key {
            return Some(u64::from_le_bytes(
                read(entry + D81_SELECTOR_VALUE_OFF as u64, 8)?
                    .try_into()
                    .ok()?,
            ));
        }
        distance += 1;
        entry += D81_SELECTOR_ENTRY_STRIDE as u64;
    }
}

#[test]
fn the_shipped_d81_plan_produces_the_same_rows_as_a_direct_read_walk() {
    let memory = stage_world();
    let expected = reference_walk(&memory);

    let recipe = domain81_copy_plan(&plan_roots());
    validate(&recipe).expect("the shipped plan must be well formed");

    let hour = HourStamp(19390901);
    let counting = CountingSnapshotMemory::new(&memory);
    let mut counters = D81PlanCounters::default();
    let observation =
        d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters).expect("custody");
    assert!(
        !counters.refused && !counters.consumer_gapped,
        "the shipped plan must capture, not refuse: {counters:?}",
    );
    assert!(
        counters.scoped_voids >= 1,
        "the fixture stages an unreadable page inside a speculative copy",
    );
    assert_eq!(counters.pointers_rejected, 1);

    let selector = Span::new(
        SELECTOR_TABLE,
        u64::from(SELECTOR_SLOTS) * crate::attach_log::D81_SELECTOR_ENTRY_STRIDE as u64,
    );
    assert!(
        observation
            .closed
            .snapshot
            .intervals()
            .iter()
            .any(|record| record.span().contains_span(&selector)),
        "the selector extent must span mask + max_distance + 2 slots",
    );

    let produced = replay_walk(&observation, hour);

    assert_eq!(produced.len(), expected.len());
    for (left, right) in produced.iter().zip(expected.iter()) {
        assert_eq!(
            left, right,
            "province {} differs between the shipped plan and the direct-read walk",
            left.province,
        );
    }

    let count = |predicate: fn(&Outcome) -> bool| {
        expected
            .iter()
            .filter(|row| predicate(&row.outcome))
            .count()
    };
    assert!(
        count(|outcome| matches!(outcome, Outcome::NoRail)) > 0,
        "the walk must include railless provinces",
    );
    assert_eq!(
        count(|outcome| matches!(outcome, Outcome::RailPointerRejected { .. })),
        1,
    );
    let unreadable = count(|outcome| matches!(outcome, Outcome::Unreadable { .. }));
    assert!(
        unreadable > 0,
        "and rows whose consumed bytes are in a void"
    );
    assert!(
        count(|outcome| matches!(outcome, Outcome::Row { .. })) > unreadable,
        "and the over-read must not have cost the rows that never touch it",
    );
    let voided_rows: Vec<u32> = expected
        .iter()
        .filter(|row| matches!(row.outcome, Outcome::Unreadable { .. }))
        .map(|row| row.province)
        .collect();
    assert!(
        voided_rows.iter().all(|province| voided(*province)),
        "only provinces whose active entry is in the voided page may refuse",
    );
}

#[test]
fn the_shipped_plans_province_tables_are_a_fixed_number_of_transactions() {
    let memory = stage_world();
    let hour = HourStamp(19390901);
    memory.reset_counters();
    let counting = CountingSnapshotMemory::new(&memory);
    let mut counters = D81PlanCounters::default();
    let observation =
        d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters).expect("custody");
    let vector = observation
        .vector
        .as_ref()
        .expect("the plan captured, so the vector pipeline ran");

    let railed_admitted: Vec<u32> = (0..PROVINCES)
        .filter(|p| railed(*p) && *p != REJECTED_PROVINCE)
        .collect();
    let expected_scattered = railed_admitted.len();
    let mut cprovinces: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut source_slots: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for &province in &railed_admitted {
        cprovinces.insert(source_province_of(province));
        source_slots.insert(province);
        let key = selector_key_of(province);
        if key < PROVINCES {
            cprovinces.insert(source_province_of(key));
            source_slots.insert(key);
        }
    }

    for (family, expected) in [
        (D81Family::RailHeader, expected_scattered),
        (D81Family::PairBlob, expected_scattered),
        (D81Family::MapRecordPtr, expected_scattered),
        (D81Family::MapAnchors, expected_scattered),
        (D81Family::RecordKey, expected_scattered),
        (D81Family::SourceProvince, cprovinces.len()),
        (D81Family::SourceSlotClose, source_slots.len()),
        (D81Family::MapSlotClose, expected_scattered),
        (D81Family::ActiveClose, expected_scattered),
    ] {
        assert_eq!(
            vector.family_window_count(family),
            expected,
            "family {family:?} must hold exactly the windows the walk reads",
        );
    }
    assert_eq!(
        vector.closing_window_counts(),
        (expected_scattered, expected_scattered),
        "the header and record-pointer closing sides cover their opening windows",
    );

    let window_images = 4 * expected_scattered
        + 2 * expected_scattered
        + 4 * expected_scattered
        + 2 * expected_scattered
        + 2 * expected_scattered
        + 2 * cprovinces.len()
        + 2 * source_slots.len()
        + 2 * expected_scattered
        + 3 * expected_scattered;
    let bulk_intervals = observation.closed.snapshot.interval_count();
    let copies = counting.calls() as usize;
    let ceiling = window_images + 3 * bulk_intervals + 64;
    assert!(
        copies <= ceiling,
        "the plan issued {copies} copies against a demand ceiling of {ceiling} \
         ({window_images} demanded window images, {bulk_intervals} bulk intervals): \
         a family has grown images nobody consumes",
    );
    let voided_railed = railed_admitted
        .iter()
        .filter(|province| voided(**province))
        .count();
    assert!(
        copies >= window_images - 2 * voided_railed,
        "sanity: the counter is live and every demanded image was attempted \
         ({copies} against {window_images} demanded minus the voided active \
         entries' {voided_railed} curtailed windows)",
    );

    let spans_whole = |base: u64, bytes: u64| {
        let want = Span::new(base, bytes);
        observation
            .closed
            .snapshot
            .intervals()
            .iter()
            .any(|record| record.span().contains_span(&want))
    };
    assert!(spans_whole(RAIL_TABLE, PROVINCES as u64 * 8));
    assert!(spans_whole(SOURCE_ARRAY, PROVINCES as u64 * 8));
    assert!(spans_whole(MAP_ARRAY, PROVINCES as u64 * 8));
    assert!(
        !spans_whole(
            ACTIVE_TABLE,
            PROVINCES as u64 * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
        ),
        "the voided page is genuinely absent from the active table's coverage",
    );
}

#[test]
fn the_shipped_plan_closes_its_rail_headers_before_the_observation_ends() {
    use crate::snapshot_recipes::synthetic::MutatingMemory;
    use crate::snapshot_recipes::SnapshotMemory as _;

    let hour = HourStamp(19390901);
    struct Recorder<'a> {
        inner: &'a SyntheticMemory,
        log: std::cell::RefCell<Vec<u64>>,
    }
    impl crate::snapshot_recipes::SnapshotMemory for Recorder<'_> {
        fn copy_into(
            &self,
            addr: u64,
            out: &mut [u8],
        ) -> Result<(), crate::snapshot_recipes::MemoryRefusal> {
            self.log.borrow_mut().push(addr);
            self.inner.copy_into(addr, out)
        }
    }
    let plain = stage_world();
    let recorder = Recorder {
        inner: &plain,
        log: std::cell::RefCell::new(Vec::new()),
    };
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut counters = D81PlanCounters::default();
    d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters).expect("custody");
    let blob_lo = PAIR_BLOB;
    let blob_hi = PAIR_BLOB + u64::from(PROVINCES) * SCATTER_STRIDE;
    let first_blob_ordinal = recorder
        .log
        .borrow()
        .iter()
        .position(|addr| (blob_lo..blob_hi).contains(addr))
        .expect("the pipeline copies at least one pair blob") as u64
        + 1;

    {
        let log = recorder.log.borrow();
        let record_ptr_addr =
            map_province_of(0) + crate::attach_log::D81_MAP_PROVINCE_RECORD_OFF as u64;
        let key_lo = RECORD;
        let key_hi = RECORD + u64::from(PROVINCES) * SCATTER_STRIDE;
        let record_close_ordinal = log
            .iter()
            .enumerate()
            .filter(|(_, addr)| **addr == record_ptr_addr)
            .map(|(ordinal, _)| ordinal)
            .nth(2)
            .expect("the record pointer takes four observations");
        let last_key_ordinal = log
            .iter()
            .enumerate()
            .filter(|(_, addr)| (key_lo..key_hi).contains(*addr))
            .map(|(ordinal, _)| ordinal)
            .last()
            .expect("the pipeline copies at least one record key");
        assert!(
            last_key_ordinal < record_close_ordinal,
            "the record-pointer closing pair (ordinal {record_close_ordinal}) \
             must be observed after every key copy (last at {last_key_ordinal})",
        );
    }

    let victim = rail_info_of(0) + crate::attach_log::RAIL_INFO_PAIRS_PTR_OFF as u64;
    let count_field = victim + crate::attach_log::RAIL_HEADER_COUNT_IN_WINDOW as u64;
    let dirty = MutatingMemory::new(
        stage_world(),
        first_blob_ordinal,
        vec![(count_field, 7u32.to_le_bytes().to_vec())],
    );
    let counting = CountingSnapshotMemory::new(&dirty);
    let mut counters = D81PlanCounters::default();
    let observation =
        d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters).expect("custody");
    assert!(dirty.mutation_applied(), "the fixture must actually write");
    let vector = observation.vector.as_ref().expect("the pipeline ran");

    let window_len = crate::attach_log::RAIL_HEADER_WINDOW_LEN as u32;
    let opening = match vector.read(victim, window_len, VectorImage::Opening) {
        VectorAnswer::Served(bytes) => bytes.to_vec(),
        other => panic!("the opening header image must serve: {other:?}"),
    };
    let closing = match vector.read(victim, window_len, VectorImage::Closing) {
        VectorAnswer::Served(bytes) => bytes.to_vec(),
        other => panic!("the closing header image must serve: {other:?}"),
    };
    assert_eq!(
        opening, closing,
        "the header's closing pair was observed before the blob wave, so a \
         write triggered at the first blob copy must not reach it",
    );
    let mut live = [0u8; 4];
    dirty
        .copy_into(count_field, &mut live)
        .expect("the count field is readable");
    assert_eq!(
        u32::from_le_bytes(live),
        7,
        "the write must be visible live"
    );
}
