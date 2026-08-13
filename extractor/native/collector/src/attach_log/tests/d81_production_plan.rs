//! THE PRODUCTION GATE: does the SHIPPED d81 copy plan, at the live reader's
//! own offsets, reach every byte the reader decodes and produce the same rows?
//!
//! # How this differs from the S4 expressibility gate
//!
//! [`super::d81_recipe_expressibility`] answered a question about the
//! SUBSTRATE: can the vocabulary express d81's observation phase at all. Its
//! world is built from lane FX's staged shape constants and its program is
//! written inside the test.
//!
//! This file answers a question about the PLAN THAT SHIPS. Its world is laid
//! out at `read_compact_rail_replay_state`'s own offsets — every one of them
//! imported from production, never restated — and the recipe under test is
//! `d81_copy_bracket::domain81_copy_plan`, the same function a converted reader
//! would call. So a production offset that drifts away from the plan fails
//! here, which is the drift lane FZ's round 2 caught by hand and single-sourced
//! against.
//!
//! # Why the population is 512 and not 13,413
//!
//! Because nothing under test depends on it. The shapes, strides and formulas
//! are production's; the count is a fixture's, chosen so that one page of the
//! active table is a page — ruling A's scoping is a page-level statement and a
//! table smaller than three pages cannot make it.
//!
//! # What this file does NOT claim
//!
//! It is not an equivalence proof for the converted READER. d81 has no
//! end-to-end fixture (see the head of [`super::d81_frame_cost`]), so the live
//! reader cannot be driven offline past its `RailRoots` stage and no offline
//! instrument can compare the two. That comparison is a capture, and the
//! conversion switch is off until one has been taken.

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

// ---------------------------------------------------------------------------
// The staged world, at PRODUCTION offsets
// ---------------------------------------------------------------------------

/// Provinces in the staged world. See the module header for why the count is
/// the one thing that is not production's.
const PROVINCES: u32 = 512;
/// Every third province is railed, close to t26's 3,510 of 13,413.
const RAIL_EVERY: u32 = 3;
/// The page granularity a scoped copy's failure narrows to.
const PAGE: u64 = crate::snapshot_recipes::SCOPED_VOID_GRANULARITY_BYTES;

/// Everything is laid out inside `[HEAP_LO, HEAP_HI)`, because that IS the
/// predicate the plan admits pointers on — the live reader's
/// `is_map_control_heap_pointer`. A fixture outside it would have every
/// dereference rejected and would prove the opposite of what it claims.
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

/// The scattered per-province families. Each gets its own 0x1000_0000 of space
/// and uses 512 * 0x0002_0000 of it, so no two can overlap however the province
/// count is raised.
const RAIL_INFO: u64 = BASE + 0x1000_0000;
const PAIR_BLOB: u64 = BASE + 0x2000_0000;
const MAP_PROVINCE: u64 = BASE + 0x3000_0000;
const RECORD: u64 = BASE + 0x4000_0000;
const SOURCE_PROVINCE: u64 = BASE + 0x5000_0000;
const SCATTER_STRIDE: u64 = 0x0002_0000;

/// The selector map's mask and probe depth. Production reads both off the
/// engine; the fixture picks them and the DERIVED extent formula
/// (`mask + max_distance + 2`) is the thing under test.
const SELECTOR_MASK: u32 = PROVINCES - 1;
const SELECTOR_MAX_DISTANCE: u8 = 2;
const SELECTOR_SLOTS: u32 = SELECTOR_MASK + 1 + SELECTOR_MAX_DISTANCE as u32 + 1;

/// Which province the fixture gives a rail pointer the reader must reject.
const REJECTED_PROVINCE: u32 = 9;
/// The page of the active table the fixture makes unreadable.
const VOID_PAGE_START: u64 = PAGE;

/// The map province's own width: through the end of the SECONDARY anchor
/// triplet at `+0x14F0`, which is the last field the reader consumes and lies
/// well past the record pointer at `+0x11B8`. Over 5 KiB for the 32 bytes the
/// reader takes out of it, which is why the plan copies three windows and not
/// the object.
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

/// True when this province's active-table entry lands in the voided page.
fn voided(province: u32) -> bool {
    let at = u64::from(province) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64;
    (VOID_PAGE_START..VOID_PAGE_START + PAGE).contains(&at)
}

/// The engine's own selector hash, transcribed from `select_secondary`. It is
/// here so the fixture can PLACE a key where the probe will look for it; the
/// plan copies the whole table and never evaluates it.
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

/// Where each province's key lands, or `None` when the probe's own reach
/// cannot place it. Linear, forward, no wrap — `select_secondary`'s own rule.
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
    // THREE regions, of which the middle one is a single page and is
    // unreadable. Ruling A: a fixture whose table could only refuse WHOLE
    // would make "gap only what is consumed" a statement about a set that is
    // always everything.
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
    memory.write_u64(SELECTOR_MAP + D81_SELECTOR_MAP_TABLE_OFF as u64, SELECTOR_TABLE);
    memory.write_u32(SELECTOR_MAP + D81_SELECTOR_MAP_MASK_OFF as u64, SELECTOR_MASK);
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

    // Every selector slot is OCCUPIED, with a marker that admits the full
    // probe distance and a key no province owns, so a probe walks its whole
    // distance instead of stopping at the first empty slot. That is the
    // engine's own shape and it is what reaches the tail the derived extent
    // exists to cover.
    for slot in 0..SELECTOR_SLOTS + 8 {
        let entry = SELECTOR_TABLE + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
        memory.write(
            entry + D81_SELECTOR_MARKER_OFF as u64,
            &[SELECTOR_MAX_DISTANCE + 1],
        );
        memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, u32::MAX);
    }

    // WHERE EACH KEY LANDS, decided before anything is written.
    //
    // The engine's table is a real open-addressed one and two keys DO collide:
    // a key whose bucket is taken moves forward, and a key that cannot be
    // placed within the probe's own reach is simply not in the table. Writing
    // every key at its unprobed bucket would silently overwrite the collisions
    // and stage a table the engine's own probe could never have produced —
    // which would then make every row's selector value agree for the wrong
    // reason.
    let placement = selector_placement();

    for province in 0..PROVINCES {
        let index = u64::from(province);
        // The active entry, the gamestate slot and the map slot exist for
        // every province, railed or not: a copy plan takes the whole table.
        memory.write(
            ACTIVE_TABLE
                + index * D81_ACTIVE_ENTRY_STRIDE as u64
                + D81_ACTIVE_WINDOW_OFFSET as u64,
            &active_window_of(province),
        );
        let source = source_province_of(province);
        memory.map_zeroed(source, crate::attach_log::CPROVINCE_OFF_ID + 0x20);
        memory.write_u32(source + crate::attach_log::CPROVINCE_OFF_ID as u64, province);
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
            memory.write_u32(map + D81_ANCHOR_PRIMARY_OFF as u64 + axis as u64 * 4, *value);
        }
        for (axis, value) in anchor_of(province, true).iter().enumerate() {
            memory.write_u32(
                map + D81_ANCHOR_SECONDARY_OFF as u64 + axis as u64 * 4,
                *value,
            );
        }
        if let Some(slot) = placement[province as usize] {
            let entry = SELECTOR_TABLE + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
            memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, selector_key_of(province));
            memory.write_u64(
                entry + D81_SELECTOR_VALUE_OFF as u64,
                selector_value_of(province),
            );
        }

        if !railed(province) {
            continue;
        }
        let info = rail_info_of(province);
        memory.map_zeroed(info, RAIL_INFO_PAIRS_PTR_OFF + RAIL_HEADER_WINDOW_LEN + 0x20);
        let blob = pair_blob_of(province);
        memory.map_zeroed(blob, 0x40);
        // Two neighbours, `(neighbor_id, level)` pairs of four bytes each.
        memory.write_u32(blob, (province + 1) % PROVINCES);
        memory.write_u32(blob + 4, 1);
        memory.write_u32(blob + 8, (province + 2) % PROVINCES);
        memory.write_u32(blob + 12, 2);
        memory.write_u64(info + RAIL_INFO_PAIRS_PTR_OFF as u64, blob);
        memory.write_u32(info + RAIL_INFO_PAIR_COUNT_OFF as u64, 2);
        // The four bytes between the two consumed fields carry a value, so a
        // whole-interval comparison would have something to see.
        memory.write_u32(info + RAIL_INFO_PAIRS_PTR_OFF as u64 + 8, 0xDEAD_0000 + province);

        let pointer = if province == REJECTED_PROVINCE {
            // Readable and 8-aligned, and OUTSIDE the heap window: the case
            // that matters is a value the substrate would happily follow and
            // the reader rejects.
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

// ---------------------------------------------------------------------------
// The outcome the two walkers must agree on
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    /// The rail slot was null: absent is not broken.
    NoRail,
    /// The rail pointer failed `is_map_control_heap_pointer` and was not
    /// followed.
    RailPointerRejected { raw: u64 },
    /// A byte this row consumes could not be read.
    Unreadable { what: &'static str },
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

/// d81's walk over its own bytes: the "live reader" arm, reading whatever it
/// likes wherever it likes.
fn reference_walk(memory: &SyntheticMemory) -> Vec<RowOutcome> {
    let read = |addr: u64, len: usize| -> Option<Vec<u8>> {
        let mut out = vec![0u8; len];
        crate::snapshot_recipes::SnapshotMemory::copy_into(memory, addr, &mut out)
            .ok()
            .map(|()| out)
    };
    walk("live", &mut |addr, len| read(addr, len))
}

/// The same walk over the plan's COPY — the vector families first, the bulk
/// view for everything they do not cover — which can see NOTHING the plan did
/// not copy. A plan that under-copies does not produce a wrong number here —
/// it produces a refusal, and the row differs.
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

/// One walk, over whatever byte source it is handed. Both arms run THIS, so a
/// difference between them is a difference in what the bytes said and never a
/// difference in what was asked.
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
        let header = read(slot + RAIL_INFO_PAIRS_PTR_OFF as u64, RAIL_HEADER_WINDOW_LEN)
            .unwrap_or_else(|| panic!("{arm}: rail header window for province {province} at {slot:#x}"));
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
        let active_addr = ACTIVE_TABLE
            + index * D81_ACTIVE_ENTRY_STRIDE as u64
            + D81_ACTIVE_WINDOW_OFFSET as u64;
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
        // POST-BRACKET compute over the copied table: the probe costs no
        // guarded calls at all once the extent is in hand.
        // A MISS IS AN OUTCOME. The engine's table holds what the engine's own
        // probe can reach, and a key it cannot is absent rather than broken.
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

/// `select_secondary`'s bounded linear probe, over whatever byte source it is
/// handed.
fn probe_selector(
    key: u32,
    read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>,
) -> Option<u64> {
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

// ---------------------------------------------------------------------------
// THE GATE
// ---------------------------------------------------------------------------

/// The SHIPPED plan, at the live reader's own offsets, over a staged world,
/// producing the same rows as a direct-read walk of the same world — refusal
/// paths included.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: any drift between a `D81_*` offset
/// the reader forms addresses from and the plan that copies for it; the derived
/// selector count losing its probe tail; the admissibility window going away
/// (the out-of-heap rail pointer is followed, its copy fails, and the recipe
/// refuses); `Tolerance::Scoped` going away (the unreadable page refuses the
/// whole recipe and no province publishes); or the map-province window list
/// losing one of its three windows.
#[test]
fn the_shipped_d81_plan_produces_the_same_rows_as_a_direct_read_walk() {
    let memory = stage_world();
    let expected = reference_walk(&memory);

    let recipe = domain81_copy_plan(&plan_roots());
    validate(&recipe).expect("the shipped plan must be well formed");

    let hour = HourStamp(19390901);
    let counting = CountingSnapshotMemory::new(&memory);
    let mut counters = D81PlanCounters::default();
    let observation = d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters)
        .expect("custody");
    assert!(
        !counters.refused && !counters.consumer_gapped,
        "the shipped plan must capture, not refuse: {counters:?}",
    );
    // RULING A: the unreadable page is a named void and the frame still
    // publishes.
    assert!(
        counters.scoped_voids >= 1,
        "the fixture stages an unreadable page inside a speculative copy",
    );
    // M3: exactly one rail pointer was rejected, on the reader's own
    // predicate — by the VECTOR pipeline now, and counted on the same wire
    // counter the recipe's rejections reach.
    assert_eq!(counters.pointers_rejected, 1);

    // M1: the selector extent spans `mask + max_distance + 2` slots, which is
    // the tail a copy sized at the masked table alone would miss.
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

    // And the fixture must contain every outcome the gate claims to cover, or
    // the loop above proves nothing about the refusal paths.
    let count = |predicate: fn(&Outcome) -> bool| {
        expected.iter().filter(|row| predicate(&row.outcome)).count()
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
    assert!(unreadable > 0, "and rows whose consumed bytes are in a void");
    assert!(
        count(|outcome| matches!(outcome, Outcome::Row { .. })) > unreadable,
        "and the over-read must not have cost the rows that never touch it",
    );
    // The voided rows are exactly the ones the fixture put in the page, and no
    // others: ruling A refuses what it must and nothing beyond it.
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

/// THE EXECUTABLE DEMAND TABLE (lane HS): the vector families' window counts,
/// pinned against the fixture's own arithmetic, and the whole-table claim the
/// conversion has always rested on.
///
/// The old transaction model — `copies == 4 * intervals + 4`, four images for
/// every interval whether the walk consumed them or not — is retired with the
/// per-province recipe branch. What replaces it is DEMAND: each family holds
/// exactly the windows the walk reads, at exactly the image count the walk
/// consumes, and this test pins the window counts family by family so a
/// family silently dropped (or silently widened to every province) fails
/// here by name rather than inside a total.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: a whole-table root becoming a
/// per-element copy (the interval count moves); a vector family dropped or
/// re-keyed off the arrays instead of the rail branch (its window count moves
/// to 0 or to the province count); the demand-driven image counts growing
/// back toward uniform four (the copies ceiling moves).
#[test]
fn the_shipped_plans_province_tables_are_a_fixed_number_of_transactions() {
    let memory = stage_world();
    let hour = HourStamp(19390901);
    memory.reset_counters();
    let counting = CountingSnapshotMemory::new(&memory);
    let mut counters = D81PlanCounters::default();
    let observation = d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters)
        .expect("custody");
    let vector = observation
        .vector
        .as_ref()
        .expect("the plan captured, so the vector pipeline ran");

    // The fixture's own arithmetic, computed from the same helpers that
    // staged the world.
    let railed_admitted: Vec<u32> = (0..PROVINCES)
        .filter(|p| railed(*p) && *p != REJECTED_PROVINCE)
        .collect();
    let expected_scattered = railed_admitted.len();
    // The CProvince family covers the union of the railed provinces' own
    // objects and the objects behind their selector keys, deduplicated by
    // address.
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

    // The DEMANDED image counts, as a ceiling on the whole observation: every
    // window at its family's image count, plus the bulk recipe's intervals at
    // an opening pair (the active table's third sample and the voided page's
    // bisect retries are the slack term). A plan that grew back toward four
    // uniform images per window moves ~40% past this and fails.
    let window_images = 4 * expected_scattered // rail headers
        + 2 * expected_scattered               // pair blobs
        + 4 * expected_scattered               // record pointers
        + 2 * expected_scattered               // anchors
        + 2 * expected_scattered               // record keys
        + 2 * cprovinces.len()                 // CProvince heads
        + 2 * source_slots.len()               // source slot closes
        + 2 * expected_scattered               // map slot closes
        + 3 * expected_scattered; // active closes
    let bulk_intervals = observation.closed.snapshot.interval_count();
    let copies = counting.calls() as usize;
    let ceiling = window_images + 3 * bulk_intervals + 64;
    assert!(
        copies <= ceiling,
        "the plan issued {copies} copies against a demand ceiling of {ceiling} \
         ({window_images} demanded window images, {bulk_intervals} bulk intervals): \
         a family has grown images nobody consumes",
    );
    // A window whose FIRST observation faults ends its side — the later
    // images are deliberately never attempted — and the fixture stages
    // exactly one such population: the railed provinces whose active entry
    // sits in the voided page, each of which attempts one closing
    // observation instead of three.
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
    // The active table is the one with a hole in it, by construction.
    assert!(
        !spans_whole(
            ACTIVE_TABLE,
            PROVINCES as u64 * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
        ),
        "the voided page is genuinely absent from the active table's coverage",
    );
}

/// MID-WALK CLOSURE, on the shipped pipeline: the rail header's closing pair
/// is observed BEFORE the pair blobs are copied — the executor's
/// `AfterWave(3)` instant, reproduced by `d81_vector_observe`'s stage order —
/// so a write to a header after its close is not a torn read of it.
///
/// The proof is the closing image's CONTENT: a write triggered at the FIRST
/// pair-blob copy lands after the header closing pair, so the header's
/// closing image still equals its opening image and the walk's closure check
/// publishes. A pipeline that deferred the header close to its end would take
/// the closing image after the write, the two images would differ, and this
/// test fails.
///
/// kills: moving `d81_vector_observe`'s header-closing stage after the
/// pair-blob stage (or folding it into the final closing stages).
#[test]
fn the_shipped_plan_closes_its_rail_headers_before_the_observation_ends() {
    use crate::snapshot_recipes::synthetic::MutatingMemory;
    use crate::snapshot_recipes::SnapshotMemory as _;

    let hour = HourStamp(19390901);
    // Where the first pair-blob copy is, measured rather than guessed: run
    // the plan once against a recording wrapper and find the ordinal of the
    // first copy whose address is a staged blob's.
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

    // AND THE RECORD POINTER'S CLOSING PAIR IS OBSERVED AFTER THE KEY COPIES
    // (Codex review round 1, finding 5): the check's purpose is that the key
    // the walk consumed came from the record the closing observation vouches
    // for, so a pipeline that closed the record pointer before copying the
    // keys would leave a record replacement during the key copy invisible.
    // The record pointer's four copies are O, S1, C, CS1 in log order, so its
    // THIRD occurrence is the first closing observation.
    // kills: moving `d81_vector_observe`'s stage 9 (record-pointer closing)
    // ahead of stage 8 (record keys).
    {
        let log = recorder.log.borrow();
        let record_ptr_addr = map_province_of(0) + crate::attach_log::D81_MAP_PROVINCE_RECORD_OFF as u64;
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

    // The write: the pair COUNT of province 0's header, a consumed field,
    // triggered at the first blob copy — after the header closing pair if and
    // only if the pipeline closes headers where it says it does.
    let victim = rail_info_of(0) + crate::attach_log::RAIL_INFO_PAIRS_PTR_OFF as u64;
    let count_field = victim + crate::attach_log::RAIL_HEADER_COUNT_IN_WINDOW as u64;
    let dirty = MutatingMemory::new(
        stage_world(),
        first_blob_ordinal,
        vec![(count_field, 7u32.to_le_bytes().to_vec())],
    );
    let counting = CountingSnapshotMemory::new(&dirty);
    let mut counters = D81PlanCounters::default();
    let observation = d81_observe_copy_plan_with(&counting, &plan_roots(), hour, &mut counters)
        .expect("custody");
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
    // And the write DID land inside the observation: the live memory holds
    // the new count, so the equality above is a statement about WHEN the
    // closing image was taken and not about the write being lost.
    let mut live = [0u8; 4];
    dirty
        .copy_into(count_field, &mut live)
        .expect("the count field is readable");
    assert_eq!(u32::from_le_bytes(live), 7, "the write must be visible live");
}
