//! THE CONVERSION GATE: does the SHIPPED walk publish the same rows when it
//! reads the plan's copy as when it reads the memory directly?
//!
//! # What was missing, and what closed it
//!
//! Lane GC shipped `domain81_copy_plan` with its switch OFF and named the
//! reason: no offline instrument could compare a converted reader against the
//! live one, because `read_compact_rail_replay_state` could only be driven
//! against a live game. Two things closed that. t46 ran the plan beside the live
//! walk on a real game — 7,024 rows compared, 0 divergent, every refusal-class
//! counter zero — and the walk itself became [`d81_walk_provinces`], one body
//! parameterised over [`D81ReadSource`].
//!
//! So this file runs THE SHIPPED WALK twice over one staged world: once through
//! [`D81ReadSource::Staged`], which reads the world directly with no plan
//! between, and once through a [`D81ReadSource::Snapshot`] over the copy
//! `domain81_copy_plan` took of that same world. Both arms are the same
//! function, so nothing here can pass by the two arms having drifted into
//! agreement about a walk neither of them is.
//!
//! # What each arm is allowed to differ in, and it is nothing
//!
//! Rows, row order, neighbour totals, and the refusal — stage, reason, subject,
//! index and raw witness alike. A conversion that published the same rows and a
//! different refusal would be a conversion that loses the vocabulary a capture
//! diagnoses d81 with, and half of d81's frames are refusals.
//!
//! # Why this world and not `d81_production_plan`'s
//!
//! That fixture is built to exercise the PLAN — a rejected rail pointer and an
//! unreadable active page, both of which refuse the whole frame at the first
//! province that touches them. A walk that refuses at province 9 proves the two
//! arms agree about province 9 and nothing about the other 502. This world is
//! built for the WALK: every province railed into a reciprocal triple, so a
//! clean arm publishes every row, and the degradations are added one at a time
//! by an arm that asks for them.

use super::*;

use crate::attach_log::d81_copy_bracket::{
    domain81_copy_plan, CountingSnapshotMemory, Domain81PlanRoots, D81ReadSource, D81_CONSUMER,
    D81_CLOSING_IMAGES_HELD,
};
use crate::snapshot_recipes::synthetic::SyntheticMemory;
use crate::snapshot_recipes::HourStamp;

// ---------------------------------------------------------------------------
// The staged world
// ---------------------------------------------------------------------------

/// Province SLOTS the engine tables hold. Id 0 is staged and never walked: the
/// engine's own neighbour field is a signed int the walk refuses at zero
/// (`neighbor_raw <= 0`), so a world whose ids started at 0 would refuse every
/// triple containing it and prove agreement about a refusal instead of about
/// rows.
const PROVINCE_SLOTS: u32 = 385;
/// Provinces the topology names: 1..=384. A multiple of three, because the
/// neighbour rings are triples.
const PROVINCES: u32 = PROVINCE_SLOTS - 1;

/// Everything is laid out inside `[HEAP_LO, HEAP_HI)`, because that IS the
/// predicate the walk admits pointers on.
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

/// The `CProvince` vtable every gamestate province carries, and the value the
/// walk is told to expect. Non-zero on purpose: zero is what an unwritten field
/// holds, and a gate whose expected vtable is the fixture's default would pass
/// on the check never having run.
const CPROVINCE_VTABLE: u64 = BASE + 0x0F00_0000;

const SELECTOR_MASK: u32 = 511;
const SELECTOR_MAX_DISTANCE: u8 = 2;
const SELECTOR_SLOTS: u32 = SELECTOR_MASK + 1 + SELECTOR_MAX_DISTANCE as u32 + 1;

/// The page granularity a scoped copy's failure narrows to.
const PAGE: u64 = crate::snapshot_recipes::SCOPED_VOID_GRANULARITY_BYTES;

/// The one triple with no rail slot at all. A WHOLE triple, because the
/// neighbour lists are reciprocal within a triple and a single railless province
/// would leave its two partners naming a row nobody published — which the
/// deferred fold refuses, and this world has to be one the fold publishes.
const RAILLESS_TRIPLE: [u32; 3] = [382, 383, 384];

/// Rows a clean walk of this world publishes: every province the topology names
/// except the railless triple.
const PUBLISHED_ROWS: usize = PROVINCES as usize - RAILLESS_TRIPLE.len();

/// WHERE THE WORLD SITS, so one staging body can build it at more than one
/// SIZE.
///
/// The fixture above is 385 province slots, which is where every gate in this
/// file runs it. Lane GL needed the SAME world at production's 3,512 slots to
/// price the copy plan on the shape a capture actually observes, and a second
/// staging body would be a second world to keep in step. So the addresses and
/// the two sizes become a value, [`Layout::DEFAULT`] reproduces the constants
/// above exactly, and every test in this file keeps calling the free helpers
/// below, which are that default.
///
/// NOTHING ABOUT THE SHAPE CHANGES WITH SIZE — the same regions in the same
/// order, the same scatter stride, the same reciprocal triples, the same one
/// railless triple at the top. Only the counts and the spacing between the
/// scattered families move, and the spacing moves only so the families stay
/// disjoint at a size that would otherwise overrun them.
#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    pub(super) province_slots: u32,
    pub(super) selector_mask: u32,
    gamestate: u64,
    idler_slot: u64,
    idler_slot_b: u64,
    idler: u64,
    owner: u64,
    container: u64,
    rail_manager: u64,
    rail_table: u64,
    active_root: u64,
    active_table: u64,
    selector_map: u64,
    selector_table: u64,
    source_array: u64,
    map_array: u64,
    rail_info: u64,
    pair_blob: u64,
    map_province: u64,
    record: u64,
    source_province: u64,
    scatter_stride: u64,
}

impl Layout {
    /// The world every gate in this file stages: the constants above, spelled
    /// through the struct so the two cannot drift.
    pub(super) const DEFAULT: Layout = Layout {
        province_slots: PROVINCE_SLOTS,
        selector_mask: SELECTOR_MASK,
        gamestate: GAMESTATE,
        idler_slot: IDLER_SLOT,
        idler_slot_b: IDLER_SLOT_B,
        idler: IDLER,
        owner: OWNER,
        container: CONTAINER,
        rail_manager: RAIL_MANAGER,
        rail_table: RAIL_TABLE,
        active_root: ACTIVE_ROOT,
        active_table: ACTIVE_TABLE,
        selector_map: SELECTOR_MAP,
        selector_table: SELECTOR_TABLE,
        source_array: SOURCE_ARRAY,
        map_array: MAP_ARRAY,
        rail_info: RAIL_INFO,
        pair_blob: PAIR_BLOB,
        map_province: MAP_PROVINCE,
        record: RECORD,
        source_province: SOURCE_PROVINCE,
        scatter_stride: SCATTER_STRIDE,
    };

    /// The same world at `province_slots` slots.
    ///
    /// The five SCATTERED families each need `slots * SCATTER_STRIDE` of
    /// address space and the default layout gives them 0x1000_0000 apiece,
    /// which 3,512 slots overrun. A sized world therefore spaces the fixed
    /// regions and the families by a full 4 GiB each — still inside
    /// `[HEAP_LO, HEAP_HI)`, which is the only predicate the reader applies to
    /// any of these addresses — and keeps the stride, the order and the
    /// disjointness the default has.
    ///
    /// `province_slots` must be `1 (mod 3)`: the topology names `1..slots` and
    /// the neighbour rings are triples, exactly as [`PROVINCE_SLOTS`] is.
    pub(super) fn sized(province_slots: u32) -> Layout {
        assert!(
            province_slots % 3 == 1 && province_slots >= 7,
            "the topology names 1..slots in reciprocal triples, and the top \
             triple is the railless one",
        );
        // A power of two minus one, at least twice the slot count, so the
        // engine's open-addressed selector table has a free slot within the
        // probe's own reach for every province. The default's 511 against 385
        // slots is the same ratio.
        let mut selector_mask = SELECTOR_MASK;
        while selector_mask + 1 < province_slots.saturating_mul(2) {
            selector_mask = selector_mask * 2 + 1;
        }
        const SPAN: u64 = 0x1_0000_0000;
        let at = |index: u64| BASE + index * SPAN;
        Layout {
            province_slots,
            selector_mask,
            gamestate: at(0),
            idler_slot: at(1),
            idler_slot_b: at(1) + 0x1_0000,
            idler: at(2),
            owner: at(3),
            container: at(4),
            rail_manager: at(5),
            rail_table: at(6),
            active_root: at(7),
            active_table: at(8),
            selector_map: at(9),
            selector_table: at(10),
            source_array: at(11),
            map_array: at(12),
            rail_info: at(13),
            pair_blob: at(14),
            map_province: at(15),
            record: at(16),
            source_province: at(17),
            scatter_stride: SCATTER_STRIDE,
        }
    }

    /// Selector slots the fixture stages, mask plus the probe's tail.
    fn selector_slots(&self) -> u32 {
        self.selector_mask + 1 + SELECTOR_MAX_DISTANCE as u32 + 1
    }

    /// The one triple with no rail slot at all: the top one, which is
    /// [`RAILLESS_TRIPLE`] at the default size.
    fn railless_triple(&self) -> [u32; 3] {
        let top = self.province_slots - 1;
        [top - 2, top - 1, top]
    }

    fn rail_info_of(&self, province: u32) -> u64 {
        self.rail_info + u64::from(province) * self.scatter_stride
    }
    fn pair_blob_of(&self, province: u32) -> u64 {
        self.pair_blob + u64::from(province) * self.scatter_stride
    }
    fn map_province_of(&self, province: u32) -> u64 {
        self.map_province + u64::from(province) * self.scatter_stride
    }
    fn record_of(&self, province: u32) -> u64 {
        self.record + u64::from(province) * self.scatter_stride
    }
    fn source_province_of(&self, province: u32) -> u64 {
        self.source_province + u64::from(province) * self.scatter_stride
    }
}

fn rail_info_of(province: u32) -> u64 {
    Layout::DEFAULT.rail_info_of(province)
}
fn pair_blob_of(province: u32) -> u64 {
    Layout::DEFAULT.pair_blob_of(province)
}
fn map_province_of(province: u32) -> u64 {
    Layout::DEFAULT.map_province_of(province)
}
fn record_of(province: u32) -> u64 {
    Layout::DEFAULT.record_of(province)
}
fn source_province_of(province: u32) -> u64 {
    Layout::DEFAULT.source_province_of(province)
}

/// The two neighbours of a province, as the engine's own `(id, level)` pairs.
///
/// A RECIPROCAL TRIPLE: provinces `3k`, `3k+1`, `3k+2` name each other and
/// nobody else. The walk itself does not check reciprocity — the deferred fold
/// does — but a world whose rows the fold would refuse could never be used to
/// drive `d81_finish_rail_replay_state` to a publication, and that is the shape
/// the bracket-split fixture needs.
fn triple_of(province: u32) -> [u32; 3] {
    let base = province - (province - 1) % 3;
    [base, base + 1, base + 2]
}

fn neighbours_of(province: u32) -> [(u32, u32); 2] {
    let index = (province - 1) % 3;
    let base = province - index;
    [
        (base + (index + 1) % 3, 1),
        (base + (index + 2) % 3, 2),
    ]
}

fn anchor_of(province: u32, secondary: bool) -> [u32; 3] {
    let bias = if secondary { 0x1000 } else { 0 };
    [province + bias, province + 100 + bias, province + 200 + bias]
}

/// The engine's own selector hash, transcribed from `select_secondary`, so the
/// fixture can PLACE a key where the probe will look for it.
fn selector_slot_of(key: u32, mask: u32) -> u32 {
    let mut hash = (key ^ (key >> 16)).wrapping_mul(0x045d_9f3b);
    hash = (hash ^ (hash >> 16)).wrapping_mul(0x045d_9f3b);
    hash ^= hash >> 16;
    hash & mask
}

/// Which provinces get a SECONDARY anchor: the ones whose key the fixture puts
/// a non-zero value against. Half of them, so `anchor_variant` is a field that
/// varies and a walk that hard-coded either value would fail.
fn secondary(province: u32) -> bool {
    province % 2 == 0
}

fn active_window_of(province: u32) -> [u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN] {
    let mut window = [0u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN];
    // `active_terminal` is `u16 != 0 || a != 0 || b != 0 || flags & 1 != 0`, so
    // the u16 at +0x12 is left ZERO: writing the province id into it would make
    // every walked province terminal and the field a constant no walk could get
    // wrong. The two remaining contributors vary independently, so provinces
    // divisible by ten are non-terminal and the rest are not.
    window[0] = 0;
    window[1] = 0;
    window[2] = if province % 5 == 0 { 0 } else { 0x5A };
    window[3] = 0;
    window[4] = 0;
    window[5] = 0;
    window[6] = (province % 2) as u8;
    window
}

/// What one arm of the fixture asks the world to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WorldShape {
    /// Make this province's rail-info object unreadable. The plan copies it
    /// speculatively (it follows every non-null rail slot); whether the walk
    /// pays for it depends on whether the topology names the province.
    pub(super) unreadable_rail_info: Option<u32>,
    /// Leave this province's whole TRIPLE out of the topology, so the walk never
    /// visits them while the plan still follows their rail slots.
    ///
    /// A TRIPLE and not a province, because the three name each other: dropping
    /// one leaves its partners naming a row nobody published, which the walk
    /// refuses as  long before the void this arm is about.
    pub(super) skipped_triple: Option<u32>,
    /// Give this province a rail pointer outside the heap window.
    pub(super) rejected_rail_pointer: Option<u32>,
}

pub(super) struct StagedWorld {
    pub(super) memory: SyntheticMemory,
    pub(super) topology: CompactMapControlTopology,
    pub(super) valid_ids: BTreeSet<u32>,
    pub(super) container: D81MapProvinceContainer,
    pub(super) roots: Domain81PlanRoots,
}

pub(super) fn stage_world(shape: WorldShape) -> StagedWorld {
    stage_world_in(shape, Layout::DEFAULT)
}

/// [`stage_world`] at a chosen [`Layout`].
///
/// Every line below reads its addresses and its two counts off `layout`, so the
/// default size and a production-sized one are the SAME world staged twice and
/// not two fixtures that have to be kept in step.
pub(super) fn stage_world_in(shape: WorldShape, layout: Layout) -> StagedWorld {
    use crate::attach_log::*;
    let mut memory = SyntheticMemory::new();

    let province_slots = layout.province_slots;
    let selector_mask = layout.selector_mask;
    let selector_slots = layout.selector_slots();
    let railless_triple = layout.railless_triple();
    let (gamestate, idler_slot, idler_slot_b, idler, owner, container_base) = (
        layout.gamestate,
        layout.idler_slot,
        layout.idler_slot_b,
        layout.idler,
        layout.owner,
        layout.container,
    );
    let (rail_manager, rail_table, active_root, active_table) = (
        layout.rail_manager,
        layout.rail_table,
        layout.active_root,
        layout.active_table,
    );
    let (selector_map, selector_table, source_array, map_array) = (
        layout.selector_map,
        layout.selector_table,
        layout.source_array,
        layout.map_array,
    );

    memory.map_zeroed(gamestate, D81_GS_ANCHOR_SELECTOR_MAP_OFF + 8);
    memory.map_zeroed(idler_slot, 8);
    memory.map_zeroed(idler_slot_b, 8);
    memory.map_zeroed(idler, D81_IDLER_OWNER_OFF + 8);
    memory.map_zeroed(owner, D81_OWNER_CONTAINER_OFF + 8);
    memory.map_zeroed(container_base, D81_CONTAINER_PROVINCE_COUNT_OFF + 4);
    memory.map_zeroed(rail_manager, D81_RAIL_MANAGER_TABLE_OFF + 8);
    memory.map_zeroed(rail_table, province_slots as usize * 8);
    memory.map_zeroed(active_root, D81_ACTIVE_ROOT_TABLE_OFF + 8);
    let active_bytes = province_slots as usize * D81_ACTIVE_ENTRY_STRIDE;
    assert!(
        active_bytes as u64 > PAGE,
        "the active table must span more than one page or its scoping is untested",
    );
    memory.map_zeroed(active_table, active_bytes);
    memory.map_zeroed(selector_map, D81_SELECTOR_MAP_MAX_DISTANCE_OFF + 1);
    memory.map_zeroed(
        selector_table,
        (selector_slots as usize + 8) * D81_SELECTOR_ENTRY_STRIDE,
    );
    memory.map_zeroed(source_array, province_slots as usize * 8);
    memory.map_zeroed(map_array, province_slots as usize * 8);

    memory.write_u64(gamestate + D81_GS_RAIL_MANAGER_OFF as u64, rail_manager);
    memory.write_u64(gamestate + CGAMESTATE_CPROVINCE_ARRAY as u64, source_array);
    memory.write_u32(gamestate + CGAMESTATE_CPROVINCE_COUNT as u64, province_slots);
    memory.write_u64(
        gamestate + D81_GS_ACTIVE_PROVINCE_ROOT_OFF as u64,
        active_root,
    );
    memory.write_u64(
        gamestate + D81_GS_ANCHOR_SELECTOR_MAP_OFF as u64,
        selector_map,
    );
    memory.write_u64(rail_manager + D81_RAIL_MANAGER_TABLE_OFF as u64, rail_table);
    memory.write_u64(active_root + D81_ACTIVE_ROOT_TABLE_OFF as u64, active_table);
    memory.write_u64(selector_map + D81_SELECTOR_MAP_TABLE_OFF as u64, selector_table);
    memory.write_u32(selector_map + D81_SELECTOR_MAP_MASK_OFF as u64, selector_mask);
    memory.write(
        selector_map + D81_SELECTOR_MAP_MAX_DISTANCE_OFF as u64,
        &[SELECTOR_MAX_DISTANCE],
    );

    memory.write_u64(idler_slot, idler);
    memory.write_u64(idler_slot_b, idler);
    memory.write_u64(idler + D81_IDLER_OWNER_OFF as u64, owner);
    memory.write_u64(owner + D81_OWNER_CONTAINER_OFF as u64, container_base);
    memory.write_u64(
        container_base + D81_CONTAINER_PROVINCE_ARRAY_OFF as u64,
        map_array,
    );
    memory.write_u32(
        container_base + D81_CONTAINER_PROVINCE_COUNT_OFF as u64,
        province_slots,
    );

    // Every selector slot occupied, with a marker that admits the full probe
    // distance and a key no province owns, so a probe walks its whole distance
    // instead of stopping at the first empty slot.
    for slot in 0..selector_slots + 8 {
        let entry = selector_table + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
        memory.write(
            entry + D81_SELECTOR_MARKER_OFF as u64,
            &[SELECTOR_MAX_DISTANCE + 1],
        );
        memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, u32::MAX);
    }
    // The keys, placed the way an open-addressed table places them: forward, no
    // wrap, first free slot within the probe's own reach.
    let mut occupied = vec![false; (selector_slots + 8) as usize];
    for province in 0..province_slots {
        let start = selector_slot_of(province, selector_mask);
        for step in 0..=u32::from(SELECTOR_MAX_DISTANCE) {
            let slot = start + step;
            if occupied[slot as usize] {
                continue;
            }
            occupied[slot as usize] = true;
            let entry = selector_table + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
            memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, province);
            memory.write_u64(
                entry + D81_SELECTOR_VALUE_OFF as u64,
                if secondary(province) { 0xA000_0000 } else { 0 },
            );
            break;
        }
    }

    for province in 0..province_slots {
        let index = u64::from(province);
        memory.write(
            active_table + index * D81_ACTIVE_ENTRY_STRIDE as u64 + D81_ACTIVE_WINDOW_OFFSET as u64,
            &active_window_of(province),
        );
        let source = layout.source_province_of(province);
        memory.map_zeroed(source, crate::attach_log::CPROVINCE_OFF_ID + 0x20);
        memory.write_u64(source, CPROVINCE_VTABLE);
        memory.write_u32(source + crate::attach_log::CPROVINCE_OFF_ID as u64, province);
        memory.write_u64(source_array + index * 8, source);

        let map = layout.map_province_of(province);
        memory.map_zeroed(map, D81_ANCHOR_SECONDARY_OFF + 12 + 0x20);
        memory.write_u64(map_array + index * 8, map);
        let record = layout.record_of(province);
        memory.map_zeroed(record, D81_RECORD_SELECTOR_KEY_OFF + 0x10);
        memory.write_u64(map + D81_MAP_PROVINCE_RECORD_OFF as u64, record);
        // The key is the province's OWN id, which is the `RecordKeyIsProvinceId`
        // reciprocity witness and the only one t47 ever observed (3,510 of
        // 3,510, with `record_is_gamestate_province` at 0).
        memory.write_u32(record + D81_RECORD_SELECTOR_KEY_OFF as u64, province);
        for (axis, value) in anchor_of(province, false).iter().enumerate() {
            memory.write_u32(map + D81_ANCHOR_PRIMARY_OFF as u64 + axis as u64 * 4, *value);
        }
        for (axis, value) in anchor_of(province, true).iter().enumerate() {
            memory.write_u32(
                map + D81_ANCHOR_SECONDARY_OFF as u64 + axis as u64 * 4,
                *value,
            );
        }

        // A RAILLESS PROVINCE IS A NULL SLOT AND NOTHING ELSE, which is the
        // branch the walk takes for 9,903 of t26's 13,413 provinces. Slot 0 is
        // never walked and one whole triple is railless, so the branch is
        // exercised BY THE WALK and not only by the staging.
        if province == 0 || railless_triple.contains(&province) {
            memory.write_u64(rail_table + index * 8, 0);
            continue;
        }
        let info = layout.rail_info_of(province);
        memory.map_zeroed(info, RAIL_INFO_PAIRS_PTR_OFF + RAIL_HEADER_WINDOW_LEN + 0x20);
        let blob = layout.pair_blob_of(province);
        memory.map_zeroed(blob, 0x40);
        for (slot, (neighbour, level)) in neighbours_of(province).iter().enumerate() {
            memory.write_u32(blob + slot as u64 * 8, *neighbour);
            memory.write_u32(blob + slot as u64 * 8 + 4, *level);
        }
        memory.write_u64(info + RAIL_INFO_PAIRS_PTR_OFF as u64, blob);
        memory.write_u32(info + RAIL_INFO_PAIR_COUNT_OFF as u64, 2);
        // The four bytes between the two consumed fields carry a value, so a
        // whole-interval comparison would have something to see.
        memory.write_u32(
            info + RAIL_INFO_PAIRS_PTR_OFF as u64 + 8,
            0xDEAD_0000 + province,
        );

        let pointer = if shape.rejected_rail_pointer == Some(province) {
            // Readable and 8-aligned, and OUTSIDE the heap window: the case that
            // matters is a value the substrate would happily follow and the
            // reader rejects.
            0x1000u64
        } else {
            info
        };
        memory.write_u64(rail_table + index * 8, pointer);
    }

    if let Some(province) = shape.unreadable_rail_info {
        memory.set_readable(layout.rail_info_of(province), false);
    }

    let provinces: Vec<CompactMapProvinceRoot> = (1..province_slots)
        .filter(|province| {
            shape
                .skipped_triple
                .map(|skipped| !triple_of(skipped).contains(province))
                .unwrap_or(true)
        })
        .map(|province| CompactMapProvinceRoot {
            province_id: province,
            province_ptr: layout.source_province_of(province) as usize,
            state_id: 1,
        })
        .collect();
    let valid_ids = provinces.iter().map(|root| root.province_id).collect();
    let topology = CompactMapControlTopology {
        province_array: source_array as usize,
        province_count: province_slots,
        states: Vec::new(),
        provinces,
    };
    let container = D81MapProvinceContainer {
        container: container_base as usize,
        province_array: map_array as usize,
        province_count: province_slots,
    };
    let roots = Domain81PlanRoots {
        game_state: gamestate,
        idler_slot: idler_slot,
        idler_slot_b: idler_slot_b,
        container: container_base,
        province_slots: topology
            .provinces
            .iter()
            .map(|root| root.province_id)
            .max()
            .expect("the world names at least one province")
            + 1,
    };
    StagedWorld {
        memory,
        topology,
        valid_ids,
        container,
        roots,
    }
}

const WORLD_HOUR: HourStamp = HourStamp(1939_09_01);

/// A deferred compute the post-bracket half will PUBLISH.
///
/// Shared with `d81_bracket_split`, which needs a copy that computes to a
/// snapshot in order to prove that a refused bracket's copy does NOT. Since the
/// conversion the rows are not carried across the bracket — the COPY is, and the
/// walk decodes it on the far side — so a fixture that handed the compute half a
/// ready-made row list would be a fixture for a design that no longer exists.
///
/// The world's neighbours are reciprocal triples, so the deferred fold runs to
/// completion; an empty hub observation emits cleanly.
pub(super) fn staged_deferred() -> D81DeferredCompute {
    let world = stage_world(WorldShape::default());
    let mut plan = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let plan_snapshot = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan);
    plan.copies_issued = memory.calls();
    let StagedWorld {
        topology,
        valid_ids,
        container,
        ..
    } = world;
    D81DeferredCompute {
        started: Instant::now(),
        status_flags: 0,
        phases: D81PhaseAccum::default(),
        admitted_record_is_province: Cell::new(0),
        admitted_key_is_province_id: Cell::new(0),
        hub: D81HubObservation {
            foreign_tables: BTreeMap::new(),
            povs: Vec::new(),
            read_failure: None,
            reread_refused: Some(false),
        },
        closure_failure: None,
        hour_stamp: None,
        snapshot_bytes: 0,
        plan,
        plan_hour: WORLD_HOUR,
        plan_snapshot,
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
        live_walk: None,
        topology,
        valid_ids,
        container_snapshot: container,
        rail_table: RAIL_TABLE as usize,
        active_table: ACTIVE_TABLE as usize,
        selector_table: SELECTOR_TABLE as usize,
        selector_mask: SELECTOR_MASK,
        selector_max_distance: SELECTOR_MAX_DISTANCE,
        expected_cprovince_vtable: CPROVINCE_VTABLE as usize,
        module_base: 0,
        game_state: GAMESTATE as usize,
    }
}

/// How many rows [`staged_deferred`] publishes, so a caller can assert against a
/// number rather than against whatever it got.
pub(super) const STAGED_DEFERRED_ROWS: usize = PUBLISHED_ROWS;

/// Run the shipped walk over the world DIRECTLY.
fn direct_walk(world: &StagedWorld) -> Result<D81WalkRows, D81FirstFailure> {
    let phases = D81PhaseAccum::default();
    walk_with(world, &D81ReadSource::Staged(&world.memory), &phases)
}

/// Take the plan's copy of the world and run the shipped walk over THAT.
fn copied_walk(
    world: &StagedWorld,
    roots: &Domain81PlanRoots,
) -> (
    Result<D81WalkRows, D81FirstFailure>,
    crate::attach_log::d81_copy_bracket::D81PlanCounters,
) {
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    let source = closed
        .source_for(WORLD_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let phases = D81PhaseAccum::default();
    let rows = walk_with(world, &source, &phases);
    counters.misses = source.misses();
    (rows, counters)
}

/// Every copy the observation issues, in order, as `(address, length)`.
///
/// THE ONLY WAY TO PLACE A WRITE INSIDE A PARTICULAR WAVE. `MutatingMemory`
/// triggers on a global copy ORDINAL, and the ordinal of a mid-walk closing copy
/// is not derivable from the interval count the way the final wave's first copy
/// is: it depends on how many provinces the branch reached before it. So the
/// plan is run once against this, the transaction log is searched for the copy
/// wanted, and its ordinal becomes the trigger.
struct RecordingMemory<'a> {
    inner: &'a SyntheticMemory,
    log: std::cell::RefCell<Vec<(u64, u64)>>,
}

impl<'a> RecordingMemory<'a> {
    fn new(inner: &'a SyntheticMemory) -> Self {
        Self {
            inner,
            log: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The 1-indexed ordinals of the copies whose span contains `span`.
    fn ordinals_covering(&self, span: crate::snapshot_recipes::Span) -> Vec<u64> {
        self.log
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (addr, len))| {
                *addr <= span.start && span.end_excl() <= u128::from(*addr) + u128::from(*len)
            })
            .map(|(index, _)| index as u64 + 1)
            .collect()
    }
}

impl crate::snapshot_recipes::SnapshotMemory for RecordingMemory<'_> {
    fn copy_into(
        &self,
        addr: u64,
        out: &mut [u8],
    ) -> Result<(), crate::snapshot_recipes::MemoryRefusal> {
        let result = self.inner.copy_into(addr, out);
        if result.is_ok() {
            self.log.borrow_mut().push((addr, out.len() as u64));
        }
        result
    }
}

/// A memory that applies a DIFFERENT write before each of several ordinals.
///
/// `MutatingMemory` applies one edit set at one trigger, which can put two
/// distinct values into a copier's images but never three. A three-observation
/// loop needs three: with two values the second and third images are equal
/// whatever the mapping does, so a test built on one write cannot tell a third
/// attempt that reads image 2 from one that re-reads image 1.
struct ScriptedMemory<'a> {
    inner: std::cell::RefCell<SyntheticMemory>,
    script: Vec<(u64, u64, Vec<u8>)>,
    seen: std::cell::Cell<u64>,
    applied: std::cell::Cell<usize>,
    _life: std::marker::PhantomData<&'a ()>,
}

impl ScriptedMemory<'_> {
    /// `script` is `(ordinal, address, bytes)`, applied immediately BEFORE the
    /// copy with that 1-based ordinal.
    fn new(inner: SyntheticMemory, script: Vec<(u64, u64, Vec<u8>)>) -> Self {
        Self {
            inner: std::cell::RefCell::new(inner),
            script,
            seen: std::cell::Cell::new(0),
            applied: std::cell::Cell::new(0),
            _life: std::marker::PhantomData,
        }
    }

    fn applied(&self) -> usize {
        self.applied.get()
    }
}

impl crate::snapshot_recipes::SnapshotMemory for ScriptedMemory<'_> {
    fn copy_into(
        &self,
        addr: u64,
        out: &mut [u8],
    ) -> Result<(), crate::snapshot_recipes::MemoryRefusal> {
        let next = self.seen.get() + 1;
        for (at, target, bytes) in &self.script {
            if next == *at {
                self.inner.borrow_mut().write(*target, bytes);
                self.applied.set(self.applied.get() + 1);
            }
        }
        let result = self.inner.borrow().copy_into(addr, out);
        if result.is_ok() {
            self.seen.set(next);
        }
        result
    }
}

/// A memory that declines the `n`-th read OF ONE ADDRESS and serves everything
/// else.
///
/// `RefusingAtMemory` counts every transaction, which is what a COPY PLAN needs
/// — its ordinals are global. A DIRECT walk re-reads the same address several
/// times and the interesting failure is "the second observation of this window",
/// so this counts per address.
struct RefusingNthAtMemory<'a> {
    inner: &'a SyntheticMemory,
    addr: u64,
    nth: u32,
    seen: std::cell::Cell<u32>,
    refused: std::cell::Cell<bool>,
}

impl<'a> RefusingNthAtMemory<'a> {
    fn new(inner: &'a SyntheticMemory, addr: u64, nth: u32) -> Self {
        Self {
            inner,
            addr,
            nth,
            seen: std::cell::Cell::new(0),
            refused: std::cell::Cell::new(false),
        }
    }

    fn refused(&self) -> bool {
        self.refused.get()
    }
}

impl crate::snapshot_recipes::SnapshotMemory for RefusingNthAtMemory<'_> {
    fn copy_into(
        &self,
        addr: u64,
        out: &mut [u8],
    ) -> Result<(), crate::snapshot_recipes::MemoryRefusal> {
        if addr == self.addr {
            let next = self.seen.get() + 1;
            self.seen.set(next);
            if next == self.nth {
                self.refused.set(true);
                return Err(crate::snapshot_recipes::MemoryRefusal {
                    span: crate::snapshot_recipes::Span::new(addr, out.len() as u64),
                });
            }
        }
        self.inner.copy_into(addr, out)
    }
}

/// A memory that refuses exactly ONE transaction, by ordinal.
///
/// `SyntheticMemory::set_readable` refuses a whole region for the whole
/// observation, which cannot express "the engine declined the SECOND observation
/// of a span whose first one it served" — and that is the condition a live
/// reader's second closing read going unreadable produces.
struct RefusingAtMemory<'a> {
    inner: &'a SyntheticMemory,
    at: u64,
    seen: std::cell::Cell<u64>,
    refused: std::cell::Cell<bool>,
}

impl<'a> RefusingAtMemory<'a> {
    fn new(inner: &'a SyntheticMemory, at: u64) -> Self {
        Self {
            inner,
            at,
            seen: std::cell::Cell::new(0),
            refused: std::cell::Cell::new(false),
        }
    }

    fn refused(&self) -> bool {
        self.refused.get()
    }
}

impl crate::snapshot_recipes::SnapshotMemory for RefusingAtMemory<'_> {
    fn copy_into(
        &self,
        addr: u64,
        out: &mut [u8],
    ) -> Result<(), crate::snapshot_recipes::MemoryRefusal> {
        let next = self.seen.get() + 1;
        self.seen.set(next);
        if next == self.at {
            self.refused.set(true);
            return Err(crate::snapshot_recipes::MemoryRefusal {
                span: crate::snapshot_recipes::Span::new(addr, out.len() as u64),
            });
        }
        self.inner.copy_into(addr, out)
    }
}

/// Take the plan's copy of a world that CHANGES between the opening wave and the
/// closing one, and run the shipped walk over that copy.
///
/// The trigger is the first copy of the FINAL closing wave, MEASURED from a
/// clean run of the same plan rather than computed from the interval count.
///
/// IT USED TO BE `total - intervals + 1`, which held only while every interval
/// took ONE closing copy. Since the owner ruling of 2026-08-11 the closing wave
/// takes as many images as an interval's consumers asked for
/// ([`D81_CLOSING_IMAGES_HELD`] = 2 on the paired sites), so that expression
/// lands in the MIDDLE of the final wave: a write triggered there is already
/// after some intervals' closing images, and the tests below that say "before
/// the closing wave" were not placing it there. The observation reports the
/// count itself — `final_wave_copies` — so the boundary is read off the run that
/// took it.
///
/// MID-WALK CLOSES ARE NOT IN IT, and that is the point of asking for the FINAL
/// wave's copies rather than for `closing_copies`: the shipped plan closes its
/// rail-header stage during wave 3, and those copies are earlier in the
/// transaction stream than everything this trigger is meant to precede.
fn copied_walk_with_mutation(
    world: StagedWorld,
    edits: Vec<(u64, Vec<u8>)>,
) -> (
    Result<D81WalkRows, D81FirstFailure>,
    crate::attach_log::d81_copy_bracket::D81PlanCounters,
    bool,
) {
    let mut clean = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let probe = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&probe, &world.roots, WORLD_HOUR, &mut clean)
        .expect("custody must admit a fixture this size");
    let trigger = final_wave_start(probe.calls(), &closed);
    drop(closed);
    copied_walk_with_mutation_at(world, edits, trigger)
}

/// The transaction ordinal of the FINAL closing stages' first copy — the
/// vector pipeline's slot and active closings — given the total the memory
/// counted and the observation that produced it.
///
/// One function so the sites that need the boundary state it once. Ordinals
/// are 1-based, matching `CountingSnapshotMemory::calls()` and
/// `MutatingMemory`'s trigger. The bulk recipe's copies all precede the
/// pipeline's, so the global ordinal is the bulk total plus the pipeline's
/// own recorded boundary.
fn final_wave_start(total_copies: u64, observation: &crate::attach_log::D81PlanObservation) -> u64 {
    let vector = observation
        .vector
        .as_ref()
        .expect("the plan captured, so the vector pipeline ran");
    total_copies
        .saturating_sub(vector.copies.get())
        .saturating_add(vector.closing_tail_start)
        + 1
}

/// [`copied_walk_with_mutation`] with the trigger chosen by the caller, so a
/// write can be placed inside a MID-WALK closing wave rather than only ahead of
/// the final one.
fn copied_walk_with_mutation_at(
    world: StagedWorld,
    edits: Vec<(u64, Vec<u8>)>,
    trigger: u64,
) -> (
    Result<D81WalkRows, D81FirstFailure>,
    crate::attach_log::d81_copy_bracket::D81PlanCounters,
    bool,
) {
    use crate::snapshot_recipes::synthetic::MutatingMemory;

    let StagedWorld {
        memory,
        topology,
        valid_ids,
        container,
        roots,
    } = world;
    let mutating = MutatingMemory::new(memory, trigger, edits);
    let counting = CountingSnapshotMemory::new(&mutating);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    let applied = mutating.mutation_applied();
    let Ok(source) = closed.source_for(WORLD_HOUR) else {
        // NO VIEW AT ALL is the outcome `DisagreeAction::Record` exists to
        // prevent, so it is reported rather than panicked on: the caller
        // asserts against it.
        counters.consumer_gapped = true;
        return (
            Err(D81FirstFailure::new(
                D81FailureStage::CopyPlan,
                D81FailureReason::PlanGapped,
            )),
            counters,
            applied,
        );
    };
    let phases = D81PhaseAccum::default();
    let world = StagedWorld {
        memory: crate::snapshot_recipes::synthetic::SyntheticMemory::new(),
        topology,
        valid_ids,
        container,
        roots,
    };
    let rows = walk_with(&world, &source, &phases);
    counters.misses = source.misses();
    (rows, counters, applied)
}

fn walk_with(
    world: &StagedWorld,
    source: &D81ReadSource<'_>,
    phases: &D81PhaseAccum,
) -> Result<D81WalkRows, D81FirstFailure> {
    let record = Cell::new(0u32);
    let key = Cell::new(0u32);
    d81_walk_provinces(
        source,
        0,
        &world.topology,
        &world.valid_ids,
        &world.container,
        RAIL_TABLE as usize,
        ACTIVE_TABLE as usize,
        SELECTOR_TABLE as usize,
        SELECTOR_MASK,
        SELECTOR_MAX_DISTANCE,
        None,
        None,
        CPROVINCE_VTABLE as usize,
        phases,
        &record,
        &key,
    )
}

fn assert_same(
    direct: &Result<D81WalkRows, D81FirstFailure>,
    copied: &Result<D81WalkRows, D81FirstFailure>,
) {
    match (direct, copied) {
        (Ok(left), Ok(right)) => {
            assert_eq!(
                left.provinces, right.provinces,
                "the two arms must publish identical rows in identical order",
            );
            assert_eq!(
                left.observed_neighbors, right.observed_neighbors,
                "the two arms must agree about the neighbour total",
            );
        }
        (Err(left), Err(right)) => assert_eq!(
            left, right,
            "the two arms must refuse identically -- stage, reason, subject, \
             index and every witness",
        ),
        (Ok(rows), Err(failure)) => panic!(
            "the direct arm published {} rows where the copy refused {failure:?}",
            rows.provinces.len()
        ),
        (Err(failure), Ok(rows)) => panic!(
            "the direct arm refused {failure:?} where the copy published {} rows",
            rows.provinces.len()
        ),
    }
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// THE CONVERSION'S OWN EQUIVALENCE PROOF, offline.
///
/// One world, the shipped walk twice: reading the memory directly, and reading
/// the copy `domain81_copy_plan` took of it. Every published row, in order, and
/// the neighbour total.
///
/// MUTATION: point any op in `domain81_copy_plan` at a different offset, narrow
/// any of its `CopyRanges` windows, or drop one of its four table extents, and
/// the copied arm loses a field the direct arm has — which is a refusal here,
/// not a silently different row, because a view cannot answer for bytes nobody
/// copied.
#[test]
fn the_authoritative_walk_publishes_what_the_live_walk_publishes() {
    let world = stage_world(WorldShape::default());
    let direct = direct_walk(&world);
    let (copied, counters) = copied_walk(&world, &world.roots);

    let rows = direct
        .as_ref()
        .expect("the clean world must publish, or the gate compares two refusals");
    assert_eq!(
        rows.provinces.len(),
        PUBLISHED_ROWS,
        "every province the topology names publishes, except the railless triple",
    );
    // THE POPULATION HAS TO VARY, or a walk that hard-coded either value would
    // pass. Both anchor variants and both terminal states are present.
    assert!(
        rows.provinces.iter().any(|row| row.anchor_variant == 0)
            && rows.provinces.iter().any(|row| row.anchor_variant == 1),
        "the fixture must exercise both anchor variants",
    );
    assert!(
        rows.provinces.iter().any(|row| row.active_terminal)
            && rows.provinces.iter().any(|row| !row.active_terminal),
        "the fixture must exercise both terminal states",
    );

    assert_same(&direct, &copied);
    assert_eq!(
        counters.misses.total(),
        0,
        "a clean world must cost the copy no miss of any class: {:?}",
        counters.misses,
    );
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "the plan must copy every byte the walk consumes",
    );
    // EVERY SECOND OBSERVATION THE WALK ASKS FOR MUST EXIST (Codex 019feb50,
    // finding 6). `samples_absent` counts the reads whose `Sample(k)` the plan
    // never took and which were served the opening image instead — a walk-side
    // A/B check silently comparing one image with itself. It is deliberately
    // OUTSIDE `total()`, because it is not a failed read; asserting it here is
    // what makes "the walk's pair checks are real" a claim rather than a hope.
    assert_eq!(
        counters.misses.samples_absent, 0,
        "every `_stable` pair and every A/B pass must have a second image to \
         compare against: {:?}",
        counters.misses,
    );
}

/// THE IMAGES ARE DISTINCT, AND THE WALK READS THE ONE IT ASKED FOR.
///
/// The equivalence gate above reads an immutable world, so every `Sample(k)` and
/// `Closing` access could be rewritten to `Opening` and it would still pass
/// (Codex 019feb50, finding 6). This is the arm that fails: the world CHANGES
/// between the copier's images, so which image a site reads decides the answer.
///
/// A MAP-ARRAY slot is rewritten before the closing wave — not the rail header,
/// which this arm cannot reach (Codex 019feb9d, finding 4): the header's stage
/// closes MID-WALK, so a write placed at the first copy of the FINAL closing
/// wave lands after its image was already taken. What is visible here is the map
/// slot, whose interval closes in the final wave:
///  * an opening read that took the CLOSING image would bind the replacement and
///    find no disagreement at all;
///  * a closing read that took an OPENING image would compare the slot with
///    itself and publish with no refusal.
/// Neither is visible against an immutable world.
///
/// The rail header's own closing image is pinned STRUCTURALLY below, in
/// `the_shipped_default_is_the_copy_plan_and_the_switch_gates_the_reader`,
/// because no trigger this fixture can compute lands inside wave 3.
#[test]
fn the_walk_reads_the_image_it_asked_for_and_the_images_are_not_the_same() {
    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
    let at = MAP_ARRAY + u64::from(CHANGED) * 8;
    const REPLACEMENT: u64 = BASE + 0x0999_0000;
    let (walked, counters, applied) =
        copied_walk_with_mutation(world, vec![(at, REPLACEMENT.to_le_bytes().to_vec())]);
    assert!(applied);
    assert!(!counters.consumer_gapped);

    // THE OPENING SIDE. The walk bound this province to the ORIGINAL map
    // province — the one the opening image holds — and refuses because the
    // closing image disagrees. A walk whose opening read took the closing image
    // would have bound the replacement and found no disagreement at all.
    let failure = walked.expect_err("the two images disagree and the walk says so");
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(
        failure.raw_value, REPLACEMENT,
        "the CLOSING value must be the one the refusal carries, or the closing \
         read took the opening image",
    );

    // THE CONTROL, and it is what makes the arm above a measurement: the SAME
    // world with no mutation publishes the same province cleanly.
    let clean = stage_world(WorldShape::default());
    let (rows, _) = copied_walk(&clean, &clean.roots);
    assert!(
        rows.expect("the clean world publishes")
            .provinces
            .iter()
            .any(|row| row.province_id == CHANGED),
        "the fixture's disagreement must come from the mutation and not from \
         the province",
    );
}

/// RULING A, END TO END: a void the reader never touches costs nothing.
///
/// The plan follows EVERY non-null rail slot, including those of provinces the
/// topology walk skipped — it skips null gamestate slots, and a skipped index
/// can still carry a rail slot. So one of those objects is made unreadable here.
/// The copy records a scoped void for it; the walk never reads it; the frame
/// publishes every other row.
///
/// MUTATION: take `Tolerance::Scoped` off `op::RAIL_HEADER` in
/// `domain81_copy_plan` and the whole recipe refuses on an object nobody reads.
#[test]
fn a_void_in_an_object_the_walk_skips_costs_the_frame_nothing() {
    const SKIPPED: u32 = 200;
    let world = stage_world(WorldShape {
        unreadable_rail_info: Some(SKIPPED),
        skipped_triple: Some(SKIPPED),
        ..WorldShape::default()
    });
    let direct = direct_walk(&world);
    let (copied, counters) = copied_walk(&world, &world.roots);

    let rows = direct.as_ref().expect("the walk never visits the void");
    assert_eq!(rows.provinces.len(), PUBLISHED_ROWS - 3);
    assert!(
        !rows.provinces.iter().any(|row| row.province_id == SKIPPED),
        "the skipped province must not appear, or the world is not the one \
         described",
    );
    assert_same(&direct, &copied);
    assert!(
        counters.scoped_voids > 0,
        "the copy must have RECORDED the void, or the arm proves only that the \
         object was readable after all",
    );
    assert_eq!(
        counters.misses.total(),
        0,
        "no read of the walk's touched the void: {:?}",
        counters.misses,
    );
}

/// THE SAME VOID, IN AN OBJECT THE WALK DOES READ, refuses identically on both
/// arms.
///
/// This is the consumed-span law's other half: the void costs exactly the rows
/// whose consumed bytes it holds, which for d81 is the frame, because the live
/// walk refuses the whole frame on an unreadable rail header and the converted
/// one must not quietly do something gentler.
#[test]
fn a_void_in_an_object_the_walk_reads_refuses_on_both_arms_identically() {
    const VISITED: u32 = 100;
    let world = stage_world(WorldShape {
        unreadable_rail_info: Some(VISITED),
        ..WorldShape::default()
    });
    let direct = direct_walk(&world);
    let (copied, counters) = copied_walk(&world, &world.roots);

    let failure = direct
        .as_ref()
        .expect_err("an unreadable rail header refuses the frame");
    assert_eq!(failure.stage, D81FailureStage::RailHeader);
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(failure.subject_id, VISITED);
    assert_same(&direct, &copied);
    // AND THE COPY'S OWN ACCOUNT NAMES THE ENGINE, not the plan. A capture that
    // read this as `not_captured` would be reading a degraded game as a broken
    // conversion.
    assert!(counters.misses.unreadable > 0);
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "a void is the engine refusing, never the plan under-asking: {:?}",
        counters.misses,
    );
}

/// A REJECTED RAIL POINTER refuses identically on both arms.
///
/// The plan admits pointers on `is_map_control_heap_pointer`, the same predicate
/// the walk rejects them with, so the substrate records a rejection and stops
/// the branch — and the walk, reading the copy, forms the same address, reads
/// the same slot value and raises its own `InvalidPointer` with the same raw
/// witness.
#[test]
fn a_rejected_rail_pointer_refuses_on_both_arms_identically() {
    const REJECTED: u32 = 33;
    let world = stage_world(WorldShape {
        rejected_rail_pointer: Some(REJECTED),
        ..WorldShape::default()
    });
    let direct = direct_walk(&world);
    let (copied, counters) = copied_walk(&world, &world.roots);

    let failure = direct
        .as_ref()
        .expect_err("a rail pointer outside the heap window refuses the frame");
    assert_eq!(failure.stage, D81FailureStage::RailHeader);
    assert_eq!(failure.reason, D81FailureReason::InvalidPointer);
    assert_eq!(failure.subject_id, REJECTED);
    assert_eq!(failure.raw_value, 0x1000);
    assert_same(&direct, &copied);
    assert!(
        counters.pointers_rejected > 0,
        "the copy must have recorded the rejection it declined to follow",
    );
}

/// A PLAN THAT UNDER-COPIES IS NAMED AS A PLAN DEFECT, never as an unreadable
/// game.
///
/// The one distinction the conversion turns on. The plan here is built with a
/// deliberately short `province_slots`, so its rail-table extent stops before
/// the provinces the walk still visits: the view has no bytes for them, the walk
/// refuses, and the refusal must say `NotCopied` with the missing address rather
/// than `Unreadable`.
///
/// MUTATION: drop the `plan_defects()` test out of `d81_restamp_plan_defect` and
/// this frame publishes `d81_failure_reason_03_unreadable`, which is the wire
/// telling a capture the game went unreadable when the plan simply did not ask.
#[test]
fn a_plan_that_under_copies_is_named_a_plan_defect_and_not_an_unreadable_game() {
    let world = stage_world(WorldShape::default());
    let short = Domain81PlanRoots {
        province_slots: PROVINCES / 2,
        ..world.roots
    };
    let (copied, counters) = copied_walk(&world, &short);

    let failure = copied.expect_err("a table that stops short cannot serve the walk");
    assert!(
        counters.misses.not_captured > 0,
        "the miss must be classed as bytes the plan never asked for: {:?}",
        counters.misses,
    );
    assert_eq!(counters.misses.unreadable, 0);
    let restamped = d81_restamp_plan_defect(failure, counters.misses);
    assert_eq!(
        restamped.reason,
        D81FailureReason::NotCopied,
        "an under-copying plan must not publish as an unreadable engine",
    );
    assert_eq!(
        restamped.raw_value, counters.misses.first_addr,
        "the refusal must carry the address of the span the plan missed",
    );
    // The stage stays the walk's own: the rename corrects WHY, never WHERE.
    assert_eq!(restamped.stage, D81FailureStage::RailHeader);
}

/// THE COPY IS HOUR-LOCKED, and the lock is structural rather than a discipline.
///
/// A view can only be taken at the snapshot's own hour, so a compute half that
/// tried to read this hour's copy under any other label has no handle at all.
/// That is the half of the hour-attribution rule an API can enforce.
#[test]
fn a_view_of_the_copy_cannot_be_taken_at_another_hour() {
    let world = stage_world(WorldShape::default());
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    assert!(closed.closed.snapshot.view_for(D81_CONSUMER, WORLD_HOUR).is_ok());
    assert!(
        closed
            .closed
            .snapshot
            .view_for(D81_CONSUMER, HourStamp(WORLD_HOUR.0 + 1))
            .is_err(),
        "a copy taken in one hour must refuse to answer for the next",
    );
}

/// THE EQUALITY MODE FINDS A DIVERGENCE WHEN THERE IS ONE, and reports none when
/// there is not.
///
/// Without this arm the mode is a function that has only ever been run on equal
/// inputs, and "0 divergent" would be a number nothing could have made non-zero.
#[test]
fn the_equality_mode_names_the_first_row_and_field_that_disagree() {
    use crate::attach_log::d81_copy_bracket::{d81_equality_compare, D81EqualityCounters};

    let world = stage_world(WorldShape::default());
    let rows = direct_walk(&world).expect("the clean world publishes");
    let mut agreeing = D81EqualityCounters::default();
    d81_equality_compare(
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: rows.provinces.clone(),
            observed_neighbors: rows.observed_neighbors,
        })),
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: rows.provinces.clone(),
            observed_neighbors: rows.observed_neighbors,
        })),
        &mut agreeing,
    );
    assert_eq!(agreeing.rows_divergent, 0);
    assert_eq!(agreeing.rows_equal, PUBLISHED_ROWS as u32);
    assert!(agreeing.failures_agree);

    // One row's anchor changed, three rows in.
    let mut altered = rows.provinces.clone();
    altered[3].anchor_bits[1] ^= 1;
    let mut diverging = D81EqualityCounters::default();
    d81_equality_compare(
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: rows.provinces.clone(),
            observed_neighbors: rows.observed_neighbors,
        })),
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: altered,
            observed_neighbors: rows.observed_neighbors,
        })),
        &mut diverging,
    );
    assert_eq!(diverging.rows_divergent, 1);
    assert_eq!(diverging.first_divergence_province, rows.provinces[3].province_id);
    assert_eq!(
        crate::attach_log::d81_copy_bracket::d81_shadow_divergence_unit_name(
            diverging.first_divergence_field,
        ),
        "d81_shadow_divergence_05_anchor_bits",
    );

    // A row one arm published and the other did not is `row_present`, not a
    // comparison that quietly did not happen.
    let mut short = rows.provinces.clone();
    short.pop();
    let mut missing = D81EqualityCounters::default();
    d81_equality_compare(
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: rows.provinces.clone(),
            observed_neighbors: rows.observed_neighbors,
        })),
        &D81WalkRows::verdict(&Ok(D81WalkRows {
            provinces: short,
            observed_neighbors: rows.observed_neighbors,
        })),
        &mut missing,
    );
    assert_eq!(missing.rows_compared, PUBLISHED_ROWS as u32);
    assert_eq!(missing.rows_divergent, 1);
    assert_eq!(
        crate::attach_log::d81_copy_bracket::d81_shadow_divergence_unit_name(
            missing.first_divergence_field,
        ),
        "d81_shadow_divergence_01_row_present",
    );

    // And two arms that refused DIFFERENTLY disagree, where two that refused
    // identically agree.
    //
    // THE TWO DIFFER BELOW THE PACKED WORD, on purpose (Codex 019feb7d, finding
    // 10). Same stage, same reason, different province: a comparison over
    // `(stage << 16) | reason` — which is what the wire carries and what the
    // first version of this compared — calls them equal, and two arms refusing
    // at different provinces are not in agreement.
    let refusal = D81FirstFailure::new(D81FailureStage::RailHeader, D81FailureReason::Unreadable)
        .subject(11)
        .index(2);
    let other = D81FirstFailure::new(D81FailureStage::RailHeader, D81FailureReason::Unreadable)
        .subject(12)
        .index(2);
    assert_eq!(
        d81_failure_witness_word(&refusal),
        d81_failure_witness_word(&other),
        "the two must be indistinguishable to the packed word, or this arm \
         passes on a comparison that never looked below it",
    );
    let mut same_refusal = D81EqualityCounters::default();
    d81_equality_compare(
        &D81WalkRows::verdict(&Err(refusal)),
        &D81WalkRows::verdict(&Err(refusal)),
        &mut same_refusal,
    );
    assert!(same_refusal.failures_agree);
    let mut different_refusal = D81EqualityCounters::default();
    d81_equality_compare(
        &D81WalkRows::verdict(&Err(refusal)),
        &D81WalkRows::verdict(&Err(other)),
        &mut different_refusal,
    );
    assert!(!different_refusal.failures_agree);
}

/// THE PRODUCTION DEFAULT IS THE RECIPE, AND THE SWITCH IS LOAD-BEARING.
///
/// The value alone would be a claim about a constant. What makes it a claim
/// about the READER is the three places it gates, pinned here against the
/// source, because a switch that gates nothing is a switch that cannot be turned
/// off — and then "the recipe is the reader" is unfalsifiable.
///
///  * the copy is TAKEN only when the switch is on, so `false` costs a frame
///    nothing rather than copying and discarding;
///  * the retired live walk runs when the lever is armed OR the switch is off,
///    so `false` leaves a reader rather than a hole;
///  * the compute half publishes the recipe's rows when it has them and the live
///    walk's otherwise, and never falls from one to the other.
///
/// MUTATION: delete any of the three gates and this fails naming which.
#[test]
fn the_shipped_default_is_the_copy_plan_and_the_switch_gates_the_reader() {
    assert!(
        crate::attach_log::d81_copy_bracket::D81_COPY_BRACKET_AUTHORITATIVE,
        "the recipe program is d81's production reader",
    );
    // Normalised: every pattern below spans lines and the file is CRLF on disk.
    let source = collector_source_lf();
    assert!(
        source.contains(
            "let plan_snapshot = if d81_copy_bracket::D81_COPY_BRACKET_AUTHORITATIVE {"
        ),
        "the copy must be taken only when it is the reader; copying and \
         discarding is a cost with no answer attached",
    );
    assert!(
        source.contains(
            "let live_walk = if equality.armed || !d81_copy_bracket::D81_COPY_BRACKET_AUTHORITATIVE {"
        ),
        "the retired walk must run when the switch is off, or turning the \
         conversion off leaves d81 with no reader at all",
    );
    assert!(
        source.contains("let walked = match (recipe_walk, live_walk) {"),
        "the published rows must be chosen between the two arms explicitly",
    );
    // AND THERE IS NO FALLBACK. The one thing this design may not do is read the
    // engine again because the copy could not answer: those bytes were never
    // observed by the labelled bracket. The compute half is where such a
    // fallback would have to live, so it is where the absence is asserted.
    let finish = source
        .split_once("fn d81_finish_rail_replay_state(")
        .expect("the post-bracket half")
        .1
        .split_once("\nfn ")
        .expect("the end of the post-bracket half")
        .0;
    // EVERY LIVE PRIMITIVE, not one constructor spelling (Codex 019feb50,
    // finding 7). A direct `read_into` or `read_u64_stable` in the compute half
    // violates hour attribution just as completely as a `Live` source does, and
    // pinning only the source type would leave that door open.
    for live in [
        "D81ReadSource::Live",
        "read_into(",
        "read_scalar",
        "read_u64_stable",
        "read_u32_stable",
        "read_u8_stable",
        "read_u64(",
        "read_u32(",
        "read_u8(",
        "d81_read_stable_bytes",
        "D81ExtentPair::copy",
        "is_map_control_heap_pointer",
    ] {
        assert!(
            !finish.contains(live),
            "the compute half must not reach {live}: a live re-read after the \
             bracket closed publishes bytes the labelled hour never observed",
        );
    }
    // THE CLOSING SITES TAKE THE CLOSING IMAGE. Two of the four are proved by
    // BEHAVIOUR — `the_rail_header_closes_against_its_own_closing_image` for the
    // mid-walk stage and `a_closing_change_the_walk_closes_over_...` for the
    // final wave — and the two that no fixture reaches are pinned by name. A
    // source pin is the weaker instrument and is used only where the stronger
    // one is unavailable.
    for closing_site in [
        ".u64_stable_at(record_address, d81_copy_bracket::D81Image::Closing)",
        "            d81_copy_bracket::D81Image::Closing,\n        ) {",
    ] {
        assert!(
            source.contains(closing_site),
            "a closing re-read must take the CLOSING image: {closing_site}",
        );
    }
    // THE PLAN MODULE'S OWN TEXT, normalised: the read source lives in
    // `d81_copy_bracket.rs`, not in the reader.
    let plan_source = include_str!("../d81_copy_bracket.rs").replace("\r\n", "\n");
    let plan_source = plan_source.as_str();
    // THE ROOT AGREEMENT CHECK IS WIRED, not merely present. Its own gate calls
    // `d81_copied_roots_agree` directly, so deleting the CALL from the compute
    // half leaves that gate passing while a table the engine replaced goes back
    // to being blamed on the plan.
    assert!(
        source.contains("let rows = d81_copied_roots_agree("),
        "the compute half must ask the copy which tables it followed before it \
         walks them",
    );
    // AND A REQUESTED IMAGE THE COPIER DOES NOT HOLD FAILS, rather than being
    // served the opening one. A clean fixture has every image it asks for, so
    // `samples_absent == 0` cannot see this; the substitution is a two-line arm
    // and it is pinned as one.
    assert!(
        plan_source.contains(
            "Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {\n                        self.note_absent_sample();\n                        false\n                    }"
        ),
        "a requested adjacent image the observation does not hold must FAIL at \
         its reading site; serving the opening image in its place makes every \
         walk-side A/B a comparison of one image with itself",
    );

    // AND THE SNAPSHOT ARM ITSELF NEVER REACHES A LIVE PRIMITIVE (Codex
    // 019feb9d, finding 5). The pin below checks the walk's CALLERS; this checks
    // the abstraction they call through, which is the one place a fallback could
    // be added that every other assertion here would pass.
    let snapshot_impl = plan_source
        .split_once("impl D81SnapshotSource<'_> {")
        .expect("the snapshot source")
        .1
        .split_once("\n}")
        .expect("the end of the snapshot source")
        .0;
    for live in ["super::read_into", "super::read_scalar", "read_u64_stable"] {
        assert!(
            !snapshot_impl.contains(live),
            "the snapshot arm must never reach {live}: a fallback there is the \
             silent post-bracket live read every other pin here assumes away",
        );
    }
    // AND THE LIVE ARM IS THE ONLY ARM THAT DOES.
    let source_impl = plan_source
        .split_once("pub fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {")
        .expect("the read source's one primitive")
        .1
        .split_once("\n    }")
        .expect("the end of the primitive")
        .0;
    assert_eq!(
        source_impl.matches("super::read_into").count(),
        1,
        "exactly one arm of the read source may touch the process, and it is \
         the LIVE one",
    );

    // AND THE WALK AND EVERY HELPER IT REACHES. `d81_walk_provinces` reads
    // through `D81ReadSource` and nothing else, and so must each of the five
    // functions it calls that read at all — otherwise the pin passes while a
    // bare `read_u64_stable` inside `d81_map_province_slot` puts a live read in
    // the compute half (Codex 019feb7d, finding 11).
    let body_of = |name: &str| -> &str {
        source
            .split_once(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("{name} must exist"))
            .1
            .split_once("\nfn ")
            .unwrap_or_else(|| panic!("the end of {name}"))
            .0
    };
    let reachable = [
        "d81_walk_provinces",
        "d81_map_province_slot",
        "d81_admit_map_province_reciprocity",
        "d81_resolve_anchor_selector_key",
        "d81_read_u64_pair_at",
        "d81_read_coherent_active_entry_at",
        "d81_copied_roots_agree",
    ];
    // WHITESPACE REMOVED, because `rustfmt` breaks `source.u64_stable(addr)`
    // across lines the moment the receiver is long enough: a counting pin that
    // matched only the one-line spelling would report every wrapped call as a
    // bare live read.
    let walk: String = reachable
        .iter()
        .flat_map(|name| body_of(name).chars())
        .filter(|character| !character.is_whitespace())
        .collect();
    let walk = walk.as_str();
    for primitive in [
        "read_into(",
        "u64_stable(",
        "u32_stable(",
        "u8_stable(",
        "u64_at(",
        "u32_at(",
        "u8_at(",
        "scalar::<",
    ] {
        // EVERY occurrence must be a call ON THE SOURCE. A bare one is the same
        // primitive reading the live process, which on the authoritative arm is
        // a read outside the labelled bracket.
        let total = walk.matches(primitive).count();
        let through_source = walk.matches(&format!("source.{primitive}")).count()
            + walk.matches(&format!("batch.{primitive}")).count();
        assert_eq!(
            total, through_source,
            "the walk must read only through its source; {total} uses of \
             {primitive} against {through_source} that go through it",
        );
    }
}

/// A CHANGED CLOSING IMAGE DOES NOT GAP THE CONSUMER; THE WALK JUDGES IT.
///
/// The substrate's default is to compare every consumed byte of every enrolled
/// interval and gap the consumer if any of them moved. For d81 that is not the
/// live walk's reach — t46 measured 78 of 80 frames gapped by it — so every op
/// of the shipped plan declares `CloseAction::Record` and the walk closes what
/// its live form closed, in its own vocabulary.
///
/// Here the MAP-ARRAY slot of a walked province is rewritten between the opening
/// wave and the closing one. The frame must still HAVE a view, and the walk must
/// refuse `MapProvince`/`ClosureChanged` naming that province — the exact
/// refusal the live walk raises for the same write, because the walk re-reads
/// that slot after taking the anchor off the object behind it.
///
/// THE MAP ARRAY AND NOT THE RAIL HEADER, and the reason is worth stating: the
/// header's stage closes MID-WALK (`CloseWhen::AfterWave(3)`), so a write placed
/// at the start of the FINAL closing wave lands after its image was already
/// taken. That is mid-walk closure working, and it is why this arm uses an
/// interval that closes in the final wave.
///
/// MUTATION: put `CloseAction::Gap` back on `op::MAP_ARRAY` and the consumer is
/// gapped, the view is gone, and the frame publishes the anonymous
/// `CopyPlan`/`PlanGapped` instead.
#[test]
fn a_closing_change_the_walk_closes_over_refuses_in_the_walks_own_vocabulary() {
    const CHANGED: u32 = 7;
    const REPLACEMENT: u64 = BASE + 0x0999_0000;
    let world = stage_world(WorldShape::default());
    let at = MAP_ARRAY + u64::from(CHANGED) * 8;
    let (walked, counters, applied) =
        copied_walk_with_mutation(world, vec![(at, REPLACEMENT.to_le_bytes().to_vec())]);

    assert!(applied, "the fixture must actually have rewritten the slot");
    assert!(
        !counters.consumer_gapped,
        "a change the walk closes over must leave the view intact, or the \
         refusal below is the substrate's and not the reader's",
    );
    let failure = walked.expect_err("a map slot that changed under the copier refuses");
    assert_eq!(failure.stage, D81FailureStage::MapProvince);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(
        failure.raw_value, REPLACEMENT,
        "the refusal must carry the CLOSING pointer, which is what the live walk \
         puts in `raw`",
    );
}

/// AND A CHANGE THE WALK NEVER CLOSES OVER COSTS THE FRAME NOTHING.
///
/// The other half of the same claim, and the one that decides delivery. The
/// walk reads each rail SLOT once and never re-reads it, so a slot rewritten
/// under the copier was never a refusal live. Under the substrate's default it
/// is: the rail table is one interval, consumed whole, and any byte of it moving
/// gaps the consumer.
///
/// The slot rewritten here belongs to a province of the RAILLESS triple, so the
/// walk does not even branch on it — this is a byte of a copied table that
/// nothing reads.
///
/// MUTATION: put `CloseAction::Gap` back on `op::RAIL_TABLE` and every row of
/// the frame is lost to one byte nobody looked at.
#[test]
fn a_closing_change_the_walk_never_reads_costs_the_frame_nothing() {
    let world = stage_world(WorldShape::default());
    let at = RAIL_TABLE + u64::from(RAILLESS_TRIPLE[0]) * 8;
    let (walked, counters, applied) =
        copied_walk_with_mutation(world, vec![(at, 0xDEADu64.to_le_bytes().to_vec())]);

    assert!(applied, "the fixture must actually have rewritten the slot");
    assert!(
        !counters.consumer_gapped,
        "a byte the walk never reads must not cost the consumer its hour",
    );
    let rows = walked.expect("the frame publishes");
    assert_eq!(
        rows.provinces.len(),
        PUBLISHED_ROWS,
        "every row must survive a write to a table byte nothing consumed",
    );
}

/// THE COPY'S OWN ROOTS ARE THE ROOTS THE WALK USES.
///
/// The plan decodes its table bases out of the copied gamestate; the walk is
/// handed the ones the reader resolved live. A frame where they disagree is a
/// frame whose ENGINE moved a table, and it must publish as
/// `FullClosure`/`ClosureChanged` — the vocabulary the live walk uses for a root
/// that moved — and never as a plan defect (Codex 019feb7d, finding 4).
///
/// MUTATION: delete the `d81_copied_roots_agree` call from the compute half and
/// this frame publishes `NotCopied` instead, blaming the plan for a table the
/// engine replaced.
#[test]
fn a_table_the_engine_replaced_refuses_as_a_closure_change_and_not_a_plan_defect() {
    let world = stage_world(WorldShape::default());
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    let source = closed.source_for(WORLD_HOUR).expect("a view");

    // The copy followed the world's real rail table, so agreement holds.
    d81_copied_roots_agree(
        &source,
        GAMESTATE as usize,
        RAIL_TABLE as usize,
        ACTIVE_TABLE as usize,
        SELECTOR_TABLE as usize,
    )
    .expect("an unmoved world must agree with its own copy");

    // And a walk handed a DIFFERENT table than the copy followed refuses as a
    // closure change, naming which root and carrying what the copy holds.
    let failure = d81_copied_roots_agree(
        &source,
        GAMESTATE as usize,
        RAIL_TABLE as usize + 8,
        ACTIVE_TABLE as usize,
        SELECTOR_TABLE as usize,
    )
    .expect_err("a table the walk does not share with the copy refuses");
    assert_eq!(failure.stage, D81FailureStage::FullClosure);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(
        failure.raw_value, RAIL_TABLE,
        "the refusal must carry what the COPY followed",
    );
}

/// THE RESTAMP NAMES THE READ THAT FAILED, not the first miss of the frame.
///
/// Two misses in one walk: an earlier one at one address and the terminal one at
/// another. The published refusal must carry the TERMINAL address, or a capture
/// chases a span that was not the one that ended the walk (Codex 019feb7d,
/// finding 9).
///
/// MUTATION: put `misses.first_addr` back in `d81_restamp_plan_defect` and this
/// fails on the address.
#[test]
fn the_restamp_carries_the_address_of_the_read_that_ended_the_walk() {
    use crate::attach_log::d81_copy_bracket::{D81ViewMisses, D81_MISS_NOT_CAPTURED};

    let misses = D81ViewMisses {
        not_captured: 2,
        first_class: D81_MISS_NOT_CAPTURED,
        first_addr: 0x1000,
        first_len: 8,
        last_class: D81_MISS_NOT_CAPTURED,
        last_addr: 0x2000,
        ..D81ViewMisses::default()
    };
    let failure = D81FirstFailure::new(D81FailureStage::Anchor, D81FailureReason::Unreadable)
        .subject(42);
    let restamped = d81_restamp_plan_defect(failure, misses);
    assert_eq!(restamped.reason, D81FailureReason::NotCopied);
    assert_eq!(
        restamped.raw_value, 0x2000,
        "the refusal must carry the LAST miss -- the read that failed -- and \
         not the first, which may belong to a read that recovered",
    );
    assert_eq!(restamped.subject_id, 42, "the site's own witness survives");

    // AND A TERMINAL MISS THAT WAS THE ENGINE IS NOT RENAMED. `Unreadable` with
    // the last miss a void is ruling A working, and calling it a plan defect
    // would report a degraded game as a broken conversion.
    let engine = D81ViewMisses {
        unreadable: 1,
        not_captured: 1,
        first_class: D81_MISS_NOT_CAPTURED,
        first_addr: 0x1000,
        last_class: crate::attach_log::d81_copy_bracket::D81_MISS_UNREADABLE,
        last_addr: 0x3000,
        ..D81ViewMisses::default()
    };
    let kept = d81_restamp_plan_defect(failure, engine);
    assert_eq!(kept.reason, D81FailureReason::Unreadable);
}

/// THE CLOSING SEQUENCE IS AS LONG AS THE OPENING ONE, and the mapping knows it.
///
/// The copier takes [`D81_CLOSING_IMAGES_HELD`] closing observations of every
/// interval and `D81_ACTIVE_COHERENCE_READS` of the active window, so every
/// walk-side A/B has two adjacent observations to compare at the CLOSING instant
/// exactly as it does at the opening. That is tear-detection parity with the
/// live walk, and it is what the owner funded on 2026-08-11.
///
/// WHAT MAKES IT FAIL: the substrate taking fewer closing images than the
/// mapping asks for, or `paired_image` going back to returning `Closing` for a
/// closing pair -- at which point the pair compares one image with itself and
/// the three scenarios below come back.
/// kills: `D81_CLOSING_IMAGES_HELD = 2` -> `1`, and `paired_image(Closing)`
///   -> `Closing` in d81_copy_bracket.rs (the mapping and the count are both
///   asserted, and the copy count is read off the transaction log)
#[test]
fn the_closing_sequence_matches_the_opening_one_and_the_mapping_knows_it() {
    use crate::attach_log::d81_copy_bracket::{D81Image, D81ReadSource, D81_CLOSING_IMAGES_HELD};

    assert_eq!(
        D81_CLOSING_IMAGES_HELD, 2,
        "a stable pair needs two closing observations, as it needs two opening \
         ones",
    );
    assert_eq!(
        D81ReadSource::paired_image(D81Image::Closing),
        D81Image::ClosingSample(1),
        "a closing pair must compare two ADJACENT closing observations",
    );
    assert_eq!(
        D81ReadSource::paired_image(D81Image::Opening),
        D81Image::Sample(1),
    );
    // A pair OF a sample is that sample: the walk never nests one, and returning
    // the next index would silently reach past what the sequence holds.
    assert_eq!(
        D81ReadSource::paired_image(D81Image::ClosingSample(1)),
        D81Image::ClosingSample(1),
    );

    // AND THE SUBSTRATE REALLY TAKES THEM. The count above is a mapping; this is
    // the observation, counted off the transaction log.
    let world = stage_world(WorldShape::default());
    let slot = crate::snapshot_recipes::Span::new(MAP_ARRAY + 7 * 8, 8);
    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    assert_eq!(
        recorder.ordinals_covering(slot).len(),
        2 * D81_CLOSING_IMAGES_HELD,
        "two opening observations and two closing ones, for an interval whose \
         reader compares a stable pair at both",
    );
    // AND THE ACTIVE TABLE TAKES ITS OWN LONGER SEQUENCE at both instants
    // FOR A RAILED PROVINCE, because its reader takes three observations
    // rather than two: three from the whole-table opening triple and three
    // from the vector pipeline's per-entry closing family. Province 1 is
    // railed at every layout (province 0 and the top triple are the railless
    // ones).
    let railed_window = crate::snapshot_recipes::Span::new(
        ACTIVE_TABLE
            + crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
            + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64,
        crate::attach_log::D81_ACTIVE_WINDOW_LEN as u64,
    );
    assert_eq!(
        recorder.ordinals_covering(railed_window).len(),
        2 * crate::attach_log::D81_ACTIVE_COHERENCE_READS,
        "the coherence loop needs as many closing observations as it takes reads",
    );
    // A RAILLESS province's entry takes the opening triple ONLY: the live
    // walk never closes over an entry it never visited, so the demand-driven
    // pipeline observes no closing images of it. The old recipe's three
    // whole-table closing copies covered it anyway — dead observations, and
    // this asserts they are gone rather than merely unclaimed.
    let railless_window = crate::snapshot_recipes::Span::new(
        ACTIVE_TABLE + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64,
        crate::attach_log::D81_ACTIVE_WINDOW_LEN as u64,
    );
    assert_eq!(
        recorder.ordinals_covering(railless_window).len(),
        crate::attach_log::D81_ACTIVE_COHERENCE_READS,
        "a railless entry is observed at the opening only",
    );
}

/// THE ROW'S CONTENT IS NOT ALWAYS THE OPENING IMAGE'S, demonstrated.
///
/// This is the arm that disproves a claim an earlier version of this file made
/// and that an owner decision would have rested on (Codex 019febfa, round 4).
/// `d81_read_coherent_active_entry` publishes the first CONSECUTIVE PAIR that
/// agrees among its observations, so a sequence `A, B, B` publishes `B` — not
/// `A`, the first one. The copy reproduces that faithfully with three images, so
/// the authoritative arm publishes `B` too.
///
/// The consequence for the residual: the two arms of the equality mode take
/// their observations at DIFFERENT instants, so a field the engine rewrites
/// inside the bracket can legitimately be published with different content by
/// each. That is the sampling-window difference the equality mode exists to
/// measure, and "only delivery can differ" was never true.
///
/// MUTATION: make the walk read `Opening` for all three attempts of the opening
/// sequence and this publishes the non-terminal opening value instead.
/// kills: `(Opening, taken) => Sample(taken)` -> `Opening` in
///   `d81_read_coherent_active_entry_at` -- the OPENING loop then coheres on A
///   while the CLOSING loop still reaches B, so the closure refuses
///   `ClosureChanged` and the `expect` below fails. It does NOT publish an
///   A-valued row: the closure catches it first.
#[test]
fn the_active_window_publishes_the_coherent_value_and_not_the_opening_image() {
    use crate::snapshot_recipes::Span;

    // A province whose OPENING active window is non-terminal: byte 0x14 zero and
    // the flag bit clear, which `active_window_of` gives every multiple of ten.
    const CHANGED: u32 = 10;
    assert!(
        !active_terminal_of(CHANGED),
        "the fixture province must start non-terminal, or the assertion below \
         cannot tell the opening image from the sampled one",
    );

    let world = stage_world(WorldShape::default());
    let window = Span::new(
        ACTIVE_TABLE
            + u64::from(CHANGED) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
            + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64,
        crate::attach_log::D81_ACTIVE_WINDOW_LEN as u64,
    );
    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(window);
    assert_eq!(
        ordinals.len(),
        2 * crate::attach_log::D81_ACTIVE_COHERENCE_READS,
        "the active table must be copied once per observation the coherence loop \
         takes, at the opening AND at the close, or this arm is not placing the \
         write where it says",
    );

    // WRITTEN BETWEEN THE FIRST AND SECOND OPENING OBSERVATIONS, so the copier
    // holds A, B, B -- the sequence the live loop coheres on B for.
    let at = ACTIVE_TABLE
        + u64::from(CHANGED) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
        + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64;
    let mut terminal = active_window_of(CHANGED);
    terminal[2] = 0x5A;
    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(at, terminal.to_vec())],
        ordinals[1],
    );

    assert!(applied, "the fixture must actually have rewritten the window");
    assert!(!counters.consumer_gapped);
    let rows = walked.expect("A, B, B coheres on B and its closure against B agrees");
    let row = rows
        .provinces
        .iter()
        .find(|row| row.province_id == CHANGED)
        .expect("the province publishes");
    assert!(
        row.active_terminal,
        "the row must carry the COHERENT value B, which the loop reached from \
         the copier's adjacent samples -- not the opening image A",
    );
}

/// `active_terminal` as the walk derives it, so the fixture can assert against
/// the same predicate the reader uses rather than against a restatement of it.
fn active_terminal_of(province: u32) -> bool {
    let window = active_window_of(province);
    let state = crate::attach_log::d81_active_state_from_window(window);
    state.active_a != 0 || state.active_b != 0 || state.active_u16 != 0 || state.active_flags & 1 != 0
}

/// THE RAIL HEADER CLOSES AGAINST ITS CLOSING IMAGE, proved rather than pinned.
///
/// The header's stage closes MID-WALK (`CloseWhen::AfterWave(3)`), which is why
/// the other mutation arm in this file uses the map array: a write placed at the
/// first copy of the FINAL closing wave lands long after the header's image was
/// taken. So this arm finds the ordinal of the header's own closing copy — by
/// running the plan once against a memory that logs every transaction — and
/// places the write immediately before it.
///
/// The pair COUNT is rewritten from two neighbours to one. The walk's opening
/// `header()` decodes two from the opening and sampled images; its closing
/// `header()` decodes one from the closing image; and the comparison between
/// them refuses `RailHeader`/`ClosureChanged` carrying the CLOSING count, which
/// is exactly what the live walk publishes for the same write.
///
/// MUTATION: change the closing `header(D81Image::Closing)` to
/// `header(D81Image::Opening)` and the two calls compare one image with itself —
/// the frame publishes a two-neighbour row and this fails. That is the mutation
/// the source-text pin used to stand in for, now killed by behaviour.
/// kills: `match header(D81Image::Closing)` -> `header(D81Image::Opening)` at
///   the walk's closing header site (the two calls would compare one image
///   with itself and publish a two-neighbour row)
/// kills: `closing_program`'s rail-header stage -> `CloseWhen::Final` (the
///   header's closing image would be taken by the FINAL wave, so its ordinal
///   would no longer be below the measured start of that wave and the mid-walk
///   assertion below would fail)
#[test]
fn the_rail_header_closes_against_its_own_closing_image() {
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());

    // WHERE THE HEADER'S CLOSING COPY FALLS. The plan copies the rail-info
    // object from `+0`, so the window the walk consumes lies inside it; the
    // copies covering it are, in order, the opening one, its adjacent sample,
    // and the mid-walk close.
    let header = Span::new(
        rail_info_of(CHANGED) + crate::attach_log::RAIL_INFO_PAIRS_PTR_OFF as u64,
        crate::attach_log::RAIL_HEADER_WINDOW_LEN as u64,
    );
    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    // AND THE CLOSE IS GENUINELY MID-WALK: it happens before the FINAL closing
    // wave, whose first copy the observation reports directly.
    //
    // THE BOUNDARY USED TO BE `total - intervals_closed + 1`, which is not the
    // final wave's head and is LATER than it — the wave takes two images at
    // every paired site, so it is about twice `intervals_closed` copies long.
    // Under that expression the assertion below admitted a header close sitting
    // inside the first half of the final wave, which is precisely the state
    // `CloseWhen::AfterWave` exists to distinguish from a mid-walk one. The
    // measured boundary is what makes this a mid-walk claim.
    let final_wave_starts = final_wave_start(counting.calls(), &closed);
    // AND THE CLOSING TAIL GENUINELY TAKES ITS PAIRS: the slot closings are
    // adjacent pairs and the active closings coherence triples, so the tail
    // is strictly longer than one copy per closing window. A tail that
    // shrank to one image per window is the pre-ruling shape this arm must
    // not silently test.
    {
        use crate::attach_log::d81_vector_store::D81Family;
        let vector = closed.vector.as_ref().expect("the pipeline ran");
        let tail = vector.copies.get().saturating_sub(vector.closing_tail_start);
        let slot_windows = (vector.family_window_count(D81Family::SourceSlotClose)
            + vector.family_window_count(D81Family::MapSlotClose))
            as u64;
        let active_windows = vector.family_window_count(D81Family::ActiveClose) as u64;
        assert!(
            tail >= 2 * slot_windows + active_windows,
            "the closing tail issued {tail} copies against {slot_windows} slot \
             windows and {active_windows} active windows: a closing pair has \
             gone back to a single image",
        );
    }
    drop(closed);
    let ordinals = recorder.ordinals_covering(header);
    assert_eq!(
        ordinals.len(),
        2 * D81_CLOSING_IMAGES_HELD,
        "the header must be copied twice at the opening and twice at the close \
         -- the closing wave takes as many observations as the opening one -- or \
         this arm is not placing the write where it says",
    );
    // The FIRST closing observation: the write lands ahead of the whole closing
    // sequence, so both closing images hold the new count and the walk's closing
    // pair AGREES on it. What refuses is the comparison against the opening.
    let closing_ordinal = ordinals[D81_CLOSING_IMAGES_HELD];

    assert!(
        closing_ordinal < final_wave_starts,
        "the header's close must land inside wave 3 ({closing_ordinal} against a \
         final wave starting at {final_wave_starts}), or `CloseWhen::AfterWave` \
         is not doing what the plan declares",
    );

    let at = rail_info_of(CHANGED) + crate::attach_log::RAIL_INFO_PAIR_COUNT_OFF as u64;
    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(at, 1u32.to_le_bytes().to_vec())],
        closing_ordinal,
    );

    assert!(applied, "the fixture must actually have rewritten the count");
    assert!(
        !counters.consumer_gapped,
        "a change the walk closes over must leave the view intact, or the \
         refusal below is the substrate's and not the reader's",
    );
    let failure = walked.expect_err("a rail header that changed under the copier refuses");
    assert_eq!(failure.stage, D81FailureStage::RailHeader);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(
        failure.raw_value, 1,
        "the refusal must carry the CLOSING count, which is what the live walk \
         puts in `raw`",
    );
}

/// SCENARIO (a): A WRITE BETWEEN THE TWO CLOSING READS REFUSES `Torn`, which is
/// what the live walk does with it.
///
/// The map-province slot is closed by `d81_read_u64_pair_at(.., Closing)`: two
/// adjacent observations, compared. Live, a write landing between them makes the
/// two reads disagree and the pair refuses `Torn`. With one closing image the
/// converted walk compared that image with itself, could not tear, and PUBLISHED
/// — the first of the three scenarios the owner funded closing.
///
/// The write is placed between the slot's two CLOSING observations, so the
/// images are `old, new` exactly as the live reads would have been.
///
/// MUTATION: take `D81_CLOSING_IMAGES_HELD` back to 1, or map
/// `paired_image(Closing)` back to `Closing`, and this frame publishes.
/// kills: `paired_image(Closing)` -> `Closing` (the closing pair would
///   compare one image with itself, never tear, and the frame would publish)
#[test]
fn a_write_between_the_two_closing_reads_refuses_torn_as_the_live_walk_does() {
    use crate::attach_log::d81_copy_bracket::D81_CLOSING_IMAGES_HELD;
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    const REPLACEMENT: u64 = BASE + 0x0999_0000;
    let world = stage_world(WorldShape::default());
    let slot = Span::new(MAP_ARRAY + u64::from(CHANGED) * 8, 8);

    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(slot);
    assert_eq!(ordinals.len(), 2 * D81_CLOSING_IMAGES_HELD);
    // BETWEEN the two closing observations: image 0 holds the old pointer and
    // image 1 the new one.
    let between_closing = ordinals[D81_CLOSING_IMAGES_HELD + 1];

    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(slot.start, REPLACEMENT.to_le_bytes().to_vec())],
        between_closing,
    );
    assert!(applied, "the fixture must actually have rewritten the slot");
    assert!(!counters.consumer_gapped);
    let failure = walked.expect_err("two disagreeing closing observations are a torn read");
    assert_eq!(failure.stage, D81FailureStage::MapProvince);
    assert_eq!(
        failure.reason,
        D81FailureReason::Torn,
        "the live walk refuses `Torn` for the same write, and so must this one",
    );
    assert_eq!(failure.subject_id, CHANGED);
}

/// SCENARIO (b): A CLOSING OBSERVATION THE ENGINE REFUSES IS `Unreadable`, and
/// it is the ENGINE that is named, not the plan.
///
/// Live, the second closing read of a stable pair returning `None` refuses
/// `Unreadable` at that site. With one closing image the converted walk never
/// asked for a second observation and published — the second of the three
/// scenarios.
///
/// The distinction this arm also pins: the plan ASKED for that observation, so
/// `closing_sample_bytes` answers `Unreadable` and not `NotCaptured`, and the
/// refusal is an engine condition rather than a plan defect. A capture that read
/// this as `NotCopied` would be told the conversion is broken when the game went
/// unreadable.
///
/// MUTATION: have `closing_sample_bytes` answer `NotCaptured` for an image its
/// consumers asked for and `plan_defects()` goes non-zero here.
/// kills: the `image < wanted` arm of `closing_sample_bytes` -> always
///   `NotCaptured` (the declined observation would be counted a plan defect
///   and restamped `NotCopied`)
#[test]
fn a_second_closing_read_the_engine_refuses_is_unreadable_and_not_a_plan_defect() {
    use crate::attach_log::d81_copy_bracket::D81_CLOSING_IMAGES_HELD;
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
    let slot = Span::new(MAP_ARRAY + u64::from(CHANGED) * 8, 8);

    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(slot);
    let second_closing = ordinals[D81_CLOSING_IMAGES_HELD + 1];

    // The engine declines exactly that transaction and no other.
    let refusing = RefusingAtMemory::new(&world.memory, second_closing);
    let counting = CountingSnapshotMemory::new(&refusing);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    assert!(refusing.refused(), "the fixture must actually have refused a copy");
    let source = closed
        .source_for(WORLD_HOUR)
        .expect("one declined closing observation must not gap the consumer");
    let phases = D81PhaseAccum::default();
    let rows = walk_with(&world, &source, &phases);
    counters.misses = source.misses();

    let failure = rows.expect_err("a closing observation the engine refused refuses the frame");
    assert_eq!(failure.stage, D81FailureStage::MapProvince);
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert!(
        counters.misses.unreadable > 0,
        "the miss must be classed as the ENGINE refusing: {:?}",
        counters.misses,
    );
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "the plan asked for that observation, so its absence is not a plan \
         defect: {:?}",
        counters.misses,
    );
    assert_eq!(
        d81_restamp_plan_defect(failure, counters.misses).reason,
        D81FailureReason::Unreadable,
        "and it must survive the restamp as an engine condition",
    );
}

/// SCENARIO (c): AN ACTIVE CLOSING SEQUENCE OF `A, B, B` COHERES ON `B` AND
/// REFUSES THE CLOSURE, which is the live verdict for the same sequence.
///
/// `d81_read_coherent_active_entry` publishes the first CONSECUTIVE PAIR that
/// agrees. Live, a closing sequence `A, B, B` coheres on `B`, and
/// `d81_admit_active_closure` then refuses because the opening was `A`. With one
/// closing image the converted walk saw `A, A`, admitted the closure and
/// PUBLISHED — the third of the three scenarios.
///
/// MUTATION: give `ACTIVE_TABLE` two samples instead of
/// `D81_ACTIVE_COHERENCE_READS`, and the third closing observation is not there
/// to cohere with.
/// kills: `(Closing, taken) => ClosingSample(taken)` removed from
///   `d81_read_coherent_active_entry_at` -- the closing loop then repeats image
///   0, coheres on A, admits the closure, and the frame PUBLISHES where this
///   expects a refusal. (Lowering `ACTIVE_TABLE`s sample count from
///   `D81_ACTIVE_COHERENCE_READS` to 2 also fails this test, but STRUCTURALLY at
///   the `ordinals.len() == 2 * reads` precondition rather than on the scenario
///   -- a different claim, and named as one.)
#[test]
fn an_active_closing_sequence_of_a_b_b_coheres_and_refuses_like_the_live_walk() {
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 11;
    let reads = crate::attach_log::D81_ACTIVE_COHERENCE_READS;
    let world = stage_world(WorldShape::default());
    let window = Span::new(
        ACTIVE_TABLE
            + u64::from(CHANGED) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
            + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64,
        crate::attach_log::D81_ACTIVE_WINDOW_LEN as u64,
    );

    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(window);
    assert_eq!(ordinals.len(), 2 * reads);
    // Between the FIRST and SECOND closing observations, so the closing sequence
    // is `A, B, B` -- the one the live loop coheres on `B` for.
    let between_closing = ordinals[reads + 1];

    let mut changed = active_window_of(CHANGED);
    changed[2] ^= 0x5A;
    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(window.start, changed.to_vec())],
        between_closing,
    );
    assert!(applied, "the fixture must actually have rewritten the window");
    assert!(!counters.consumer_gapped);
    let failure = walked
        .expect_err("the closing sequence coheres on B and the closure against A refuses");
    assert_eq!(failure.stage, D81FailureStage::ActiveState);
    assert_eq!(failure.subject_id, CHANGED);
    // THE REASON, not just the stage (Codex 019fedc5, round 6, finding 2). A
    // third attempt mapped back onto image 1 would give `A, B, A`, which
    // exhausts the loop and refuses `Torn` — a different verdict at the same
    // stage, and asserting only the stage would pass for it.
    assert_eq!(
        failure.reason,
        D81FailureReason::ClosureChanged,
        "the loop must COHERE on B and refuse the closure against opening A",
    );
}

/// THE THIRD CLOSING OBSERVATION IS ITS OWN IMAGE, proved with three DISTINCT
/// values.
///
/// The `A, B, B` arm above cannot prove this (Codex 019fedc5, round 6, finding
/// 2): one write makes the second and third images equal, so a third attempt
/// that re-read image 1 would produce the same sequence and pass. Three distinct
/// values separate them — `A, B, C` has no agreeing consecutive pair, so the
/// loop exhausts and refuses `Torn`, while a third attempt mapped back onto
/// image 1 would see `A, B, B`, cohere, and refuse `ClosureChanged` instead.
///
/// MUTATION: map the coherence loop's third closing attempt to
/// `ClosingSample(1)` and this refuses `ClosureChanged` rather than `Torn`.
/// kills: `(Closing, taken) => ClosingSample(taken)` -> `ClosingSample(1)`
///   (A, B, C would read as A, B, B, cohere on B and refuse `ClosureChanged`
///   instead of `Torn`)
#[test]
fn the_third_closing_observation_is_its_own_image_and_not_a_repeat() {
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 11;
    let reads = crate::attach_log::D81_ACTIVE_COHERENCE_READS;
    let world = stage_world(WorldShape::default());
    let window = Span::new(
        ACTIVE_TABLE
            + u64::from(CHANGED) * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
            + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64,
        crate::attach_log::D81_ACTIVE_WINDOW_LEN as u64,
    );

    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(window);
    assert_eq!(ordinals.len(), 2 * reads);

    // A, B, C across the three CLOSING observations.
    let mut b = active_window_of(CHANGED);
    b[2] ^= 0x5A;
    let mut third = active_window_of(CHANGED);
    third[2] ^= 0x5A;
    third[3] ^= 0x33;
    let script = vec![
        (ordinals[reads + 1], window.start, b.to_vec()),
        (ordinals[reads + 2], window.start, third.to_vec()),
    ];

    let StagedWorld {
        memory,
        topology,
        valid_ids,
        container,
        roots,
    } = stage_world(WorldShape::default());
    let scripted = ScriptedMemory::new(memory, script);
    let counting = CountingSnapshotMemory::new(&scripted);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    assert_eq!(scripted.applied(), 2, "both writes must land");
    let source = closed
        .source_for(WORLD_HOUR)
        .expect("a view");
    let phases = D81PhaseAccum::default();
    let world = StagedWorld {
        memory: SyntheticMemory::new(),
        topology,
        valid_ids,
        container,
        roots,
    };
    let failure = walk_with(&world, &source, &phases)
        .expect_err("three disagreeing observations exhaust the loop");
    assert_eq!(failure.stage, D81FailureStage::ActiveState);
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(
        failure.reason,
        D81FailureReason::Torn,
        "no consecutive pair agrees, so the loop must exhaust -- a third attempt \
         re-reading image 1 would have cohered and refused `ClosureChanged`",
    );
}

/// A RAIL HEADER WHOSE TWO CLOSING OBSERVATIONS DISAGREE REFUSES `Torn`, not
/// `Unreadable`.
///
/// The header's `header()` falls back to scalar reads when its two window
/// observations disagree, because live a 16-byte range straddling two regions
/// refuses where the scalars succeed. A COPY that holds the window has already
/// answered that question, so the fallback there would re-read the same two
/// images and report `Unreadable` for bytes that were read perfectly well
/// (Codex 019fedc5, round 6, finding 1).
///
/// Live refuses `ClosureChanged` for this write, because its scalars observe at
/// a third instant the copy does not hold — `MAX_ADJACENT_SAMPLES` is three and
/// this site would need six. Both arms REFUSE; `Torn` is the honest reason for
/// two disagreeing observations, and `Unreadable` was not.
///
/// MUTATION: send the snapshot arm back through `scalars()` and this refuses
/// `Unreadable`.
/// kills: the `source.is_snapshot()` arm on the header's DISAGREEMENT path
///   (the fallback would re-read the same two images and refuse `Unreadable`)
#[test]
fn a_rail_header_whose_two_closing_images_disagree_refuses_torn() {
    use crate::attach_log::d81_copy_bracket::D81_CLOSING_IMAGES_HELD;
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
    let header = Span::new(
        rail_info_of(CHANGED) + crate::attach_log::RAIL_INFO_PAIRS_PTR_OFF as u64,
        crate::attach_log::RAIL_HEADER_WINDOW_LEN as u64,
    );
    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(header);
    assert_eq!(ordinals.len(), 2 * D81_CLOSING_IMAGES_HELD);
    // BETWEEN the two closing observations, so they disagree.
    let between_closing = ordinals[D81_CLOSING_IMAGES_HELD + 1];

    let at = rail_info_of(CHANGED) + crate::attach_log::RAIL_INFO_PAIR_COUNT_OFF as u64;
    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(at, 1u32.to_le_bytes().to_vec())],
        between_closing,
    );
    assert!(applied);
    assert!(!counters.consumer_gapped);
    let failure = walked.expect_err("two disagreeing closing observations refuse");
    assert_eq!(failure.stage, D81FailureStage::RailHeader);
    assert_eq!(
        failure.reason,
        D81FailureReason::Torn,
        "a header whose two closing observations disagree is a TORN read; \
         `Unreadable` would blame the engine for bytes it served",
    );
    assert_eq!(
        counters.misses.total(),
        0,
        "and nothing failed to read: {:?}",
        counters.misses,
    );
}

/// A RAIL HEADER WHOSE SECOND CLOSING OBSERVATION THE ENGINE DECLINED REFUSES
/// FROM THE WINDOW READ, not by re-reading it through the scalar fallback.
///
/// The fallback is a LIVE-only device: it exists because a 16-byte range
/// straddling two regions refuses where each scalar succeeds, and because a
/// transient can be recovered from by observing again. On the copy it would
/// re-read the very image that was unavailable and arrive at the same refusal by
/// a longer route (Codex 019fedf3, round 7).
///
/// WHAT LIVE CAN DO THAT THIS CANNOT, and BOTH ARMS ARE RUN HERE so the claim is
/// tested rather than asserted (Codex 019fee1c, round 8): a direct walk whose
/// second window observation is declined recovers through the four later scalar
/// observations and PUBLISHES, while the copy — which asked for two observations
/// and got one — cannot render the pair check and refuses the hour. Closing that
/// needs six observations of this object against a ceiling of three.
///
/// MUTATION: send the snapshot arm back through `scalars()` on a read failure
/// and the refusal comes from the scalar path instead -- with an `index` the
/// window read never set.
/// kills: the `source.is_snapshot()` arm on the header's READ-FAILURE path
///   (the fallback would re-read the unavailable image and refuse from the
///   scalar path, with index 0 rather than 1)
#[test]
fn a_rail_header_missing_its_second_closing_observation_refuses_from_the_window_read() {
    use crate::attach_log::d81_copy_bracket::D81_CLOSING_IMAGES_HELD;
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
    let header = Span::new(
        rail_info_of(CHANGED) + crate::attach_log::RAIL_INFO_PAIRS_PTR_OFF as u64,
        crate::attach_log::RAIL_HEADER_WINDOW_LEN as u64,
    );
    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(header);
    assert_eq!(ordinals.len(), 2 * D81_CLOSING_IMAGES_HELD);
    let second_closing = ordinals[D81_CLOSING_IMAGES_HELD + 1];

    let refusing = RefusingAtMemory::new(&world.memory, second_closing);
    let counting = CountingSnapshotMemory::new(&refusing);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    assert!(refusing.refused(), "the fixture must actually decline a copy");
    let source = closed
        .source_for(WORLD_HOUR)
        .expect("one declined observation must not gap the consumer");
    let phases = D81PhaseAccum::default();
    let failure = walk_with(&world, &source, &phases)
        .expect_err("a header whose pair cannot be rendered refuses the hour");
    counters.misses = source.misses();

    assert_eq!(failure.stage, D81FailureStage::RailHeader);
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(
        failure.index, 1,
        "the FIRST observation succeeded and the second did not, and the \
         refusal says which",
    );
    assert!(
        counters.misses.unreadable > 0,
        "the engine declined an observation the plan asked for: {:?}",
        counters.misses,
    );
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "the plan asked for it, so its absence is not a plan defect: {:?}",
        counters.misses,
    );

    // THE LIVE ARM, over the same failure. A DIRECT walk whose SECOND window
    // observation is declined falls back to the four per-field reads, which
    // succeed on a stable header, and publishes. That is the recovery the copy
    // cannot perform, and without running it this test would pass just as well
    // if the direct arm had stopped recovering too.
    let direct_world = stage_world(WorldShape::default());
    let declining = RefusingNthAtMemory::new(
        &direct_world.memory,
        header.start,
        2,
    );
    let phases = D81PhaseAccum::default();
    let rows = walk_with(
        &direct_world,
        &D81ReadSource::Staged(&declining),
        &phases,
    )
    .expect("the live arm recovers through its scalar reads and publishes");
    assert!(declining.refused(), "the fixture must decline the second window read");
    assert_eq!(
        rows.provinces.len(),
        PUBLISHED_ROWS,
        "and it publishes every row, which is the hour the copy loses",
    );
}

/// THE ANCHOR WINDOW IS THE SECOND STRICTER SITE, and it behaves like the first.
///
/// `read_anchor_triplet` falls back to three per-axis reads when its 12-byte
/// window observation fails — live, because a 12-byte range straddling two
/// regions refuses where three 4-byte ranges succeed, and because a transient can
/// be recovered from by observing again. On the copy that fallback would re-read
/// the very image that was unavailable (Codex 019fee1c, round 8), so the snapshot
/// arm refuses from the window read with axis 0's own witness.
///
/// Both arms are run: the copy refuses `Anchor`/`Unreadable`, the direct walk
/// recovers through its three per-axis reads and publishes. That is the same
/// stricter-than-live trade the rail header makes, and the receipt now lists
/// BOTH sites rather than claiming the header is the only one.
///
/// MUTATION: send the snapshot arm back through `scalars()` and the refusal comes
/// from the per-axis path instead.
/// kills: the `source.is_snapshot()` arm in `read_anchor_triplet` (the
///   per-axis fallback would re-read the unavailable image on the copy)
#[test]
fn the_anchor_window_refuses_from_the_window_read_where_the_live_arm_recovers() {
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
    // Province 7's anchor: `secondary(7)` is false, so it is the PRIMARY triplet.
    let anchor = Span::new(
        map_province_of(CHANGED) + crate::attach_log::D81_ANCHOR_PRIMARY_OFF as u64,
        12,
    );

    let recorder = RecordingMemory::new(&world.memory);
    let counting = CountingSnapshotMemory::new(&recorder);
    let mut probe = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut probe)
        .expect("custody");
    drop(closed);
    let ordinals = recorder.ordinals_covering(anchor);
    assert!(
        ordinals.len() >= 2,
        "the anchor window must be observed more than once",
    );
    // The SECOND opening observation: the walk's A/B pass needs it and the copy
    // will not have it.
    let second_opening = ordinals[1];

    let refusing = RefusingAtMemory::new(&world.memory, second_opening);
    let counting = CountingSnapshotMemory::new(&refusing);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    assert!(refusing.refused());
    let source = closed
        .source_for(WORLD_HOUR)
        .expect("one declined observation must not gap the consumer");
    let phases = D81PhaseAccum::default();
    let failure = walk_with(&world, &source, &phases)
        .expect_err("the copy cannot render the anchor pass and refuses");
    counters.misses = source.misses();
    assert_eq!(failure.stage, D81FailureStage::Anchor);
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(failure.subject_id, CHANGED);
    // AN OPENING OBSERVATION THE ENGINE DECLINED is `samples_absent`, which is
    // the opening-side twin of the closing side's `Unreadable`: the plan asked
    // for the adjacent image and did not get it. Both are ENGINE conditions and
    // neither is a plan defect -- what differs is only which counter names it,
    // because the substrate records an untaken opening sample by omitting it
    // while an untaken closing one is distinguishable from one nobody asked for.
    // EXACTLY ONE (Codex 019fee4b, round 9). `> 0` does not kill this test's own
    // named reversal: without the snapshot guard the per-axis fallback runs and
    // re-reads the same unavailable image, which produces the SAME
    // `Anchor`/`Unreadable` at axis 0 and merely counts the miss again. The
    // count is what tells the two apart -- one declined observation, or one
    // declined observation and a fallback that asked for it a second time.
    assert_eq!(
        counters.misses.samples_absent, 1,
        "the walk must ask for the declined observation ONCE and refuse; a \
         fallback that re-reads it counts a second miss: {:?}",
        counters.misses,
    );
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "and that is not a plan defect: {:?}",
        counters.misses,
    );

    // AND THE LIVE ARM RECOVERS, which is what makes this a stricter site rather
    // than a broken one.
    let direct_world = stage_world(WorldShape::default());
    let declining = RefusingNthAtMemory::new(&direct_world.memory, anchor.start, 2);
    let phases = D81PhaseAccum::default();
    let rows = walk_with(
        &direct_world,
        &D81ReadSource::Staged(&declining),
        &phases,
    )
    .expect("the live arm recovers through its per-axis reads and publishes");
    assert!(declining.refused());
    assert_eq!(rows.provinces.len(), PUBLISHED_ROWS);
}

/// THE PLAN'S OWN PRICE IS MEASURED, and `d81_plan_us` is not a zero the wire
/// carries on every frame.
///
/// `D81PlanCounters::plan_us` names itself "THE NUMBER THE CONVERSION IS ABOUT"
/// and `d81_plan_units` publishes it as `d81_plan_us`. Between the conversion
/// landing and lane GJ, NO WRITER EXISTED: the field was default-constructed
/// and published untouched, so BOTH captures that carry the word — t48 and t49
/// — read `d81_plan_us` = 0 across the whole capture while `d81_phase_plan_us`
/// on the same records read 59.2 and 67.0 ms per observation. The wire-shape
/// test that "covered" it supplied its own 4,012 and so could not see this.
///
/// kills: `counters.plan_us = d81_elapsed_us(started)` removed from
/// `d81_observe_copy_plan_with`'s success path — the field falls back to
/// `Default::default()`'s zero and this assertion fails, which is exactly the
/// state t48 and t49 shipped in.
///
/// WHY NON-ZERO IS A REAL BOUND HERE and not a race with a fast clock: this
/// fixture's observation issues thousands of guarded copies through
/// `CountingSnapshotMemory`, asserted below against the same counter, and this
/// same fixture was MEASURED at 6,985-8,130 us over six runs while cutting the
/// tolerance in the test below. A whole
/// observation cannot land inside one microsecond.
#[test]
fn the_plans_own_price_is_measured_rather_than_left_at_its_default_zero() {
    let world = stage_world(WorldShape::default());
    let counting = CountingSnapshotMemory::new(&world.memory);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    drop(closed);
    assert!(
        counters.copies_issued > 1_000,
        "the fixture must be big enough for the bound below to be a bound; it issued {}",
        counters.copies_issued,
    );
    assert!(
        counters.plan_us > 0,
        "the plan published a zero price for {} guarded copies; `plan_us` has no writer",
        counters.copies_issued,
    );
}

/// `d81_plan_us` names the WHOLE observation and not a slice of it, so a reader
/// pricing the conversion from the plan family is not reading a fragment of what
/// the phase family carries.
///
/// WHAT THIS PINS, AND WHAT IT DOES NOT — stated because the two are easy to
/// confuse. It brackets `d81_observe_copy_plan_with` from OUTSIDE with the
/// production accumulator (`D81PhaseAccum::plan_ns`, closed exactly as the call
/// site closes it) and requires the word written INSIDE to account for at least
/// FOUR FIFTHS of that window. It therefore pins a LOWER BOUND on the inner
/// window's extent, not the extent itself.
///
/// It does NOT pin the production call site's own boundary: that `plan_mark`
/// lives in `read_compact_rail_replay_state`, which needs a live engine, so
/// moving it — to include the hub read, say — is invisible here and is checked
/// by reading. Nor does it establish that the two words come out EQUAL: it
/// proves ordering and a lower bound, and no capture has been taken since the
/// writer was added, so what a manifest will show is unobserved. What the
/// production boundary is, and how the two windows relate, is written at the
/// assignment itself.
///
/// kills: `plan_us` moved to cover the RECIPE BUILD alone, or `close_witnesses`
/// alone. Both leave the word non-zero — so the zero-check above still passes —
/// and this test refuses them, on a measured share rather than an argued one.
/// IT DOES NOT REFUSE A TIMER AROUND `observe` ALONE, and that is stated rather
/// than glossed: `observe` measures about 95% of the window on this fixture, so
/// it clears the bound. The ~5% it omits is ~350-400 us here and `plan_us` has
/// microsecond resolution, so that omission WOULD reach the wire — this test
/// simply does not bound it.
///
/// The two kills are MEASURED here rather than argued from transaction shares.
/// An earlier revision justified its threshold by saying the closing wave was
/// one image of four; it is two of four, and in any case a transaction share is
/// not a wall share — which is the same mistake `runs/gj_crit_doubling` spends
/// several pages telling other people not to make.
#[test]
fn the_plan_price_and_the_plan_phase_timer_measure_the_same_window() {
    let world = stage_world(WorldShape::default());
    let counting = CountingSnapshotMemory::new(&world.memory);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let phases = D81PhaseAccum::default();
    let mark = Instant::now();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    phases.close(&phases.plan_ns, mark);
    drop(closed);
    let phase_us = phases.snapshot().plan_us;
    assert!(
        phase_us >= counters.plan_us,
        "the phase window encloses the plan's own, so it cannot read shorter: \
         phase {phase_us} us against plan {} us",
        counters.plan_us,
    );
    // A FRACTION AND NOT A MICROSECOND GAP, chosen from measurement rather than
    // taste. Six runs of this fixture put the observation at 6,985-8,130 us and
    // the gap between the two words at 1-2 us, so an absolute tolerance would be
    // a bet on the scheduler: any preemption between the outer and inner
    // `Instant::now()` inflates the OUTER word alone, with no production defect
    // behind it, and a 50 us bound is one quantum away from a flake.
    //
    // Against a measured ratio of ~0.9997, four fifths needs about 1.9 ms of
    // stall on this fixture to flake. What it REFUSES is measured below, on this
    // same fixture, rather than inferred from how the transactions divide.
    assert!(
        counters.plan_us * 5 >= phase_us * 4,
        "the plan's own word must account for the enclosing window, not a slice \
         of it: phase {phase_us} us against plan {} us",
        counters.plan_us,
    );

    // THE CANDIDATE WRONG WINDOWS, RUN AND TIMED. The same calls
    // `d81_observe_copy_plan_with` makes, in its order, on this fixture, so that
    // "the threshold refuses that slice" is an observation and not an argument
    // about transaction counts.
    //
    // MEDIAN OF N, AND NOT THE MINIMUM. A single pass would be a worse bet than
    // the absolute tolerance this test started with: a scheduler stall landing
    // inside the recipe build inflates the NUMERATOR and drags the ratio toward
    // one, failing with no code change behind it (Codex round 9).
    //
    // But the obvious repair — take the smallest share of N — is worse than it
    // looks, and Codex round 10 caught it: a stall OUTSIDE the candidate
    // inflates only the denominator and pushes that pass's share DOWN, so
    // minimising actively SELECTS the contaminated pass and would let a genuinely
    // oversized candidate through on one bad iteration. The median is the
    // estimator that resists contamination on both sides, and at five passes it
    // survives two stalls in either direction.
    //
    // THE MARGINS, MEASURED over four runs of this fixture:
    //     recipe build      0.060% - 0.080% of the observation
    //     close_witnesses   4.68%  - 5.25%
    // against a bound of 80%. That is ~1000x of headroom for the first and ~15x
    // for the second, so what these assert is the SHAPE of the observation, not
    // a timing target.
    //
    // WHICH ALSO SIZES THE ONE CANDIDATE THIS TEST DOES NOT REFUSE: `observe`
    // alone is the remaining ~95%, comfortably past four fifths, so a `plan_us`
    // around it would pass here. That is a deliberate limit and not a claim that
    // the omission is invisible — 5% of a 7.5 ms observation is ~350-400 us and
    // `plan_us` has microsecond resolution, so the wire WOULD show it. What this
    // test pins is a lower bound of four fifths, not the exact window.
    use crate::snapshot_recipes::executor::{observe, ObservationConfig};
    const PASSES: usize = 5;
    // Ratios, not raw spans: comparing nanoseconds across passes would rank by
    // how fast the box was on each pass rather than by how much of the
    // observation the candidate occupied.
    let mut build_shares: Vec<f64> = Vec::with_capacity(PASSES);
    let mut close_shares: Vec<f64> = Vec::with_capacity(PASSES);
    for _ in 0..PASSES {
        let probe = CountingSnapshotMemory::new(&world.memory);
        let whole = Instant::now();
        let build_mark = Instant::now();
        let plan = domain81_copy_plan(&world.roots);
        let build_ns = build_mark.elapsed().as_nanos() as f64;
        let built = observe(&[plan], &probe, ObservationConfig::new(WORLD_HOUR)).expect("custody");
        let close_mark = Instant::now();
        let reclosed = built.close_witnesses();
        let close_ns = close_mark.elapsed().as_nanos() as f64;
        let whole_ns = whole.elapsed().as_nanos() as f64;
        drop(reclosed);
        assert!(whole_ns > 0.0, "the observation must take measurable time");
        build_shares.push(build_ns / whole_ns);
        close_shares.push(close_ns / whole_ns);
    }
    let median = |mut shares: Vec<f64>| -> f64 {
        shares.sort_by(|a, b| a.partial_cmp(b).expect("no NaN: every span is finite"));
        shares[shares.len() / 2]
    };
    for (name, share) in [
        ("recipe build", median(build_shares)),
        ("close_witnesses", median(close_shares)),
    ] {
        assert!(
            share < 0.8,
            "across {PASSES} passes a `plan_us` wrapping only the {name} would \
             read a median {share} of the observation, which the four-fifths \
             bound must refuse but does not",
        );
    }
}
