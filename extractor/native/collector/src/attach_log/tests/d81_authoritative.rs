use super::*;

use crate::attach_log::d81_copy_bracket::{
    domain81_copy_plan, CountingSnapshotMemory, D81ReadSource, Domain81PlanRoots,
    D81_CLOSING_IMAGES_HELD, D81_CONSUMER,
};
use crate::snapshot_recipes::synthetic::SyntheticMemory;
use crate::snapshot_recipes::HourStamp;

pub(super) const PROVINCE_SLOTS: u32 = 385;
const PROVINCES: u32 = PROVINCE_SLOTS - 1;

const BASE: u64 = 0x0000_0200_0000_0000;

pub(super) const GAMESTATE: u64 = BASE;
const IDLER_SLOT: u64 = BASE + 0x0001_0000;
const IDLER_SLOT_B: u64 = BASE + 0x0001_8000;
pub(super) const IDLER: u64 = BASE + 0x0002_0000;
pub(super) const OWNER: u64 = BASE + 0x0003_0000;
const CONTAINER: u64 = BASE + 0x0004_0000;
const RAIL_MANAGER: u64 = BASE + 0x0005_0000;
pub(super) const RAIL_TABLE: u64 = BASE + 0x0006_0000;
const ACTIVE_ROOT: u64 = BASE + 0x0010_0000;
pub(super) const ACTIVE_TABLE: u64 = BASE + 0x0011_0000;
const SELECTOR_MAP: u64 = BASE + 0x0020_0000;
pub(super) const SELECTOR_TABLE: u64 = BASE + 0x0021_0000;
pub(super) const SOURCE_ARRAY: u64 = BASE + 0x0030_0000;
const MAP_ARRAY: u64 = BASE + 0x0031_0000;

const RAIL_INFO: u64 = BASE + 0x1000_0000;
const PAIR_BLOB: u64 = BASE + 0x2000_0000;
const MAP_PROVINCE: u64 = BASE + 0x3000_0000;
const RECORD: u64 = BASE + 0x4000_0000;
const SOURCE_PROVINCE: u64 = BASE + 0x5000_0000;
const SCATTER_STRIDE: u64 = 0x0002_0000;

pub(super) const CPROVINCE_VTABLE: u64 = BASE + 0x0F00_0000;

pub(super) const SELECTOR_MASK: u32 = 511;
pub(super) const SELECTOR_MAX_DISTANCE: u8 = 2;
const SELECTOR_SLOTS: u32 = SELECTOR_MASK + 1 + SELECTOR_MAX_DISTANCE as u32 + 1;

const PAGE: u64 = crate::snapshot_recipes::SCOPED_VOID_GRANULARITY_BYTES;

const RAILLESS_TRIPLE: [u32; 3] = [382, 383, 384];

const PUBLISHED_ROWS: usize = PROVINCES as usize - RAILLESS_TRIPLE.len();

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

    pub(super) fn sized(province_slots: u32) -> Layout {
        assert!(
            province_slots % 3 == 1 && province_slots >= 7,
            "the topology names 1..slots in reciprocal triples, and the top \
             triple is the railless one",
        );
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

    fn selector_slots(&self) -> u32 {
        self.selector_mask + 1 + SELECTOR_MAX_DISTANCE as u32 + 1
    }

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
pub(super) fn map_province_of(province: u32) -> u64 {
    Layout::DEFAULT.map_province_of(province)
}
fn record_of(province: u32) -> u64 {
    Layout::DEFAULT.record_of(province)
}
fn source_province_of(province: u32) -> u64 {
    Layout::DEFAULT.source_province_of(province)
}

fn triple_of(province: u32) -> [u32; 3] {
    let base = province - (province - 1) % 3;
    [base, base + 1, base + 2]
}

fn neighbours_of(province: u32) -> [(u32, u32); 2] {
    let index = (province - 1) % 3;
    let base = province - index;
    [(base + (index + 1) % 3, 1), (base + (index + 2) % 3, 2)]
}

fn anchor_of(province: u32, secondary: bool) -> [u32; 3] {
    let bias = if secondary { 0x1000 } else { 0 };
    [
        province + bias,
        province + 100 + bias,
        province + 200 + bias,
    ]
}

fn selector_slot_of(key: u32, mask: u32) -> u32 {
    let mut hash = (key ^ (key >> 16)).wrapping_mul(0x045d_9f3b);
    hash = (hash ^ (hash >> 16)).wrapping_mul(0x045d_9f3b);
    hash ^= hash >> 16;
    hash & mask
}

fn secondary(province: u32) -> bool {
    province % 2 == 0
}

fn active_window_of(province: u32) -> [u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN] {
    let mut window = [0u8; crate::attach_log::D81_ACTIVE_WINDOW_LEN];
    window[0] = 0;
    window[1] = 0;
    window[2] = if province % 5 == 0 { 0 } else { 0x5A };
    window[3] = 0;
    window[4] = 0;
    window[5] = 0;
    window[6] = (province % 2) as u8;
    window
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WorldShape {
    pub(super) unreadable_rail_info: Option<u32>,
    pub(super) skipped_triple: Option<u32>,
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
    memory.write_u32(
        gamestate + CGAMESTATE_CPROVINCE_COUNT as u64,
        province_slots,
    );
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
    memory.write_u64(
        selector_map + D81_SELECTOR_MAP_TABLE_OFF as u64,
        selector_table,
    );
    memory.write_u32(
        selector_map + D81_SELECTOR_MAP_MASK_OFF as u64,
        selector_mask,
    );
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

    for slot in 0..selector_slots + 8 {
        let entry = selector_table + u64::from(slot) * D81_SELECTOR_ENTRY_STRIDE as u64;
        memory.write(
            entry + D81_SELECTOR_MARKER_OFF as u64,
            &[SELECTOR_MAX_DISTANCE + 1],
        );
        memory.write_u32(entry + D81_SELECTOR_KEY_OFF as u64, u32::MAX);
    }
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
        memory.write_u32(
            source + crate::attach_log::CPROVINCE_OFF_ID as u64,
            province,
        );
        memory.write_u64(source_array + index * 8, source);

        let map = layout.map_province_of(province);
        memory.map_zeroed(map, D81_ANCHOR_SECONDARY_OFF + 12 + 0x20);
        memory.write_u64(map_array + index * 8, map);
        let record = layout.record_of(province);
        memory.map_zeroed(record, D81_RECORD_SELECTOR_KEY_OFF + 0x10);
        memory.write_u64(map + D81_MAP_PROVINCE_RECORD_OFF as u64, record);
        memory.write_u32(record + D81_RECORD_SELECTOR_KEY_OFF as u64, province);
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

        if province == 0 || railless_triple.contains(&province) {
            memory.write_u64(rail_table + index * 8, 0);
            continue;
        }
        let info = layout.rail_info_of(province);
        memory.map_zeroed(
            info,
            RAIL_INFO_PAIRS_PTR_OFF + RAIL_HEADER_WINDOW_LEN + 0x20,
        );
        let blob = layout.pair_blob_of(province);
        memory.map_zeroed(blob, 0x40);
        for (slot, (neighbour, level)) in neighbours_of(province).iter().enumerate() {
            memory.write_u32(blob + slot as u64 * 8, *neighbour);
            memory.write_u32(blob + slot as u64 * 8 + 4, *level);
        }
        memory.write_u64(info + RAIL_INFO_PAIRS_PTR_OFF as u64, blob);
        memory.write_u32(info + RAIL_INFO_PAIR_COUNT_OFF as u64, 2);
        memory.write_u32(
            info + RAIL_INFO_PAIRS_PTR_OFF as u64 + 8,
            0xDEAD_0000 + province,
        );

        let pointer = if shape.rejected_rail_pointer == Some(province) {
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

pub(super) const WORLD_HOUR: HourStamp = HourStamp(1939_09_01);

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
        chain_snapshot: crate::attach_log::D81MapProvinceChain {
            idler: IDLER as usize,
            owner: OWNER as usize,
            container,
        },
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

pub(super) const STAGED_DEFERRED_ROWS: usize = PUBLISHED_ROWS;

fn direct_walk(world: &StagedWorld) -> Result<D81WalkRows, D81FirstFailure> {
    let phases = D81PhaseAccum::default();
    walk_with(world, &D81ReadSource::Staged(&world.memory), &phases)
}

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

struct ScriptedMemory<'a> {
    inner: std::cell::RefCell<SyntheticMemory>,
    script: Vec<(u64, u64, Vec<u8>)>,
    seen: std::cell::Cell<u64>,
    applied: std::cell::Cell<usize>,
    _life: std::marker::PhantomData<&'a ()>,
}

impl ScriptedMemory<'_> {
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

pub(super) fn walk_with(
    world: &StagedWorld,
    source: &D81ReadSource<'_>,
    phases: &D81PhaseAccum,
) -> Result<D81WalkRows, D81FirstFailure> {
    walk_with_layout(world, &Layout::DEFAULT, source, phases)
}

/// [`walk_with`] against a world staged at some OTHER layout.
///
/// `walk_with` reads the default layout's table addresses out of module
/// constants, so pointing it at a `Layout::sized` world walks tables that world
/// never staged and refuses at its first province. Every profile that varies
/// the province count therefore has to go through here.
pub(super) fn walk_with_layout(
    world: &StagedWorld,
    layout: &Layout,
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
        layout.rail_table as usize,
        layout.active_table as usize,
        layout.selector_table as usize,
        layout.selector_mask,
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
    assert_eq!(
        counters.misses.samples_absent, 0,
        "every `_stable` pair and every A/B pass must have a second image to \
         compare against: {:?}",
        counters.misses,
    );
}

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

    let failure = walked.expect_err("the two images disagree and the walk says so");
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(
        failure.raw_value, REPLACEMENT,
        "the CLOSING value must be the one the refusal carries, or the closing \
         read took the opening image",
    );

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
    assert!(counters.misses.unreadable > 0);
    assert_eq!(
        counters.misses.plan_defects(),
        0,
        "a void is the engine refusing, never the plan under-asking: {:?}",
        counters.misses,
    );
}

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
    assert_eq!(restamped.stage, D81FailureStage::RailHeader);
}

#[test]
fn a_view_of_the_copy_cannot_be_taken_at_another_hour() {
    let world = stage_world(WorldShape::default());
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody must admit a fixture this size");
    assert!(closed
        .closed
        .snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .is_ok());
    assert!(
        closed
            .closed
            .snapshot
            .view_for(D81_CONSUMER, HourStamp(WORLD_HOUR.0 + 1))
            .is_err(),
        "a copy taken in one hour must refuse to answer for the next",
    );
}

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
    assert_eq!(
        diverging.first_divergence_province,
        rows.provinces[3].province_id
    );
    assert_eq!(
        crate::attach_log::d81_copy_bracket::d81_shadow_divergence_unit_name(
            diverging.first_divergence_field,
        ),
        "d81_shadow_divergence_05_anchor_bits",
    );

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

#[test]
fn the_shipped_default_is_the_copy_plan_and_the_switch_gates_the_reader() {
    assert!(
        crate::attach_log::d81_copy_bracket::D81_COPY_BRACKET_AUTHORITATIVE,
        "the recipe program is d81's production reader",
    );
    let source = collector_source_lf();
    assert!(
        source
            .contains("let plan_snapshot = if d81_copy_bracket::D81_COPY_BRACKET_AUTHORITATIVE {"),
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
    let finish = source
        .split_once("fn d81_finish_rail_replay_state(")
        .expect("the post-bracket half")
        .1
        .split_once("\nfn ")
        .expect("the end of the post-bracket half")
        .0;
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
    for closing_site in [
        ".u64_stable_at(record_address, d81_copy_bracket::D81Image::Closing)",
        "            d81_copy_bracket::D81Image::Closing,\n        ) {",
    ] {
        assert!(
            source.contains(closing_site),
            "a closing re-read must take the CLOSING image: {closing_site}",
        );
    }
    let plan_source = include_str!("../d81_copy_bracket.rs").replace("\r\n", "\n");
    let plan_source = plan_source.as_str();
    assert!(
        source.contains("let rows = d81_copied_roots_agree("),
        "the compute half must ask the copy which tables it followed before it \
         walks them",
    );
    assert!(
        plan_source.contains(
            "Err(crate::snapshot_recipes::view::ViewRefusal::NotCaptured { .. }) => {\n                        self.note_absent_sample();\n                        false\n                    }"
        ),
        "a requested adjacent image the observation does not hold must FAIL at \
         its reading site; serving the opening image in its place makes every \
         walk-side A/B a comparison of one image with itself",
    );

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
    let source_impl = plan_source
        .split_once(
            "pub fn read_into(&self, addr: usize, out: &mut [u8], image: D81Image) -> bool {",
        )
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

#[test]
fn a_table_the_engine_replaced_refuses_as_a_closure_change_and_not_a_plan_defect() {
    let world = stage_world(WorldShape::default());
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    let source = closed.source_for(WORLD_HOUR).expect("a view");

    d81_copied_roots_agree(
        &source,
        GAMESTATE as usize,
        RAIL_TABLE as usize,
        ACTIVE_TABLE as usize,
        SELECTOR_TABLE as usize,
    )
    .expect("an unmoved world must agree with its own copy");

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
    let failure =
        D81FirstFailure::new(D81FailureStage::Anchor, D81FailureReason::Unreadable).subject(42);
    let restamped = d81_restamp_plan_defect(failure, misses);
    assert_eq!(restamped.reason, D81FailureReason::NotCopied);
    assert_eq!(
        restamped.raw_value, 0x2000,
        "the refusal must carry the LAST miss -- the read that failed -- and \
         not the first, which may belong to a read that recovered",
    );
    assert_eq!(restamped.subject_id, 42, "the site's own witness survives");

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
    assert_eq!(
        D81ReadSource::paired_image(D81Image::ClosingSample(1)),
        D81Image::ClosingSample(1),
    );

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

#[test]
fn the_active_window_publishes_the_coherent_value_and_not_the_opening_image() {
    use crate::snapshot_recipes::Span;

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

    assert!(
        applied,
        "the fixture must actually have rewritten the window"
    );
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

fn active_terminal_of(province: u32) -> bool {
    let window = active_window_of(province);
    let state = crate::attach_log::d81_active_state_from_window(window);
    state.active_a != 0
        || state.active_b != 0
        || state.active_u16 != 0
        || state.active_flags & 1 != 0
}

#[test]
fn the_rail_header_closes_against_its_own_closing_image() {
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
    let final_wave_starts = final_wave_start(counting.calls(), &closed);
    {
        use crate::attach_log::d81_vector_store::D81Family;
        let vector = closed.vector.as_ref().expect("the pipeline ran");
        let tail = vector
            .copies
            .get()
            .saturating_sub(vector.closing_tail_start);
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

    assert!(
        applied,
        "the fixture must actually have rewritten the count"
    );
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

    let refusing = RefusingAtMemory::new(&world.memory, second_closing);
    let counting = CountingSnapshotMemory::new(&refusing);
    let mut counters = crate::attach_log::d81_copy_bracket::D81PlanCounters::default();
    let closed = d81_observe_copy_plan_with(&counting, &world.roots, WORLD_HOUR, &mut counters)
        .expect("custody");
    assert!(
        refusing.refused(),
        "the fixture must actually have refused a copy"
    );
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
    let between_closing = ordinals[reads + 1];

    let mut changed = active_window_of(CHANGED);
    changed[2] ^= 0x5A;
    let (walked, counters, applied) = copied_walk_with_mutation_at(
        stage_world(WorldShape::default()),
        vec![(window.start, changed.to_vec())],
        between_closing,
    );
    assert!(
        applied,
        "the fixture must actually have rewritten the window"
    );
    assert!(!counters.consumer_gapped);
    let failure =
        walked.expect_err("the closing sequence coheres on B and the closure against A refuses");
    assert_eq!(failure.stage, D81FailureStage::ActiveState);
    assert_eq!(failure.subject_id, CHANGED);
    assert_eq!(
        failure.reason,
        D81FailureReason::ClosureChanged,
        "the loop must COHERE on B and refuse the closure against opening A",
    );
}

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
    let closed =
        d81_observe_copy_plan_with(&counting, &roots, WORLD_HOUR, &mut counters).expect("custody");
    assert_eq!(scripted.applied(), 2, "both writes must land");
    let source = closed.source_for(WORLD_HOUR).expect("a view");
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
    assert!(
        refusing.refused(),
        "the fixture must actually decline a copy"
    );
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

    let direct_world = stage_world(WorldShape::default());
    let declining = RefusingNthAtMemory::new(&direct_world.memory, header.start, 2);
    let phases = D81PhaseAccum::default();
    let rows = walk_with(&direct_world, &D81ReadSource::Staged(&declining), &phases)
        .expect("the live arm recovers through its scalar reads and publishes");
    assert!(
        declining.refused(),
        "the fixture must decline the second window read"
    );
    assert_eq!(
        rows.provinces.len(),
        PUBLISHED_ROWS,
        "and it publishes every row, which is the hour the copy loses",
    );
}

#[test]
fn the_anchor_window_refuses_from_the_window_read_where_the_live_arm_recovers() {
    use crate::snapshot_recipes::Span;

    const CHANGED: u32 = 7;
    let world = stage_world(WorldShape::default());
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

    let direct_world = stage_world(WorldShape::default());
    let declining = RefusingNthAtMemory::new(&direct_world.memory, anchor.start, 2);
    let phases = D81PhaseAccum::default();
    let rows = walk_with(&direct_world, &D81ReadSource::Staged(&declining), &phases)
        .expect("the live arm recovers through its per-axis reads and publishes");
    assert!(declining.refused());
    assert_eq!(rows.provinces.len(), PUBLISHED_ROWS);
}

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
    assert!(
        counters.plan_us * 5 >= phase_us * 4,
        "the plan's own word must account for the enclosing window, not a slice \
         of it: phase {phase_us} us against plan {} us",
        counters.plan_us,
    );

    use crate::snapshot_recipes::executor::{observe, ObservationConfig};
    const PASSES: usize = 5;
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
