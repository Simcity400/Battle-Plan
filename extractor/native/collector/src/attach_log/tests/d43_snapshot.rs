//! Lane PD: d43's bounded snapshot, driven end to end over a world staged at
//! the cardinalities `runs/perf_d43_market/P4_D43_FRAME_UNITS.json` recorded
//! (pair_1_session_2, 355 frames): 439 country roots, 62 contracts, 412
//! routes, 505 stockpile items, 3260 land units, 144 fleets, 225 task forces,
//! 1282 wings visited per frame (641 per arm, 888 of them idle).

use super::state_fixture::{
    compact_control_scheduling, drive_compact_frame_carrying, fixture_game_hour_raw,
    CollectorStateFixture, FixtureRegion, FrameOutcome,
};
use super::*;
use crate::attach_log::d43_snapshot_bracket::{
    capture_compact_market_trade_snapshot, domain43_eligibility_mask,
    finish_compact_market_trade_snapshot, with_engine_call_override,
    CompactMarketTradeSnapshotTracker, Domain43CapturedRead, Domain43EngineCallCapture,
    Domain43RegionGate, D43_SNAPSHOT_MODE_ARMING, D43_SNAPSHOT_MODE_LIVE,
    D43_SNAPSHOT_MODE_SNAPSHOT,
};
use crate::attach_log::d43_snapshot_memory::{
    plan_refusal_code, Domain43AccessTrace, Domain43ReadTrace, Domain43SnapshotLimits,
    Domain43SnapshotPlan, Domain43TracingMemory, D43_SNAPSHOT_BYTE_CAP_CAUSE,
    D43_SNAPSHOT_OS_REGION_CAP_CAUSE, D43_SNAPSHOT_REGION_CAP_CAUSE,
    D43_SNAPSHOT_TRACE_CAP_CAUSE,
};
use compact_contract::build_profile as bp;
use compact_contract::payload_span_delta::PayloadSpanDeltaConsumer;
use std::ffi::c_void;
use std::ptr;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        alloc_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
}
const MEM_RESERVE: Dword = 0x2000;
const MEM_RELEASE: Dword = 0x8000;
const PAGE_READWRITE: Dword = 0x04;
const PAGE_EXECUTE_READWRITE: Dword = 0x40;
const PAGE_READONLY: Dword = 0x02;
const PAGE: usize = 0x1000;

/// `pages` pages reserved (not committed) inside the map-control heap range,
/// so a pointer into them passes `is_map_control_heap_pointer` and a read of
/// them fails the live reader's gate. The caller commits and releases.
fn reserve_pages_in_heap_range(pages: usize) -> usize {
    for k in 0..256u64 {
        let candidate = 0x2000_0000_0000u64 + k * 0x1_0000_0000;
        let base = unsafe {
            VirtualAlloc(candidate as *mut c_void, pages * PAGE, MEM_RESERVE, PAGE_READWRITE)
        };
        if !base.is_null() {
            return base as usize;
        }
    }
    panic!("no free reservation in the heap range");
}

fn commit_page(page: usize) {
    let committed = unsafe { VirtualAlloc(page as *mut c_void, PAGE, MEM_COMMIT, PAGE_READWRITE) };
    assert_eq!(committed as usize, page, "commit");
}

/// `len` bytes of the fixture heap at `offset`, placed at `address`.
fn place_heap_bytes(world: &World, offset: usize, len: usize, address: usize) {
    unsafe {
        ptr::copy_nonoverlapping((world.heap_base + offset) as *const u8, address as *mut u8, len)
    };
}

fn carries_trailer(payload: &[u8], magic: &[u8]) -> bool {
    payload.windows(magic.len()).any(|window| window == magic)
}

fn protect_page(page: usize, protect: Dword) {
    let mut old: Dword = 0;
    let protected = unsafe { VirtualProtect(page as *mut c_void, PAGE, protect, &mut old) };
    assert_ne!(protected, 0, "protect");
}

fn release_pages(base: usize) {
    unsafe { VirtualFree(base as *mut c_void, 0, MEM_RELEASE) };
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    countries: usize,
    resources: usize,
    contracts: usize,
    routes: usize,
    stockpile_items: usize,
    units: usize,
    fleets: usize,
    task_forces: usize,
    wings_per_arm: usize,
    idle_wings_per_arm: usize,
    lines_per_country: usize,
    entries_per_line: usize,
}

/// P4's receipts, per frame.
const P4_SHAPE: Shape = Shape {
    countries: 439,
    resources: 7,
    contracts: 62,
    routes: 412,
    stockpile_items: 505,
    units: 3260,
    fleets: 144,
    task_forces: 225,
    wings_per_arm: 641,
    idle_wings_per_arm: 444,
    lines_per_country: 2,
    entries_per_line: 3,
};

/// The same world at a size the refusal-path tests can drive many frames of.
const SMALL_SHAPE: Shape = Shape {
    countries: 12,
    resources: 7,
    contracts: 6,
    routes: 10,
    stockpile_items: 15,
    units: 40,
    fleets: 5,
    task_forces: 8,
    wings_per_arm: 18,
    idle_wings_per_arm: 12,
    lines_per_country: 1,
    entries_per_line: 2,
};

const TOKENS: [&str; 7] = [
    "oil",
    "steel",
    "rubber",
    "tungsten",
    "aluminium",
    "chromium",
    "coal",
];

const ELAPSED_WALL_MS: u64 = 11;

/// Where the world's objects sit relative to one another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// Packed at 16-byte alignment: the smallest heap, the most cross-object
    /// merging a plan can see.
    Contiguous,
    /// 64..4160 bytes between consecutive objects (xorshift), so no two
    /// objects merge at the production gap: the region count a scattered
    /// heap costs this shape.
    Scattered,
}

impl Layout {
    fn heap_len(self) -> usize {
        match self {
            Layout::Contiguous => 0x1C0_0000,
            Layout::Scattered => 0x600_0000,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Layout::Contiguous => "contiguous",
            Layout::Scattered => "scattered",
        }
    }
}

/// `total` spread over `n` holders: holder `i` gets its floor share plus one
/// of the remainder while any is left.
fn share(total: usize, n: usize, i: usize) -> usize {
    total / n + usize::from(i < total % n)
}

struct Bump {
    next: usize,
    len: usize,
    scatter: Option<u64>,
}

impl Bump {
    fn new(layout: Layout) -> Self {
        Self {
            next: 0,
            len: layout.heap_len(),
            scatter: (layout == Layout::Scattered).then_some(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn alloc(&mut self, len: usize) -> usize {
        if let Some(state) = self.scatter.as_mut() {
            let mut x = *state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *state = x;
            self.next += 64 + 16 * (x % 256) as usize;
        }
        let at = (self.next + 15) & !15;
        let end = at + len.max(1);
        assert!(
            end <= self.len,
            "the world outgrew its heap: {end:#x} > {:#x}",
            self.len
        );
        self.next = end;
        at
    }
}

struct Country {
    object: usize,
    resources: usize,
    buckets: usize,
    relation_data: usize,
    produced_values: usize,
    units: Vec<usize>,
    stockpile_items: usize,
    stockpile_pool: usize,
    container: usize,
    wings: Vec<usize>,
}

struct World {
    fixture: CollectorStateFixture,
    bump: Bump,
    heap_base: usize,
    shape: Shape,
    gs: usize,
    countries: Vec<Country>,
    routes: Vec<usize>,
    contracts: Vec<usize>,
    contributing_wings: Vec<usize>,
    /// The executable stub the eligibility slot points at, when staged.
    engine_stub: Option<usize>,
}

impl World {
    fn addr(&self, offset: usize) -> u64 {
        (self.heap_base + offset) as u64
    }

    fn state(&self) -> CollectorState {
        self.fixture.collector_state()
    }

    fn set_hour(&mut self, hour: i32) {
        self.fixture
            .heap
            .write_i32(self.gs + GS_CURRENT_DATE_OFFSET, hour);
    }

    /// A handful of words every family publishes, moved so a frame that
    /// carried the old ones is distinguishable from one that carried these.
    fn mutate(&mut self, salt: i64) {
        let heap = &mut self.fixture.heap;
        let first = &self.countries[0];
        heap.write_i64(first.units[0] + 0x628, 20_000 + salt);
        heap.write_i64(first.produced_values + 0x10 * 2, 1_700_000 + 100_000 * salt);
        let second = &self.countries[1];
        heap.write_i64(second.stockpile_items + CEQUIPMENT_POOL_ITEM_AMOUNT_OFF, 9_000 + salt);
        heap.write_i64(
            self.routes[0] + CRESOURCE_DELIVERY_ROUTE_DELIVERED_RAW_OFF,
            720_000 + 1_000 * salt,
        );
        heap.write_i32(
            self.contracts[0] + bp::DOMAIN43_PURCHASE_CONTRACT_DAYS,
            12 + salt as i32,
        );
        heap.write_i32(self.contributing_wings[0] + 0x170, 1 + (salt as i32 % 3));
    }

    /// One more wing on a country's container, in a wing vector the walk has
    /// never read: the shape a plan cannot have covered.
    fn grow_wings(&mut self, country: usize) {
        let container = self.countries[country].container;
        let old = self.countries[country].wings.clone();
        let wing = self.bump.alloc(0x9D0);
        let wing_ptrs = self.bump.alloc(8 * (old.len() + 1));
        let heap = &mut self.fixture.heap;
        stage_wing(heap, wing, false, 3, country + 1, 4);
        for (k, existing) in old.iter().enumerate() {
            heap.write_u64(wing_ptrs + 8 * k, (self.heap_base + existing) as u64);
        }
        heap.write_u64(wing_ptrs + 8 * old.len(), (self.heap_base + wing) as u64);
        heap.write_u64(container + 0x28, (self.heap_base + wing_ptrs) as u64);
        heap.write_i32(container + 0x34, (old.len() + 1) as i32);
        self.countries[country].wings.push(wing);
        self.contributing_wings.push(wing);
    }
}

fn stage_wing(heap: &mut FixtureRegion, wing: usize, idle: bool, mission_bit: u32, tag: usize, count: i32) {
    if idle {
        heap.write_i32(wing + 0xA4, 0);
        return;
    }
    heap.write_i32(wing + 0xA4, 1 << mission_bit);
    heap.write_i32(wing + 0x170, count);
    heap.write_i64(wing + 0x5B8, 10_000);
    heap.write_i32(wing + 0x9C4, tag as i32);
    heap.write_u8(wing + 0xA8, 2);
}

/// A values vector `(resources + 1) * 0x10` wide, subscripted by the engine's
/// one-based ordinal, hung off `ptr_slot`.
fn stage_values(
    heap: &mut FixtureRegion,
    bump: &mut Bump,
    heap_base: usize,
    resources: usize,
    ptr_slot: usize,
    seed: i64,
) -> usize {
    let values = bump.alloc(0x10 * (resources + 1));
    for j in 1..=resources {
        heap.write_i64(values + 0x10 * j, seed + 100_000 * j as i64);
    }
    heap.write_u64(ptr_slot, (heap_base + values) as u64);
    values
}

fn build_world(shape: Shape) -> World {
    build_world_with(shape, Layout::Contiguous)
}

fn build_world_with(shape: Shape, layout: Layout) -> World {
    build_world_staged(shape, layout, false)
}

/// The world with the reader's one engine CALL reachable: a PE header on the
/// module window, an executable stub the eligibility slot points at, and the
/// modifier definition the staged rows resolve to asking for eligibility.
fn build_world_with_engine_callback(shape: Shape) -> World {
    build_world_staged(shape, Layout::Contiguous, true)
}

const ENGINE_STUB_MASK: u32 = 0xAB;

fn build_world_staged(shape: Shape, layout: Layout, engine_callback: bool) -> World {
    assert!(shape.resources <= TOKENS.len());
    assert!(shape.countries >= 2);
    let mut rvas = vec![
        C_GAME_STATE_DAILY_UPDATE_OFFSET,
        GET_LONG_STRING_OFFSET,
        bp::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        CSTRATEGIC_RESOURCE_DATABASE_PTR_RVA,
        PDX_LEXER_INITIALIZED_RVA,
        PDX_LEXER_MAX_TOKEN_ID_RVA,
        PDX_LEXER_TOKEN_NAME_TABLE_PTR_RVA,
        PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA,
        MODIFIER_DEFINITION_TABLE_RVA,
        OIL_CURRENT_MAX_GLOBAL_UNIT_A_RVA,
        OIL_CURRENT_MAX_GLOBAL_UNIT_B_RVA,
        OIL_CURRENT_MAX_GLOBAL_UNIT_C_RVA,
        OIL_CURRENT_MAX_GLOBAL_NAVAL_A_RVA,
        OIL_CURRENT_MAX_GLOBAL_NAVAL_B_RVA,
        OIL_CURRENT_MAX_GLOBAL_AIR_WEIGHT_RVA,
        OIL_CURRENT_MAX_GLOBAL_AIR_MISSION_COSTS_RVA,
    ];
    let stub_rva = engine_callback.then(|| {
        let top = *rvas.iter().max().expect("staged RVAs");
        let stub_rva = (top + 2 * PAGE) & !(PAGE - 1);
        rvas.extend([0, MODIFIER_ELIGIBILITY_CALLBACK_RVA, stub_rva + PAGE]);
        stub_rva
    });
    let mut fixture = CollectorStateFixture::new(&rvas, 0x1000, layout.heap_len());
    fixture.stage_profile_sentinels();
    let heap_base = fixture.heap.addr();
    let a = |offset: usize| (heap_base + offset) as u64;
    let base = fixture.module.module_base();
    let mut bump = Bump::new(layout);
    let n = shape.countries;
    let r = shape.resources;

    let gs = bump.alloc(0x700);
    fixture.stage_scheduler_clock_at(gs);
    let heap = &mut fixture.heap;

    let country_table = bump.alloc(8 * (n + 1));
    let link_table = bump.alloc(4 * (n + 1));
    heap.write_u64(gs + CGAMESTATE_COUNTRY_TABLE, a(country_table));
    heap.write_i32(gs + CGAMESTATE_COUNTRY_COUNT, (n + 1) as i32);
    heap.write_u64(gs + bp::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_PTR, a(link_table));
    heap.write_i32(gs + bp::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_COUNT, (n + 1) as i32);
    for tag in 1..=n {
        heap.write_i32(link_table + 4 * tag, tag as i32);
    }

    let manager = bump.alloc(0x60);
    heap.write_u64(gs + CGAMESTATE_MARKET_MANAGER_OFF, a(manager));
    let outer = [bump.alloc(0x18 * (n + 1)), bump.alloc(0x18 * (n + 1))];
    for (t, table_off) in MARKET_CONTRACT_TABLE_OFFSETS.iter().enumerate() {
        heap.write_u64(manager + table_off + MARKET_VECTOR_BEGIN_OFF, a(outer[t]));
        heap.write_i32(manager + table_off + MARKET_VECTOR_CAP_OFF, (n + 1) as i32);
        heap.write_i32(manager + table_off + MARKET_VECTOR_COUNT_OFF, (n + 1) as i32);
    }

    let air_manager = bump.alloc(0x40);
    let air_items = bump.alloc(8 * (n + 1));
    heap.write_u64(gs + CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER, a(air_manager));
    heap.write_u64(air_manager + 0x30, a(air_items));
    heap.write_i32(air_manager + 0x3C, (n + 1) as i32);
    let witness = bump.alloc(0xD8);
    heap.write_i64(witness + 0xD0, 1);
    let mission_costs = bump.alloc(18 * 8);
    for slot in 0..18usize {
        heap.write_i64(mission_costs + 8 * slot, 100_000 + 1_000 * slot as i64);
    }
    let definitions_table = bump.alloc(0x19E * 0x78 + 0x78);

    let db = bump.alloc(0x80);
    let db_items = bump.alloc(8 * (r + 1));
    let db_keys = bump.alloc(4 * r);
    let null_object = bump.alloc(0xE8);
    heap.write_u8(null_object + CSTRATEGIC_RESOURCE_ACTIVE_OFF, 1);
    heap.write_u64(db_items, a(null_object));
    let mut descriptors = Vec::with_capacity(r);
    for j in 1..=r {
        let descriptor = bump.alloc(0xE8);
        heap.write_u32(descriptor + CSTRATEGIC_RESOURCE_TOKEN_ID_OFF, j as u32);
        heap.write_u8(descriptor + CSTRATEGIC_RESOURCE_ACTIVE_OFF, 1);
        heap.write_i32(descriptor + CSTRATEGIC_RESOURCE_INDEX_OFF, j as i32);
        heap.write_i64(
            descriptor + CSTRATEGIC_RESOURCE_FACTORY_COST_RAW_OFF,
            100_000 + 10_000 * j as i64,
        );
        heap.write_u64(db_items + 8 * j, a(descriptor));
        heap.write_u32(db_keys + 4 * (j - 1), j as u32);
        descriptors.push(descriptor);
    }
    heap.write_u64(db + CSTRATEGIC_RESOURCE_DATABASE_ITEMS_OFF, a(db_items));
    heap.write_i32(db + CSTRATEGIC_RESOURCE_DATABASE_COUNT_OFF, (r + 1) as i32);
    heap.write_u64(db + CSTRATEGIC_RESOURCE_DATABASE_KEYS_OFF, a(db_keys));
    heap.write_i32(db + CSTRATEGIC_RESOURCE_DATABASE_KEY_COUNT_OFF, r as i32);
    heap.write_u64(db + CSTRATEGIC_RESOURCE_DATABASE_OIL_OFF, a(descriptors[0]));
    heap.write_u64(db + 0x68, a(null_object));
    heap.write_u64(db + 0x70, a(null_object));
    let token_table = bump.alloc(PDX_LEXER_TOKEN_NAME_STRIDE * (r + 1));
    for (j, token) in TOKENS.iter().take(r).enumerate() {
        let entry = token_table + PDX_LEXER_TOKEN_NAME_STRIDE * (j + 1);
        heap.write_bytes(entry, token.as_bytes());
        heap.write_u64(entry + 0x10, token.len() as u64);
        heap.write_u64(entry + 0x18, 15);
    }

    let country_vtable = base + COUNTRY_VTABLE_RVA;
    let resources_vtable = (base + CCOUNTRY_RESOURCES_VTABLE_RVA) as u64;
    let pool_vtable = (base + CSTRATEGIC_RESOURCE_POOL_VTABLE_RVA) as u64;
    let variant_vtable = (base + VARIANT_VTABLE_RVA) as u64;
    let variant_pool_vtable = (base + CEQUIPMENT_VARIANT_POOL_VTABLE_RVA) as u64;
    let contract_vtable = (base + bp::DOMAIN43_PURCHASE_CONTRACT_VTABLE_RVA) as u64;
    let unit_vtable = (base + DIVISION_WRAPPER_VTABLE_RVA) as u64;
    let fleet_vtable = (base + FLEET_VTABLE_RVA) as u64;
    let task_force_vtable = (base + TASK_FORCE_SUBOBJECT_VTABLE_RVA) as u64;
    let definitions: Vec<usize> = (0..64usize)
        .map(|k| {
            let definition = bump.alloc(0x2D0);
            heap.write_i64(definition + 0x2C8, 400_000 + 1_000 * k as i64);
            definition
        })
        .collect();

    let mut countries = Vec::with_capacity(n);
    let mut global_fleet = 0usize;
    let mut global_item = 0usize;
    let mut global_wing = 0usize;
    let mut contributing_wings = Vec::new();
    for i in 1..=n {
        let object = bump.alloc(0x1600);
        heap.write_u64(object, country_vtable as u64);
        heap.write_u32(object + 0x08, i as u32);
        heap.write_u64(country_table + 8 * i, a(object));
        if i % 5 == 0 {
            let rows = bump.alloc(0x10);
            heap.write_u32(rows, 0x19E);
            heap.write_i64(rows + 8, 25_000);
            heap.write_u64(object + CCOUNTRY_MODIFIER_CONTAINER_OFF, a(rows));
            heap.write_i32(object + CCOUNTRY_MODIFIER_CONTAINER_OFF + 0xC, 1);
        }

        let resources = bump.alloc(0x780);
        heap.write_u64(resources, resources_vtable);
        heap.write_u32(resources + 0x20, i as u32);
        let mut produced_values = 0;
        for &(_, pool_off) in &CCOUNTRY_RESOURCES_POOLS {
            heap.write_u64(resources + pool_off, pool_vtable);
            let values = stage_values(
                heap,
                &mut bump,
                heap_base,
                r,
                resources + pool_off + CSTRATEGIC_RESOURCE_POOL_DATA_PTR_OFF,
                1_000_000 + 7_000 * i as i64 + pool_off as i64,
            );
            if pool_off + CSTRATEGIC_RESOURCE_POOL_DATA_PTR_OFF == 0x30 {
                produced_values = values;
            }
        }
        for slot in [
            CCOUNTRY_RESOURCES_ADJUSTMENT_VALUES_OFF,
            CCOUNTRY_RESOURCES_SPECIAL_TOTAL_VALUES_OFF,
            CCOUNTRY_RESOURCES_PRODUCTION_LINE_VALUES_OFF,
        ] {
            stage_values(heap, &mut bump, heap_base, r, resources + slot, 300_000 + slot as i64);
        }
        let buckets = bump.alloc(0x18 * (r + 1));
        heap.write_u64(resources + CCOUNTRY_RESOURCES_EXPORT_ROUTE_BUCKETS_OFF, a(buckets));
        heap.write_u64(object + CCOUNTRY_RESOURCES_OFF, a(resources));

        let production = bump.alloc(0x460);
        heap.write_u64(production + CPRODUCTION_STATUS_OWNER_COUNTRY_OFF, a(object));
        heap.write_i64(
            production + CPRODUCTION_STATUS_FACTORY_TOTAL_RAW_OFF,
            4_200_000 + 100_000 * (i % 9) as i64,
        );
        heap.write_i64(production + CPRODUCTION_STATUS_FACTORY_AVAILABLE_SUB_RAW_OFF, 300_000);
        heap.write_i32(production + CPRODUCTION_STATUS_FACTORY_AVAILABLE_SUB_COUNT_OFF, 2);
        heap.write_i32(production + CPRODUCTION_STATUS_FACTORY_TOTAL_SUB_0_OFF, 1);
        heap.write_i32(production + CPRODUCTION_STATUS_FACTORY_TOTAL_SUB_2_OFF, 1);
        heap.write_i32(production + CPRODUCTION_STATUS_FACTORY_USED_OFF, 30);
        let lines = bump.alloc(8 * shape.lines_per_country);
        heap.write_u64(production + 0x58, a(lines));
        heap.write_i32(production + 0x64, shape.lines_per_country as i32);
        for l in 0..shape.lines_per_country {
            let line = bump.alloc(0x70);
            let entries = bump.alloc(0x20 * shape.entries_per_line);
            heap.write_u64(line + 0x58, a(entries));
            heap.write_i32(line + 0x64, shape.entries_per_line as i32);
            for e in 0..shape.entries_per_line {
                let entry = entries + 0x20 * e;
                heap.write_u64(entry + 8, a(descriptors[(l + e) % r]));
                heap.write_i64(entry + 0x10, 150_000);
                heap.write_i64(entry + 0x18, 50_000);
            }
            heap.write_u64(lines + 8 * l, a(line));
        }
        heap.write_u64(object + CCOUNTRY_PRODUCTION_STATUS_OFF, a(production));

        let politics = bump.alloc(0x360);
        let relation_data = bump.alloc(8 * (n + 1));
        heap.write_u64(politics + 8, a(relation_data));
        heap.write_u64(object + CCOUNTRY_POLITICS_STATUS_OFF, a(politics));

        let fuel = bump.alloc(0xF0);
        heap.write_i64(fuel + 0x18, 900_000);
        heap.write_i64(fuel + 0x20, 100_000);
        heap.write_i64(fuel + 0x38, 50_000);
        heap.write_i64(fuel + 0x40, 1_000);
        heap.write_u64(fuel + 0xE0, a(object));
        heap.write_u64(object + CCOUNTRY_FUEL_STATUS_OFF, a(fuel));

        let unit_count = share(shape.units, n, i - 1);
        let unit_ptrs = bump.alloc(8 * unit_count);
        let mut units = Vec::with_capacity(unit_count);
        for k in 0..unit_count {
            let unit = bump.alloc(0x630);
            heap.write_u64(unit, unit_vtable);
            heap.write_u64(unit + 0x138, a(definitions[(i + k) % definitions.len()]));
            heap.write_i64(unit + 0x628, 20_000 + 10 * k as i64);
            heap.write_u64(unit_ptrs + 8 * k, a(unit));
            units.push(unit);
        }
        heap.write_u64(object + 0x290, a(unit_ptrs));
        heap.write_i32(object + 0x29C, unit_count as i32);

        let fleet_count = share(shape.fleets, n, i - 1);
        let fleet_ptrs = bump.alloc(8 * fleet_count);
        for k in 0..fleet_count {
            let fleet = bump.alloc(0xD0);
            heap.write_u64(fleet, fleet_vtable);
            let task_force_count = share(shape.task_forces, shape.fleets, global_fleet);
            let task_force_ptrs = bump.alloc(8 * task_force_count);
            for t in 0..task_force_count {
                let task_force = bump.alloc(0x510);
                heap.write_u64(task_force, task_force_vtable);
                heap.write_u64(
                    task_force + 0x138,
                    a(definitions[(global_fleet + t) % definitions.len()]),
                );
                heap.write_i64(task_force + 0x500, 1_000 + 5 * t as i64);
                heap.write_u64(task_force_ptrs + 8 * t, a(task_force));
            }
            heap.write_u64(fleet + 0xB8, a(task_force_ptrs));
            heap.write_i32(fleet + 0xC4, task_force_count as i32);
            heap.write_u64(fleet_ptrs + 8 * k, a(fleet));
            global_fleet += 1;
        }
        heap.write_u64(object + 0x278, a(fleet_ptrs));
        heap.write_i32(object + 0x284, fleet_count as i32);

        let wrapper = bump.alloc(0x80);
        let stockpile = bump.alloc(0x80);
        heap.write_u64(wrapper + CEQUIPMENT_MARKET_STATUS_STOCKPILE_OFF, a(stockpile));
        heap.write_u64(wrapper + CEQUIPMENT_MARKET_STATUS_MANAGER_OFF, a(manager));
        heap.write_u32(stockpile + CMARKET_STOCKPILE_OWNER_TAG_DB_ID_OFF, i as u32);
        let pool = stockpile + CMARKET_STOCKPILE_EQUIPMENT_POOL_OFF;
        heap.write_u64(pool, variant_pool_vtable);
        let item_count = share(shape.stockpile_items, n, i - 1);
        let stockpile_items = bump.alloc(CEQUIPMENT_POOL_ITEM_STRIDE * item_count);
        for k in 0..item_count {
            let variant = bump.alloc(0x40);
            heap.write_u64(variant, variant_vtable);
            heap.write_u32(variant + VARIANT_DEF_TYPE, VARIANT_TYPE);
            heap.write_u32(variant + VARIANT_DEF_ID, 5_000 + global_item as u32);
            let item = stockpile_items + CEQUIPMENT_POOL_ITEM_STRIDE * k;
            heap.write_u64(item + CEQUIPMENT_POOL_ITEM_VARIANT_OFF, a(variant));
            heap.write_i64(item + CEQUIPMENT_POOL_ITEM_AMOUNT_OFF, 1_000 + global_item as i64);
            global_item += 1;
        }
        heap.write_u64(pool + CEQUIPMENT_POOL_ITEMS_BEGIN_OFF, a(stockpile_items));
        heap.write_i32(pool + CEQUIPMENT_POOL_ITEMS_CAP_OFF, item_count as i32);
        heap.write_i32(pool + CEQUIPMENT_POOL_ITEMS_SIZE_OFF, item_count as i32);
        heap.write_u64(object + CCOUNTRY_MARKET_STOCKPILE_WRAPPER_OFF, a(wrapper));

        let entry = bump.alloc(0x40);
        let tables = bump.alloc(8);
        let table = bump.alloc(0x70);
        let slots = bump.alloc(8 * (n + 1));
        let level1 = bump.alloc(0xD8);
        let containers = bump.alloc(8);
        let container = bump.alloc(0x40);
        heap.write_u64(air_items + 8 * i, a(entry));
        heap.write_u64(entry + 0x10, a(tables));
        heap.write_i32(entry + 0x1C, 1);
        heap.write_u64(tables, a(table));
        heap.write_u64(table + 0x30, a(slots));
        heap.write_u64(slots, a(witness));
        heap.write_u64(slots + 8 * i, a(level1));
        heap.write_u64(level1 + 0xB8, a(containers));
        heap.write_i32(level1 + 0xC4, 1);
        heap.write_u64(containers, a(container));
        let wing_count = share(shape.wings_per_arm, n, i - 1);
        let wing_ptrs = bump.alloc(8 * wing_count);
        let mut wings = Vec::with_capacity(wing_count);
        for k in 0..wing_count {
            let wing = bump.alloc(0x9D0);
            let idle = (global_wing + 1) * shape.idle_wings_per_arm / shape.wings_per_arm
                > global_wing * shape.idle_wings_per_arm / shape.wings_per_arm;
            stage_wing(
                heap,
                wing,
                idle,
                1 + (global_wing % 5) as u32,
                i,
                1 + (global_wing % 3) as i32,
            );
            if !idle {
                contributing_wings.push(wing);
            }
            heap.write_u64(wing_ptrs + 8 * k, a(wing));
            wings.push(wing);
            global_wing += 1;
        }
        heap.write_u64(container + 0x28, a(wing_ptrs));
        heap.write_i32(container + 0x34, wing_count as i32);

        countries.push(Country {
            object,
            resources,
            buckets,
            relation_data,
            produced_values,
            units,
            stockpile_items,
            stockpile_pool: pool,
            container,
            wings,
        });
    }

    let mut exports: Vec<Vec<Vec<usize>>> = vec![vec![Vec::new(); r + 1]; n + 1];
    let mut imports: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut routes = Vec::with_capacity(shape.routes);
    for k in 0..shape.routes {
        let exporter = 1 + k % n;
        let importer = 1 + (k + 1) % n;
        let engine_index = 1 + k % r;
        let route = bump.alloc(0xF0);
        heap.write_u64(
            route + CRESOURCE_DELIVERY_ROUTE_EXPORTER_OFF,
            a(countries[exporter - 1].resources),
        );
        heap.write_i32(route + CRESOURCE_DELIVERY_ROUTE_IMPORTER_TAG_OFF, importer as i32);
        heap.write_u64(
            route + CRESOURCE_DELIVERY_ROUTE_RESOURCE_OFF,
            a(descriptors[engine_index - 1]),
        );
        heap.write_i64(
            route + CRESOURCE_DELIVERY_ROUTE_DELIVERED_RAW_OFF,
            720_000 + 1_000 * k as i64,
        );
        heap.write_i32(route + CRESOURCE_DELIVERY_ROUTE_REQUESTED_INPUT_OFF, 8);
        heap.write_i32(route + CRESOURCE_DELIVERY_ROUTE_CIVILIAN_FACTORIES_OFF, 3);
        heap.write_i32(route + CRESOURCE_DELIVERY_ROUTE_CONVOYS_ASSIGNED_OFF, 4);
        heap.write_i32(
            route + CRESOURCE_DELIVERY_ROUTE_CONVOYS_REQUIRED_OFF,
            if k % 2 == 0 { 6 } else { 0 },
        );
        heap.write_i64(route + CRESOURCE_DELIVERY_ROUTE_EFFICIENCY_A_OFF, 100_000);
        heap.write_i64(route + CRESOURCE_DELIVERY_ROUTE_EFFICIENCY_B_OFF, 90_000);
        let relation_entry = bump.alloc(0x150);
        heap.write_u64(
            countries[exporter - 1].relation_data + 8 * importer,
            a(relation_entry),
        );
        exports[exporter][engine_index].push(route);
        imports[importer].push(route);
        routes.push(route);
    }
    for i in 1..=n {
        for j in 1..=r {
            let bucket_routes = &exports[i][j];
            if bucket_routes.is_empty() {
                continue;
            }
            let array = bump.alloc(8 * bucket_routes.len());
            for (q, route) in bucket_routes.iter().enumerate() {
                heap.write_u64(array + 8 * q, a(*route));
            }
            let bucket = countries[i - 1].buckets + 0x18 * j;
            heap.write_u64(bucket, a(array));
            heap.write_i32(bucket + 0xC, bucket_routes.len() as i32);
        }
        let import_routes = &imports[i];
        if !import_routes.is_empty() {
            let array = bump.alloc(8 * import_routes.len());
            for (q, route) in import_routes.iter().enumerate() {
                heap.write_u64(array + 8 * q, a(*route));
            }
            let resources = countries[i - 1].resources;
            heap.write_u64(resources + CCOUNTRY_RESOURCES_IMPORT_ROUTES_OFF, a(array));
            heap.write_i32(
                resources + CCOUNTRY_RESOURCES_IMPORT_ROUTE_COUNT_OFF,
                import_routes.len() as i32,
            );
        }
    }

    let mut by_seller: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut by_buyer: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut contracts = Vec::with_capacity(shape.contracts);
    for k in 0..shape.contracts {
        let seller = 1 + (k * 7) % n;
        let buyer = 1 + (k * 7 + 3) % n;
        assert_ne!(seller, buyer);
        let contract = bump.alloc(0x280);
        heap.write_u64(contract, contract_vtable);
        heap.write_u32(
            contract + bp::DOMAIN43_PURCHASE_CONTRACT_ID_TYPE,
            bp::DOMAIN43_PURCHASE_CONTRACT_TYPE,
        );
        heap.write_u32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_ID_ID, (k + 1) as u32);
        heap.write_u32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_SELLER_TAG_DB_ID, seller as u32);
        heap.write_u32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_BUYER_TAG_DB_ID, buyer as u32);
        heap.write_u64(
            contract + bp::DOMAIN43_PURCHASE_CONTRACT_BUYER_PTR,
            a(countries[buyer - 1].object),
        );
        let variant = bump.alloc(0x40);
        heap.write_u64(variant, variant_vtable);
        heap.write_u32(variant + VARIANT_DEF_TYPE, VARIANT_TYPE);
        heap.write_u32(variant + VARIANT_DEF_ID, 1_000 + k as u32);
        let equipment = bump.alloc(MARKET_TRADE_SUBBUFFER_SCAN_BYTES);
        heap.write_u64(equipment, a(variant));
        heap.write_i32(equipment + 8, 5 + k as i32);
        let delivery = bump.alloc(MARKET_TRADE_SUBBUFFER_SCAN_BYTES);
        heap.write_u64(delivery, a(variant));
        heap.write_i32(delivery + 8, 40 + k as i32);
        heap.write_u64(contract + bp::DOMAIN43_PURCHASE_CONTRACT_EQUIPMENT_SUBBUF, a(equipment));
        heap.write_u64(contract + bp::DOMAIN43_PURCHASE_CONTRACT_DELIVERY_SUBBUF, a(delivery));
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_SPEED_RAW, 1_000);
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_CONVOYS_ASSIGNED, 2);
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_CONVOYS_TOTAL, 4);
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_EFFICIENCY_RAW, 90_000);
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_DAYS, 12);
        heap.write_i32(contract + bp::DOMAIN43_PURCHASE_CONTRACT_FACTORY_CIC_PROGRESS_RAW, 30_000);
        heap.write_i32(
            contract + bp::DOMAIN43_PURCHASE_CONTRACT_EQUIPMENT_TRANSFER_PROGRESS_RAW,
            40_000,
        );
        by_seller[seller].push(contract);
        by_buyer[buyer].push(contract);
        contracts.push(contract);
    }
    for (t, table) in [&by_seller, &by_buyer].into_iter().enumerate() {
        for (slot, members) in table.iter().enumerate() {
            if members.is_empty() {
                continue;
            }
            let body = bump.alloc(8 * members.len());
            for (q, contract) in members.iter().enumerate() {
                heap.write_u64(body + 8 * q, a(*contract));
            }
            let header = outer[t] + MARKET_CONTRACT_VECTOR_STRIDE * slot;
            heap.write_u64(header + MARKET_VECTOR_BEGIN_OFF, a(body));
            heap.write_i32(header + MARKET_VECTOR_CAP_OFF, members.len() as i32);
            heap.write_i32(header + MARKET_VECTOR_COUNT_OFF, members.len() as i32);
        }
    }

    let engine_stub = stub_rva.map(|stub_rva| {
        // The module's PE header: one executable section holding the stub.
        let module = &mut fixture.module;
        module.write_bytes_at_rva(0, b"MZ");
        module.write_bytes_at_rva(0x3C, &0x80u32.to_le_bytes());
        module.write_bytes_at_rva(0x80, b"PE\0\0");
        module.write_bytes_at_rva(0x86, &1u16.to_le_bytes());
        module.write_bytes_at_rva(0x94, &0xF0u16.to_le_bytes());
        let section = 0x80 + 0x18 + 0xF0;
        module.write_bytes_at_rva(section + 0x8, &(PAGE as u32).to_le_bytes());
        module.write_bytes_at_rva(section + 0xC, &(stub_rva as u32).to_le_bytes());
        module.write_bytes_at_rva(section + 0x10, &(PAGE as u32).to_le_bytes());
        module.write_bytes_at_rva(section + 0x24, &0x6000_0020u32.to_le_bytes());
        // `mov eax, ENGINE_STUB_MASK; ret` on a page made executable.
        let mut code = vec![0xB8u8];
        code.extend_from_slice(&ENGINE_STUB_MASK.to_le_bytes());
        code.push(0xC3);
        module.write_bytes_at_rva(stub_rva, &code);
        let stub = base + stub_rva;
        let mut old: Dword = 0;
        let protected = unsafe {
            VirtualProtect(stub as *mut c_void, PAGE, PAGE_EXECUTE_READWRITE, &mut old)
        };
        assert_ne!(protected, 0, "the stub page is executable");
        module.write_u64_at_rva(MODIFIER_ELIGIBILITY_CALLBACK_RVA, stub as u64);
        // The definition the staged 0x19E rows resolve to asks for eligibility.
        fixture
            .heap
            .write_i32(definitions_table + 0x19E * 0x78 + 0x68, 1);
        stub
    });

    fixture
        .module
        .write_u64_at_rva(CSTRATEGIC_RESOURCE_DATABASE_PTR_RVA, a(db));
    fixture
        .module
        .write_bytes_at_rva(PDX_LEXER_INITIALIZED_RVA, &[1u8]);
    fixture
        .module
        .write_u64_at_rva(PDX_LEXER_TOKEN_NAME_TABLE_PTR_RVA, a(token_table));
    fixture
        .module
        .write_bytes_at_rva(PDX_LEXER_MAX_TOKEN_ID_RVA, &(r as u32).to_le_bytes());
    fixture.module.write_bytes_at_rva(
        PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA,
        &((r + 1) as u32).to_le_bytes(),
    );
    fixture
        .module
        .write_u64_at_rva(MODIFIER_DEFINITION_TABLE_RVA, a(definitions_table));
    fixture
        .module
        .write_u64_at_rva(OIL_CURRENT_MAX_GLOBAL_AIR_MISSION_COSTS_RVA, a(mission_costs));
    for (rva, value) in [
        (OIL_CURRENT_MAX_GLOBAL_UNIT_A_RVA, 60_000u64),
        (OIL_CURRENT_MAX_GLOBAL_UNIT_B_RVA, 40_000),
        (OIL_CURRENT_MAX_GLOBAL_UNIT_C_RVA, 100_000),
        (OIL_CURRENT_MAX_GLOBAL_NAVAL_A_RVA, 200_000),
        (OIL_CURRENT_MAX_GLOBAL_NAVAL_B_RVA, 50_000),
        (OIL_CURRENT_MAX_GLOBAL_AIR_WEIGHT_RVA, 300_000),
    ] {
        fixture.module.write_u64_at_rva(rva, value);
    }

    let rows = countries
        .iter()
        .enumerate()
        .map(|(index, country)| {
            test_country_row(
                &format!("C{:03}", index + 1),
                (index + 1) as u32,
                heap_base + country.object,
            )
        })
        .collect();
    fixture.set_verified_country_roster("gamestate_country_table_verified", country_vtable, rows);

    World {
        fixture,
        bump,
        heap_base,
        shape,
        gs,
        countries,
        routes,
        contracts,
        contributing_wings,
        engine_stub,
    }
}

/// Consecutive hours over one world, through the production frame builder,
/// with the trackers and the delta consumer carried frame to frame.
struct Session {
    world: World,
    trackers: Option<CompactPerfGateTrackers>,
    consumer: PayloadSpanDeltaConsumer,
    next_hour: i32,
    last_hour: i32,
}

impl Session {
    fn new(world: World) -> Self {
        Self {
            world,
            trackers: None,
            consumer: PayloadSpanDeltaConsumer::new(),
            next_hour: fixture_game_hour_raw(),
            last_hour: fixture_game_hour_raw(),
        }
    }

    fn with_limits(world: World, limits: Domain43SnapshotLimits) -> Self {
        let mut session = Self::new(world);
        let mut trackers = CompactPerfGateTrackers::default();
        trackers.market_trade_live_snapshot.limits = limits;
        session.trackers = Some(trackers);
        session
    }

    fn snapshot_tracker(&self) -> &CompactMarketTradeSnapshotTracker {
        &self
            .trackers
            .as_ref()
            .expect("a frame has run")
            .market_trade_live_snapshot
    }

    fn frame(&mut self) -> FrameOutcome {
        let hour = self.next_hour;
        self.next_hour += 1;
        self.last_hour = hour;
        self.world.set_hour(hour);
        let state = self.world.state();
        let trackers = self.trackers.take().unwrap_or_default();
        let mut outcome = drive_compact_frame_carrying(
            Some(&state),
            &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
            hour,
            ELAPSED_WALL_MS,
            trackers,
            &mut self.consumer,
            None,
        );
        self.trackers = Some(std::mem::take(&mut outcome.trackers));
        outcome
    }

    /// The same hour over the same world through fresh trackers: a plain live
    /// walk, published as a full baseline.
    fn live_reference(&self) -> FrameOutcome {
        let state = self.world.state();
        drive_compact_frame_carrying(
            Some(&state),
            &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
            self.last_hour,
            ELAPSED_WALL_MS,
            CompactPerfGateTrackers::default(),
            &mut PayloadSpanDeltaConsumer::new(),
            None,
        )
    }
}

fn payload(outcome: &FrameOutcome) -> Vec<u8> {
    outcome
        .domain_data(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 published a DATA record")
        .payload
        .clone()
}

fn unit(outcome: &FrameOutcome, name: &str) -> u64 {
    outcome
        .diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, name)
        .unwrap_or_else(|| panic!("d43 published {name}"))
}

fn status(outcome: &FrameOutcome) -> u32 {
    outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 published a PERF record")
        .status_flags
}

fn assert_clean(outcome: &FrameOutcome, what: &str) {
    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 published a PERF record");
    assert_eq!(
        perf.status_flags, 0,
        "{what}: d43 must be clean; diagnostics {:?}",
        perf.diagnostics
    );
}

fn assert_unavailable(outcome: &FrameOutcome, what: &str) {
    let flags = status(outcome);
    assert_eq!(
        flags & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
        "{what}: the hour must be sealed unavailable, status {flags:#x}"
    );
    assert_eq!(
        flags & compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        "{what}: the hour must be sealed partial, status {flags:#x}"
    );
}

/// Every d43 family, at the world's cardinalities, on the read the decode
/// produced. The equivalence tests compare two executions of one reader; this
/// is what says the reader published everything it should have.
fn assert_all_families(read: &CompactMarketTradeRead, shape: Shape, what: &str) {
    let n = shape.countries;
    assert_eq!(read.rows.len(), shape.contracts, "{what}: contract rows");
    assert_eq!(read.detail_rows.len(), shape.contracts, "{what}: detail rows");
    assert!(read.resource_flow_present, "{what}: resource flow present");
    assert_eq!(
        read.resource_flow_rows.len(),
        n * CCOUNTRY_RESOURCES_POOLS.len(),
        "{what}: resource flow rows"
    );
    assert!(read.factory_header_present, "{what}: factory header present");
    assert_eq!(read.factory_header_rows.len(), n, "{what}: factory header rows");
    assert!(read.resource_rows_present, "{what}: resource rows present");
    assert_eq!(read.resource_rows.len(), n * shape.resources, "{what}: resource rows");
    let oil_rows: Vec<_> = read
        .resource_rows
        .iter()
        .filter(|row| row.resource_index == 0)
        .collect();
    assert_eq!(oil_rows.len(), n, "{what}: one oil row per root");
    assert!(
        oil_rows
            .iter()
            .all(|row| row.needed_type == COMPACT_MARKET_TRADE_RESOURCE_ROW_NEEDED_CURRENT_MAX),
        "{what}: every oil trio derived"
    );
    assert_eq!(read.resource_tokens.len(), shape.resources, "{what}: tokens");
    assert!(read.classic_deals_present, "{what}: classic deals present");
    assert_eq!(read.classic_deal_rows.len(), 2 * shape.routes, "{what}: classic deal rows");
    assert_eq!(read.classic_deal_country_witnesses.len(), n, "{what}: witnesses");
    assert!(read.classic_deal_incomplete_owners.is_empty(), "{what}: every owner exact");
    assert!(read.stockpile_present, "{what}: stockpile present");
    assert_eq!(read.stockpile_countries.len(), n, "{what}: stockpile countries");
    assert_eq!(read.stockpile_items.len(), shape.stockpile_items, "{what}: stockpile items");
}

/// The DATA payload carries every family's trailer.
fn assert_payload_carries_every_family(payload: &[u8]) {
    for (name, magic) in [
        ("detail", COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC),
        ("resource flow", COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC),
        ("factory header", COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC),
        ("resource rows", COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC),
        ("classic deals", COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC),
        ("stockpile", COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_MAGIC),
    ] {
        assert!(
            payload.windows(4).any(|window| window == magic),
            "the DATA payload lost the {name} trailer"
        );
    }
}

fn assert_mode(outcome: &FrameOutcome, mode: u64, what: &str) {
    assert_eq!(unit(outcome, "d43_snapshot_mode"), mode, "{what}: snapshot mode");
}

/// The world's cardinalities as the walk counted them, on every mode.
fn assert_world_counts(outcome: &FrameOutcome, shape: Shape, what: &str) {
    assert_eq!(unit(outcome, "d43_oil_roots_walked"), shape.countries as u64, "{what}: roots");
    assert_eq!(
        unit(outcome, "d43_oil_wings_visited"),
        2 * shape.wings_per_arm as u64,
        "{what}: wings"
    );
    assert_eq!(
        unit(outcome, "d43_oil_wings_idle_path"),
        2 * shape.idle_wings_per_arm as u64,
        "{what}: idle wings"
    );
    assert_eq!(
        outcome
            .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
            .expect("perf")
            .row_count,
        shape.contracts as u32,
        "{what}: contract rows"
    );
}

/// kills: a decode that reads anything the bracket did not copy, a copy that
/// misses anything the walk reads, a plan that stops covering the walk after
/// its first use, and any drift between the three modes' payloads.
#[test]
fn d43_snapshot_frames_publish_the_live_payload_byte_for_byte() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut session = Session::new(build_world(P4_SHAPE));

    let live = session.frame();
    assert_mode(&live, D43_SNAPSHOT_MODE_LIVE, "first frame");
    assert_clean(&live, "first frame");
    assert_world_counts(&live, P4_SHAPE, "live");
    assert_eq!(payload(&live), payload(&session.live_reference()));
    assert_payload_carries_every_family(&payload(&live));
    assert!(unit(&live, "d43_snapshot_trace_capacity") > 0, "a trace buffer is prepared");

    session.world.mutate(1);
    let arming = session.frame();
    assert_mode(&arming, D43_SNAPSHOT_MODE_ARMING, "arming frame");
    assert_clean(&arming, "arming frame");
    assert_world_counts(&arming, P4_SHAPE, "arming");
    assert_eq!(payload(&arming), payload(&session.live_reference()));
    assert!(unit(&arming, "d43_snapshot_arm_trace_reads") > 0);
    assert_eq!(unit(&arming, "d43_snapshot_plan_refused"), 0);
    let region_count = unit(&arming, "d43_snapshot_region_count");
    let byte_count = unit(&arming, "d43_snapshot_byte_count");
    assert!(region_count > 0 && byte_count > 0, "a plan was armed");
    assert!(session.snapshot_tracker().plan().is_some());

    for round in 2..6 {
        session.world.mutate(round);
        let snapshot = session.frame();
        let what = format!("snapshot frame {round}");
        assert_mode(&snapshot, D43_SNAPSHOT_MODE_SNAPSHOT, &what);
        assert_clean(&snapshot, &what);
        assert_world_counts(&snapshot, P4_SHAPE, &what);
        assert_eq!(unit(&snapshot, "d43_snapshot_region_count"), region_count);
        assert_eq!(unit(&snapshot, "d43_snapshot_copied_region_count"), region_count);
        assert_eq!(unit(&snapshot, "d43_snapshot_byte_count"), byte_count);
        assert_eq!(unit(&snapshot, "d43_snapshot_copied_byte_count"), byte_count);
        assert_eq!(unit(&snapshot, "d43_snapshot_region_miss"), 0);
        assert_eq!(unit(&snapshot, "d43_snapshot_copy_refused"), 0);
        assert_eq!(unit(&snapshot, "d43_snapshot_engine_call_mismatch"), 0);
        let reference = session.live_reference();
        assert_eq!(payload(&snapshot), payload(&reference), "{what}: byte-identical to live");
        assert_payload_carries_every_family(&payload(&snapshot));
        assert_ne!(payload(&snapshot), payload(&live), "{what}: the mutation reached the wire");
        assert_eq!(unit(&snapshot, "d43_snapshot_probe_regions"), 0);
        assert_eq!(unit(&snapshot, "d43_snapshot_gate_budget_exhausted"), 0);
        assert!(unit(&snapshot, "d43_snapshot_gate_queries") >= 1);
        assert!(unit(&snapshot, "d43_snapshot_os_regions") >= 1);
    }
}

/// kills: any read in the decode that reaches live memory instead of the
/// copy. The copy is taken, the world moves, the decode runs: it must publish
/// the world AS COPIED.
#[test]
fn d43_snapshot_decode_serves_the_copy_and_never_the_live_word() {
    let _rc_counters = super::super::lock_rc_counters();
    let mut world = build_world(P4_SHAPE);
    let state = world.state();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        let captured = capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        );
        finish_compact_market_trade_snapshot(captured)
    };
    let live = read(&mut trackers);
    assert_eq!(live.snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    assert_eq!(live.status_flags, 0);
    assert_all_families(&live, P4_SHAPE, "live");
    let arming = read(&mut trackers);
    assert_eq!(arming.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(arming.status_flags, 0);
    assert_all_families(&arming, P4_SHAPE, "arming");
    let before = compact_market_trade_data_payload(&arming, ELAPSED_WALL_MS);
    assert_eq!(compact_market_trade_data_payload(&live, ELAPSED_WALL_MS), before);

    page_cache_new_frame();
    let captured = capture_compact_market_trade_snapshot(
        &state,
        0,
        &mut trackers.market_trade_live,
        &mut trackers.market_trade_live_snapshot,
    );
    world.mutate(7);
    let decoded = finish_compact_market_trade_snapshot(captured);
    assert_eq!(decoded.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(decoded.status_flags, 0);
    assert_all_families(&decoded, P4_SHAPE, "decoded");
    assert_eq!(
        compact_market_trade_data_payload(&decoded, ELAPSED_WALL_MS),
        before,
        "the decode published the world as copied, not as it is now"
    );

    let mut fresh = CompactPerfGateTrackers::default();
    page_cache_new_frame();
    let reference = finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
        &state,
        0,
        &mut fresh.market_trade_live,
        &mut fresh.market_trade_live_snapshot,
    ));
    let after = compact_market_trade_data_payload(&reference, ELAPSED_WALL_MS);
    assert_ne!(after, before, "the mutation is visible to a live walk");
    let next = read(&mut trackers);
    assert_eq!(next.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(compact_market_trade_data_payload(&next, ELAPSED_WALL_MS), after);
}

/// kills: serving a read the plan does not cover from anywhere, publishing
/// the hour a miss was found in, and not re-arming after it.
#[test]
fn d43_snapshot_region_miss_seals_the_hour_by_name_and_re_arms() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut session = Session::new(build_world(SMALL_SHAPE));
    assert_mode(&session.frame(), D43_SNAPSHOT_MODE_LIVE, "live");
    assert_mode(&session.frame(), D43_SNAPSHOT_MODE_ARMING, "arming");
    let armed = session.frame();
    assert_mode(&armed, D43_SNAPSHOT_MODE_SNAPSHOT, "snapshot");
    assert_clean(&armed, "snapshot");

    session.world.grow_wings(3);
    let missed = session.frame();
    assert_mode(&missed, D43_SNAPSHOT_MODE_SNAPSHOT, "miss frame");
    assert_eq!(unit(&missed, "d43_snapshot_region_miss"), 1);
    assert_eq!(unit(&missed, "d43_snapshot_copy_refused"), 0);
    assert_unavailable(&missed, "miss frame");
    assert_eq!(
        missed
            .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
            .expect("perf")
            .row_count,
        0,
        "a missed hour publishes no rows"
    );
    assert!(session.snapshot_tracker().plan().is_none(), "the plan is dropped");

    let rearmed = session.frame();
    assert_mode(&rearmed, D43_SNAPSHOT_MODE_ARMING, "re-arm");
    assert_clean(&rearmed, "re-arm");
    let grown = Shape {
        wings_per_arm: SMALL_SHAPE.wings_per_arm + 1,
        ..SMALL_SHAPE
    };
    assert_world_counts(&rearmed, grown, "re-arm");
    let recovered = session.frame();
    assert_mode(&recovered, D43_SNAPSHOT_MODE_SNAPSHOT, "recovered");
    assert_clean(&recovered, "recovered");
    assert_world_counts(&recovered, grown, "recovered");
    assert_eq!(payload(&recovered), payload(&session.live_reference()));
}

/// kills: a cap that truncates instead of refusing, a refusal that is not
/// sticky, and a refused plan that degrades the hour (it must not: the live
/// walk still runs).
#[test]
fn d43_snapshot_cap_refusals_are_sticky_named_and_keep_the_live_walk() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    for (cause, limits) in [
        (
            D43_SNAPSHOT_REGION_CAP_CAUSE,
            Domain43SnapshotLimits {
                max_regions: 8,
                ..Domain43SnapshotLimits::default()
            },
        ),
        (
            D43_SNAPSHOT_BYTE_CAP_CAUSE,
            Domain43SnapshotLimits {
                max_bytes: 64,
                ..Domain43SnapshotLimits::default()
            },
        ),
        (
            D43_SNAPSHOT_TRACE_CAP_CAUSE,
            Domain43SnapshotLimits {
                trace_reserve_reads: 64,
                max_trace_reads: 64,
                ..Domain43SnapshotLimits::default()
            },
        ),
        (
            D43_SNAPSHOT_OS_REGION_CAP_CAUSE,
            Domain43SnapshotLimits {
                max_os_regions: 0,
                ..Domain43SnapshotLimits::default()
            },
        ),
    ] {
        let mut session = Session::with_limits(build_world(SMALL_SHAPE), limits);
        let live = session.frame();
        assert_mode(&live, D43_SNAPSHOT_MODE_LIVE, cause);
        let arming = session.frame();
        assert_mode(&arming, D43_SNAPSHOT_MODE_ARMING, cause);
        assert_clean(&arming, cause);
        assert_eq!(unit(&arming, "d43_snapshot_plan_refused"), 1, "{cause}");
        assert_eq!(
            unit(&arming, "d43_snapshot_plan_refusal_cause"),
            plan_refusal_code(cause),
            "{cause}"
        );
        assert_eq!(session.snapshot_tracker().refusal(), Some(cause));
        if cause != D43_SNAPSHOT_TRACE_CAP_CAUSE {
            assert!(
                unit(&arming, "d43_snapshot_region_count") > 8
                    && unit(&arming, "d43_snapshot_byte_count") > 64,
                "{cause}: the refused arming frame publishes the attempted demand"
            );
        }
        for round in 0..3 {
            let later = session.frame();
            let what = format!("{cause} round {round}");
            assert_mode(&later, D43_SNAPSHOT_MODE_LIVE, &what);
            assert_clean(&later, &what);
            assert_world_counts(&later, SMALL_SHAPE, &what);
            assert_eq!(unit(&later, "d43_snapshot_plan_refused"), 1, "{what}");
            assert_eq!(
                unit(&later, "d43_snapshot_plan_refusal_cause"),
                plan_refusal_code(cause),
                "{what}"
            );
            assert_eq!(payload(&later), payload(&session.live_reference()), "{what}");
        }
    }
}

/// kills: a trace overflow that refuses for good while the ceiling still has
/// room, and a re-arm that does not grow the buffer.
#[test]
fn d43_snapshot_trace_overflow_doubles_the_buffer_up_to_the_ceiling() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut session = Session::with_limits(
        build_world(SMALL_SHAPE),
        Domain43SnapshotLimits {
            trace_reserve_reads: 256,
            ..Domain43SnapshotLimits::default()
        },
    );
    assert_mode(&session.frame(), D43_SNAPSHOT_MODE_LIVE, "live");
    let mut capacity = 256u64;
    let mut armed_at = None;
    for round in 0..16 {
        let frame = session.frame();
        let what = format!("round {round}");
        assert_clean(&frame, &what);
        if unit(&frame, "d43_snapshot_mode") == D43_SNAPSHOT_MODE_SNAPSHOT {
            armed_at = Some(round);
            break;
        }
        assert_mode(&frame, D43_SNAPSHOT_MODE_ARMING, &what);
        assert_eq!(unit(&frame, "d43_snapshot_trace_capacity"), capacity, "{what}");
        if unit(&frame, "d43_snapshot_plan_refused") == 1 {
            assert_eq!(
                unit(&frame, "d43_snapshot_plan_refusal_cause"),
                plan_refusal_code(D43_SNAPSHOT_TRACE_CAP_CAUSE),
                "{what}"
            );
            assert_eq!(unit(&frame, "d43_snapshot_arm_trace_reads"), capacity, "{what}: full");
            capacity *= 2;
        }
    }
    let armed_at = armed_at.expect("the buffer reached the walk's demand");
    assert!(armed_at >= 2, "256 reads cannot hold this walk; the buffer had to grow");
    assert!(capacity > 256);
    assert_eq!(session.snapshot_tracker().refusal(), None);
}

/// kills: a copy failure that publishes the arena as if it were copied, and a
/// copy failure that keeps the plan.
#[test]
fn d43_snapshot_copy_refusal_seals_the_hour_by_name_and_re_arms() {
    let _rc_counters = super::super::lock_rc_counters();
    let world = build_world(SMALL_SHAPE);
    let state = world.state();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        ))
    };
    assert_eq!(read(&mut trackers).snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    assert_eq!(read(&mut trackers).snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert!(trackers.market_trade_live_snapshot.plan().is_some());

    // A plan with one region on the null page: readable to no one.
    let mut trace = Domain43AccessTrace::with_capacity(4);
    trace.push(Domain43ReadTrace {
        address: 0x10,
        len: 8,
        succeeded: true,
    });
    let poisoned = Domain43SnapshotPlan::from_trace(trace, Domain43SnapshotLimits::default())
        .expect("one region plans");
    trackers.market_trade_live_snapshot.install_plan(poisoned);

    let refused = read(&mut trackers);
    assert_eq!(refused.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(refused.snapshot.copy_refused, 1);
    assert_eq!(refused.snapshot.region_count, 1);
    assert_eq!(refused.snapshot.copied_region_count, 0);
    assert_eq!(
        refused.status_flags,
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
            | compact_contract::COMPACT_STATUS_PARTIAL_DATA
    );
    assert!(refused.rows.is_empty() && refused.resource_rows.is_empty());
    let reason = compact_market_trade_failure_reason(&refused);
    assert!(
        reason.contains("snapshot_copy_refused"),
        "the reason names the copy refusal: {reason}"
    );
    assert!(trackers.market_trade_live_snapshot.plan().is_none());

    let rearmed = read(&mut trackers);
    assert_eq!(rearmed.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(rearmed.status_flags, 0);
    let recovered = read(&mut trackers);
    assert_eq!(recovered.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(recovered.status_flags, 0);
    assert_eq!(
        compact_market_trade_data_payload(&recovered, ELAPSED_WALL_MS),
        compact_market_trade_data_payload(&rearmed, ELAPSED_WALL_MS)
    );
}

extern "system" fn probe_callback() -> u32 {
    0xAB
}

/// kills: serving the live call under a snapshot decode, serving a captured
/// mask for a different callback, and not counting either mismatch.
#[test]
fn d43_engine_call_seam_serves_the_captured_mask_and_counts_mismatches() {
    let callback = probe_callback as *const () as usize;
    assert_eq!(domain43_eligibility_mask(callback), Some(0xAB), "live: the call itself");
    let captured = Domain43EngineCallCapture {
        callback,
        mask: 0x5,
    };
    let (served, mismatches) = with_engine_call_override(Some(captured), || {
        (
            domain43_eligibility_mask(callback),
            domain43_eligibility_mask(callback + 16),
        )
    });
    assert_eq!(served, (Some(0x5), None));
    assert_eq!(mismatches, 1);
    let (served, mismatches) =
        with_engine_call_override(None, || domain43_eligibility_mask(callback));
    assert_eq!(served, None, "a call the bracket did not capture is refused");
    assert_eq!(mismatches, 1);
    assert_eq!(
        domain43_eligibility_mask(callback),
        Some(0xAB),
        "the override is scoped"
    );
}

fn bracket_source() -> String {
    include_str!("../d43_snapshot_bracket.rs").replace("\r\n", "\n")
}

/// A method inside an `impl`: from its signature to its four-space brace.
fn impl_method_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start..]
        .find("\n    }\n")
        .map(|offset| start + offset + 7)
        .expect("a method closes on a four-space brace");
    &source[start..end]
}

fn fn_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start..]
        .find("\n}\n")
        .map(|offset| start + offset + 3)
        .unwrap_or(source.len());
    &source[start..end]
}

/// The bracket half copies and calls nothing else; the decode walks the copy
/// through the override and nothing else; the frame builder reaches d43 only
/// through the finishing spawn.
#[test]
fn d43_snapshot_bracket_copies_and_the_decode_walks_the_copy() {
    let source = bracket_source();
    let capture = fn_body(&source, "fn capture_from_plan(");
    for forbidden in [
        "read_compact_market_trade(",
        "Vec::",
        "vec!",
        "Box::",
        ".collect(",
        "String",
        ".clone()",
        "to_vec(",
        "prepare_trace_buffer",
        "from_trace(",
    ] {
        assert!(
            !capture.contains(forbidden),
            "the bracket half must not contain {forbidden:?}"
        );
    }
    // The ARMING walk records its reads in-bracket: a push within capacity
    // and nothing that can allocate.
    let memory = include_str!("../d43_snapshot_memory.rs").replace("\r\n", "\n");
    let method_body = |signature: &str| -> &str {
        let start = memory
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} must exist"));
        let end = memory[start..]
            .find("\n    }\n")
            .map(|offset| start + offset + 7)
            .expect("a method closes on a four-space brace");
        &memory[start..end]
    };
    let note = method_body("    fn note(");
    let push = method_body("    pub fn push(");
    for body in [note, push] {
        for forbidden in ["insert(", "Vec::", "vec!", "Box::", "reserve", "extend("] {
            assert!(!body.contains(forbidden), "the trace recorder must not contain {forbidden:?}");
        }
    }
    assert!(push.contains("self.reads.len() == self.reads.capacity()"));
    assert!(capture.contains("plan.capture_with("));
    // The engine-call capture reads the slot and the module header from the
    // COPY: the live read path's span cache can grow inside the bracket.
    assert!(capture.contains("capture_engine_call(state, &plan.view())"));
    let engine = fn_body(&source, "fn capture_engine_call(");
    let seam = engine
        .find("with_d81_memory_override(view")
        .expect("the engine-call capture reads under the view");
    let closure_end = engine[seam..]
        .find("})?;")
        .map(|offset| seam + offset)
        .expect("the override closure closes");
    assert!(!engine[..seam].contains("read_"), "no live read before the view");
    assert!(!engine[closure_end..].contains("read_"), "no live read after the view");
    for forbidden in ["range_confirmed_good(", "D81LiveMemory", "Vec::", "vec!", "Box::"] {
        assert!(
            !engine.contains(forbidden),
            "the engine-call capture must not contain {forbidden:?}"
        );
    }
    assert!(capture.contains("gate.confirm("));
    assert!(capture.contains("seh_read_exact("));
    assert!(
        !capture.contains("range_confirmed_good("),
        "the frame-epoch span cache can grow inside the bracket; the copy gates with its own"
    );
    let confirm = impl_method_body(&source, "    pub(super) fn confirm(");
    for forbidden in [
        "Vec::",
        "vec!",
        "Box::",
        "RefCell",
        ".with(",
        "insert(",
        "range_confirmed_good(",
        "String",
        "rc_bump",
    ] {
        assert!(!confirm.contains(forbidden), "the gate must not contain {forbidden:?}");
    }
    assert!(confirm.contains("query_region("));
    assert!(confirm.contains("self.queries >= self.budget"));
    assert!(confirm.contains("PAGE_GUARD"));

    let bracket = fn_body(&source, "pub(super) fn capture_compact_market_trade_snapshot<");
    let plan_arm = bracket
        .find("if let Some(plan) = snapshot.plan.take()")
        .expect("the plan arm");
    let plan_arm_end = bracket[plan_arm..]
        .find("} else if")
        .map(|offset| plan_arm + offset)
        .expect("the plan arm closes");
    let plan_arm_body = &bracket[plan_arm..plan_arm_end];
    assert!(plan_arm_body.contains("capture_from_plan("));
    assert!(!plan_arm_body.contains("read_compact_market_trade("));

    let decode = fn_body(&source, "fn decode_from_plan(");
    assert_eq!(
        decode.matches("read_compact_market_trade(").count(),
        1,
        "the decode walks exactly once"
    );
    let view = decode.find("with_d81_memory_override(&view").expect("the override");
    let walk = decode.find("read_compact_market_trade(").expect("the walk");
    assert!(view < walk, "the walk runs under the snapshot view");
    assert!(decode.contains("view.first_miss()"));
    assert!(decode.contains("domain43_degraded_cause::SNAPSHOT_REGION_MISS"));
    assert!(decode.contains("domain43_degraded_cause::SNAPSHOT_COPY_REFUSED"));
    assert!(decode.contains("domain43_degraded_cause::SNAPSHOT_ENGINE_CALL_MISMATCH"));
    assert_eq!(
        decode.matches("domain43_snapshot_unavailable_read(").count(),
        3,
        "copy refusal, region miss and engine-call mismatch each seal the hour unavailable"
    );
    assert!(
        !decode.contains("read.status_flags |="),
        "no snapshot failure publishes rows under a status bit; each seals"
    );

    let attach_log = super::collector_source_lf();
    // The executable check the capture calls reads through the override too:
    // nothing in it may pick a memory of its own.
    let executable = fn_body(
        &attach_log,
        "fn compact_market_trade_module_executable_address(",
    );
    for forbidden in [
        "with_d81_memory_override(",
        "D81LiveMemory",
        "range_confirmed_good(",
        "seh_read_exact(",
        "copy_selected(",
    ] {
        assert!(
            !executable.contains(forbidden),
            "the executable check must not contain {forbidden:?}"
        );
    }
    assert!(executable.contains("read_u16(") && executable.contains("read_u32("));
    // Nothing in the bracket module flushes this thread's read counters, so
    // the footprint probe's window sees every live-path read it makes.
    for forbidden in ["RcDomainScope", "rc_flush_current_thread", "rc_take_and_reset"] {
        assert!(!source.contains(forbidden), "the bracket module must not contain {forbidden:?}");
    }
    let call_site = attach_log
        .find("d43_snapshot_bracket::capture_compact_market_trade_snapshot(")
        .expect("the frame builder captures through the bracket");
    let window = &attach_log[call_site.saturating_sub(600)..call_site];
    assert!(window.contains("spawn_gated_finishing("));
    assert!(window.contains("COMPACT_DOMAIN_MARKET_TRADE_LIVE,"));
    assert!(attach_log.contains("d43_snapshot_bracket::finish_compact_market_trade_snapshot,"));
    assert_eq!(
        attach_log.matches("read_compact_market_trade(").count(),
        1,
        "attach_log.rs calls the walk nowhere: the bracket module owns every call"
    );
}

/// The receipt units ride every frame, in the bracket module's order.
#[test]
fn d43_snapshot_units_ride_every_frame_in_order() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut session = Session::new(build_world(SMALL_SHAPE));
    let frame = session.frame();
    let names: Vec<String> = frame
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("perf")
        .diagnostics
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let expected = crate::attach_log::d43_snapshot_bracket::D43_SNAPSHOT_UNIT_NAMES;
    assert_eq!(&names[names.len() - expected.len()..], &expected[..]);
    frame.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_MARKET_TRADE_LIVE);
}

/// kills: the plan replaying an arming-hour failure for ever, a copy that
/// fails a probe and refuses the frame, and a view that misses on a read
/// across a plan-time cut. Country 2's stockpile blob (two items, 32 bytes) is
/// staged across the boundary between a committed page and a reserved one: its
/// read fails live, becomes one probe the plan cuts at the boundary, and
/// replays ACROSS the cut -- refused without a miss while the reserved side
/// stays unreadable, refused without a miss while the two sides are two
/// operating-system regions (the live gate's rule, both readable or not), and
/// served the hour they are one; the plan is kept throughout. The
/// healed hour is a MISS (the walk goes on into regions the plan never
/// covered), sealed by name, re-armed the next, and the plan after it carries
/// the healed read as data.
#[test]
fn d43_snapshot_probe_regions_replay_a_failed_read_and_serve_it_once_readable() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut session = Session::new(build_world(SMALL_SHAPE));
    let pages = reserve_pages_in_heap_range(2);
    commit_page(pages);
    let boundary = pages + PAGE;
    let country = &session.world.countries[1];
    let (pool, items) = (country.stockpile_pool, country.stockpile_items);
    let blob_len = 2 * CEQUIPMENT_POOL_ITEM_STRIDE;
    assert_eq!(share(SMALL_SHAPE.stockpile_items, SMALL_SHAPE.countries, 1), 2);
    let begin = boundary - CEQUIPMENT_POOL_ITEM_STRIDE;
    // The first item on the committed page, the second on the reserved one.
    place_heap_bytes(&session.world, items, CEQUIPMENT_POOL_ITEM_STRIDE, begin);
    session
        .world
        .fixture
        .heap
        .write_u64(pool + CEQUIPMENT_POOL_ITEMS_BEGIN_OFF, begin as u64);
    let partial = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
    let stockpile = COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_MAGIC;

    let live = session.frame();
    assert_mode(&live, D43_SNAPSHOT_MODE_LIVE, "live");
    assert_eq!(status(&live) & partial, partial, "the stockpile family is unavailable");
    assert!(!carries_trailer(&payload(&live), stockpile));
    let arming = session.frame();
    assert_mode(&arming, D43_SNAPSHOT_MODE_ARMING, "arming");
    assert_eq!(
        unit(&arming, "d43_snapshot_probe_regions"),
        1,
        "the blob read that failed (twice) is one probe"
    );
    let region_count = unit(&arming, "d43_snapshot_region_count");
    let trace_reads = unit(&arming, "d43_snapshot_arm_trace_reads");

    let probed = session.frame();
    assert_mode(&probed, D43_SNAPSHOT_MODE_SNAPSHOT, "probed");
    assert_eq!(unit(&probed, "d43_snapshot_probe_regions"), 1);
    assert_eq!(
        unit(&probed, "d43_snapshot_probe_unreadable"),
        1,
        "cut at the boundary: the committed side copied, the reserved side did not"
    );
    assert_eq!(
        unit(&probed, "d43_snapshot_region_miss"),
        0,
        "the 32-byte replay crosses the cut: refused whole, not missed"
    );
    assert_eq!(unit(&probed, "d43_snapshot_copy_refused"), 0);
    assert_eq!(status(&probed) & partial, partial);
    assert!(!carries_trailer(&payload(&probed), stockpile));
    assert_eq!(payload(&probed), payload(&session.live_reference()), "the live reader's answer");
    assert!(session.snapshot_tracker().plan().is_some(), "the plan is kept");

    let again = session.frame();
    assert_mode(&again, D43_SNAPSHOT_MODE_SNAPSHOT, "still unreadable");
    assert_eq!(unit(&again, "d43_snapshot_probe_unreadable"), 1);
    assert_eq!(unit(&again, "d43_snapshot_region_miss"), 0);
    assert_eq!(unit(&again, "d43_snapshot_region_count"), region_count);

    // The reserved page is committed READ-ONLY with the second item on it:
    // both sides copy, but they are two operating-system regions, and the
    // live gate refuses a range straddling two regions even when both are
    // readable (`range_confirmed_good`). So must the copy: refused, no miss,
    // the plan kept, the live reader's answer.
    commit_page(boundary);
    place_heap_bytes(&session.world, items, blob_len, begin);
    protect_page(boundary, PAGE_READONLY);
    let two_regions = session.frame();
    assert_mode(&two_regions, D43_SNAPSHOT_MODE_SNAPSHOT, "two regions");
    assert_eq!(unit(&two_regions, "d43_snapshot_probe_unreadable"), 0, "both sides copied");
    assert_eq!(unit(&two_regions, "d43_snapshot_region_miss"), 0);
    assert_eq!(status(&two_regions) & partial, partial);
    assert!(!carries_trailer(&payload(&two_regions), stockpile), "served by neither");
    assert_eq!(payload(&two_regions), payload(&session.live_reference()));
    assert!(session.snapshot_tracker().plan().is_some(), "the plan is kept");

    // One region again: the probe copies inside it, the decode serves the
    // current bytes, and the walk goes on past the read that used to refuse
    // -- into the items' variants, regions this plan never covered.
    protect_page(boundary, PAGE_READWRITE);
    let healed = session.frame();
    assert_mode(&healed, D43_SNAPSHOT_MODE_SNAPSHOT, "healed");
    assert_eq!(unit(&healed, "d43_snapshot_probe_regions"), 1);
    assert_eq!(unit(&healed, "d43_snapshot_probe_unreadable"), 0, "the probe copied");
    assert_eq!(unit(&healed, "d43_snapshot_region_miss"), 1, "the walk went further");
    assert_unavailable(&healed, "healed");
    assert!(session.snapshot_tracker().plan().is_none());
    let rearmed = session.frame();
    assert_mode(&rearmed, D43_SNAPSHOT_MODE_ARMING, "re-armed");
    assert_clean(&rearmed, "re-armed");
    assert_eq!(unit(&rearmed, "d43_snapshot_probe_regions"), 0, "nothing fails now");
    assert!(
        unit(&rearmed, "d43_snapshot_arm_trace_reads") > trace_reads,
        "the healed walk reads what the refusal cut short"
    );
    let served = session.frame();
    assert_mode(&served, D43_SNAPSHOT_MODE_SNAPSHOT, "served");
    assert_clean(&served, "served");
    assert_eq!(unit(&served, "d43_snapshot_region_miss"), 0);
    assert_eq!(payload(&served), payload(&session.live_reference()));
    assert_payload_carries_every_family(&payload(&served));
    assert_ne!(payload(&served), payload(&probed));
    release_pages(pages);
}

/// kills: `mismatches > 0` weakened -- a decode that reached a callback the
/// bracket did not capture publishing rows and keeping the plan. The world's
/// engine call is real (an executable stub the eligibility slot points at), so
/// the count comes from the decode's own walk.
#[test]
fn d43_snapshot_engine_call_mismatch_seals_the_hour_by_name_and_re_arms() {
    let _rc_counters = super::super::lock_rc_counters();
    let mut world = build_world_with_engine_callback(SMALL_SHAPE);
    let stub = world.engine_stub.expect("the stub is staged");
    // Countries 5 and 10 carry the 0x19E row; without country 10's the walk
    // reaches the call exactly once, so one mismatch is the whole count.
    let tenth = world.countries[9].object;
    world
        .fixture
        .heap
        .write_i32(tenth + CCOUNTRY_MODIFIER_CONTAINER_OFF + 0xC, 0);
    let state = world.state();
    assert_eq!(domain43_eligibility_mask(stub), Some(ENGINE_STUB_MASK), "the stub runs");
    fn capture<'t>(
        state: &'t CollectorState,
        trackers: &'t mut CompactPerfGateTrackers,
    ) -> Domain43CapturedRead<'t> {
        page_cache_new_frame();
        capture_compact_market_trade_snapshot(
            state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        )
    }
    let mut trackers = CompactPerfGateTrackers::default();
    let live = finish_compact_market_trade_snapshot(capture(&state, &mut trackers));
    assert_eq!(live.snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    assert_eq!(live.status_flags, 0);
    assert_all_families(&live, SMALL_SHAPE, "live");
    let arming = finish_compact_market_trade_snapshot(capture(&state, &mut trackers));
    assert_eq!(arming.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(arming.status_flags, 0);
    let reference = compact_market_trade_data_payload(&arming, ELAPSED_WALL_MS);

    let captured = capture(&state, &mut trackers);
    assert_eq!(
        captured.engine_call(),
        Some(Some(Domain43EngineCallCapture {
            callback: stub,
            mask: ENGINE_STUB_MASK,
        })),
        "the bracket captured the stub's answer"
    );
    let served = finish_compact_market_trade_snapshot(captured);
    assert_eq!(served.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(served.status_flags, 0);
    assert_eq!(served.snapshot.engine_call_mismatch, 0);
    assert_eq!(compact_market_trade_data_payload(&served, ELAPSED_WALL_MS), reference);

    for (label, bracket_saw) in [
        (
            "another slot",
            Some(Domain43EngineCallCapture {
                callback: stub + 16,
                mask: ENGINE_STUB_MASK,
            }),
        ),
        ("no call", None),
    ] {
        let mut captured = capture(&state, &mut trackers);
        assert!(captured.override_engine_call(bracket_saw), "{label}: a snapshot arm");
        let sealed = finish_compact_market_trade_snapshot(captured);
        assert_eq!(sealed.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT, "{label}");
        assert_eq!(
            sealed.snapshot.engine_call_mismatch, 1,
            "{label}: the decode reached the call exactly once, so `>= 2` cannot pass"
        );
        assert_eq!(sealed.snapshot.region_miss, 0, "{label}");
        assert_eq!(
            sealed.status_flags,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                | compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            "{label}: sealed"
        );
        assert!(sealed.rows.is_empty() && sealed.resource_rows.is_empty(), "{label}: no rows");
        let reason = compact_market_trade_failure_reason(&sealed);
        assert!(reason.contains("snapshot_engine_call_mismatch"), "{label}: {reason}");
        assert!(
            trackers.market_trade_live_snapshot.plan().is_none(),
            "{label}: the plan is dropped"
        );
        let rearmed = finish_compact_market_trade_snapshot(capture(&state, &mut trackers));
        assert_eq!(rearmed.snapshot.mode, D43_SNAPSHOT_MODE_ARMING, "{label}");
        assert_eq!(rearmed.status_flags, 0, "{label}");
    }
    let recovered = finish_compact_market_trade_snapshot(capture(&state, &mut trackers));
    assert_eq!(recovered.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(recovered.status_flags, 0);
    assert_eq!(recovered.snapshot.engine_call_mismatch, 0);
    assert_eq!(compact_market_trade_data_payload(&recovered, ELAPSED_WALL_MS), reference);
}

/// kills: the bracket reading the slot and the module header LIVE (the live
/// read path's span cache can grow inside the bracket). With no country row
/// asking for eligibility the walk never reads the slot, the plan does not
/// cover it, and the bracket -- reading the copy -- captures no call, though
/// the slot is live, nonzero and executable.
#[test]
fn d43_snapshot_engine_call_capture_reads_the_copy_not_live_memory() {
    let _rc_counters = super::super::lock_rc_counters();
    let mut world = build_world_with_engine_callback(SMALL_SHAPE);
    let stub = world.engine_stub.expect("the stub is staged");
    for index in [4, 9] {
        let object = world.countries[index].object;
        world
            .fixture
            .heap
            .write_i32(object + CCOUNTRY_MODIFIER_CONTAINER_OFF + 0xC, 0);
    }
    let state = world.state();
    assert_eq!(domain43_eligibility_mask(stub), Some(ENGINE_STUB_MASK), "the slot is live");
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        ))
    };
    assert_eq!(read(&mut trackers).snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    let arming = read(&mut trackers);
    assert_eq!(arming.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(arming.status_flags, 0);
    page_cache_new_frame();
    let captured = capture_compact_market_trade_snapshot(
        &state,
        0,
        &mut trackers.market_trade_live,
        &mut trackers.market_trade_live_snapshot,
    );
    assert_eq!(
        captured.engine_call(),
        Some(None),
        "the slot is not in the plan: no call captured"
    );
    let served = finish_compact_market_trade_snapshot(captured);
    assert_eq!(served.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(served.status_flags, 0);
    assert_eq!(served.snapshot.engine_call_mismatch, 0);
    assert_eq!(
        compact_market_trade_data_payload(&served, ELAPSED_WALL_MS),
        compact_market_trade_data_payload(&arming, ELAPSED_WALL_MS)
    );
}

/// kills: the module header read LIVE by the executable check inside the
/// bracket (the slot alone from the copy). The arming walk reads a ZERO slot,
/// so the plan covers the slot and not the header; the slot then names the
/// stub before the copy. A copy-only capture sees the stub in the copied slot,
/// misses the header on the copy and captures NO call; a live header read
/// would capture the stub. The decode over the same copy misses the header
/// too: sealed by name, re-armed, and the plan after it carries the header.
#[test]
fn d43_snapshot_engine_call_capture_reads_the_module_header_from_the_copy() {
    let _rc_counters = super::super::lock_rc_counters();
    let mut world = build_world_with_engine_callback(SMALL_SHAPE);
    let stub = world.engine_stub.expect("the stub is staged");
    world
        .fixture
        .module
        .write_u64_at_rva(MODIFIER_ELIGIBILITY_CALLBACK_RVA, 0);
    let state = world.state();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        ))
    };
    assert_eq!(read(&mut trackers).snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    let arming = read(&mut trackers);
    assert_eq!(arming.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(arming.status_flags, 0);

    // The slot names the stub now; the header was never read, so the plan
    // does not cover it.
    world
        .fixture
        .module
        .write_u64_at_rva(MODIFIER_ELIGIBILITY_CALLBACK_RVA, stub as u64);
    page_cache_new_frame();
    let captured = capture_compact_market_trade_snapshot(
        &state,
        0,
        &mut trackers.market_trade_live,
        &mut trackers.market_trade_live_snapshot,
    );
    assert_eq!(
        captured.engine_call(),
        Some(None),
        "the copied slot names the stub but the header is not in the plan: no call"
    );
    let sealed = finish_compact_market_trade_snapshot(captured);
    assert_eq!(sealed.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(
        sealed.snapshot.region_miss, 1,
        "the decode reaches the header the plan never covered"
    );
    assert_eq!(
        sealed.status_flags,
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
            | compact_contract::COMPACT_STATUS_PARTIAL_DATA
    );
    assert!(trackers.market_trade_live_snapshot.plan().is_none());

    let rearmed = read(&mut trackers);
    assert_eq!(rearmed.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(rearmed.status_flags, 0);
    page_cache_new_frame();
    let captured = capture_compact_market_trade_snapshot(
        &state,
        0,
        &mut trackers.market_trade_live,
        &mut trackers.market_trade_live_snapshot,
    );
    assert_eq!(
        captured.engine_call(),
        Some(Some(Domain43EngineCallCapture {
            callback: stub,
            mask: ENGINE_STUB_MASK,
        })),
        "the header is in the plan now"
    );
    let served = finish_compact_market_trade_snapshot(captured);
    assert_eq!(served.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(served.status_flags, 0);
    assert_eq!(served.snapshot.engine_call_mismatch, 0);
    assert_eq!(
        compact_market_trade_data_payload(&served, ELAPSED_WALL_MS),
        compact_market_trade_data_payload(&rearmed, ELAPSED_WALL_MS)
    );
}

/// What THIS thread did inside `body`: heap allocations (the crate's test
/// allocation counter), guarded live reads (`rc_read_probe`: every live
/// `read_scalar` / `read_bytes`) and region-gate answers
/// (`range_confirmed_good`: every `D81LiveMemory` read). The read counts are
/// this thread's cells PLUS whatever any flush inside the window moved to the
/// process-wide sink (a nested `RcDomainScope`, a `rc_flush_current_thread`
/// in any helper, however deep: a flush cannot hide a read, it can only move
/// it), and `probes` is `rc_read_probe`'s own free-running counter, which no
/// flush touches. Under nextest each test is its own process, so the sink is
/// this test's; under one shared process another thread's flush could only
/// inflate the counts (a false red), never lower them. The caller holds the
/// rc-counter lock.
fn bracket_footprint<R>(body: impl FnOnce() -> R) -> (R, usize, u64, u64, u64) {
    RC_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    rc_flush_current_thread();
    let _ = rc_take_and_reset();
    let probes_before = RC_LOCAL.with(|local| local.spl_ctr.get());
    let (out, allocations) = crate::test_allocation_counter::measure(body);
    let probes = RC_LOCAL.with(|local| local.spl_ctr.get()).wrapping_sub(probes_before);
    let (local_reads, local_gate) = RC_LOCAL.with(|local| {
        (
            local.v[RC_READS].get(),
            local.v[RC_REGION_HITS].get() + local.v[RC_REGION_MISSES].get(),
        )
    });
    let sink = rc_take_and_reset();
    RC_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    (
        out,
        allocations,
        local_reads + sink[RC_READS],
        local_gate + sink[RC_REGION_HITS] + sink[RC_REGION_MISSES],
        probes,
    )
}

/// kills: ANY heap allocation inside the bracket half, and any live read
/// through any helper under it -- the slot, the module header, a section or
/// vtable check added later, a scratch-growing scan. Not a source law over
/// named functions: the allocation counter observes the allocator and the read
/// counters observe the path every read actually took, wherever a flush moved
/// them. The snapshot capture reaches the slot, the header and the engine
/// call (the capture is `Some(Some(stub))`) with zero allocations and zero
/// live-path reads; the same probe sees a live walk allocate and read.
#[test]
fn d43_snapshot_capture_allocates_nothing_and_takes_the_live_read_path_zero_times() {
    let _rc_counters = super::super::lock_rc_counters();
    let world = build_world_with_engine_callback(SMALL_SHAPE);
    let stub = world.engine_stub.expect("the stub is staged");
    let state = world.state();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        ))
    };
    assert_eq!(read(&mut trackers).snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    let arming = read(&mut trackers);
    assert_eq!(arming.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(arming.status_flags, 0);

    // The probe sees a live walk.
    let mut fresh = CompactPerfGateTrackers::default();
    page_cache_new_frame();
    let (live, allocations, reads, gate, probes) = bracket_footprint(|| {
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut fresh.market_trade_live,
            &mut fresh.market_trade_live_snapshot,
        ))
    });
    assert_eq!(live.snapshot.mode, D43_SNAPSHOT_MODE_LIVE);
    assert!(
        allocations > 0 && reads > 0 && gate > 0 && probes > 0,
        "a live walk: {allocations} allocations, {reads} reads, {gate} gate answers, {probes} probes"
    );

    // The bracket half of a snapshot frame: the copy through the gate and
    // `seh_read_exact`, the slot and the header from the copy, the one call.
    page_cache_new_frame();
    let (captured, allocations, reads, gate, probes) = bracket_footprint(|| {
        capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        )
    });
    assert_eq!(
        captured.engine_call(),
        Some(Some(Domain43EngineCallCapture {
            callback: stub,
            mask: ENGINE_STUB_MASK,
        })),
        "the capture went through the slot, the header and the call"
    );
    assert_eq!(
        (allocations, reads, gate, probes),
        (0, 0, 0, 0),
        "the bracket half allocated or took the live read path"
    );
    let served = finish_compact_market_trade_snapshot(captured);
    assert_eq!(served.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
    assert_eq!(served.status_flags, 0);
    assert_eq!(
        compact_market_trade_data_payload(&served, ELAPSED_WALL_MS),
        compact_market_trade_data_payload(&arming, ELAPSED_WALL_MS)
    );
}

/// The production copy: the bracket's gate before the SEH copy.
fn gated_copy(gate: &mut Domain43RegionGate) -> impl FnMut(u64, &mut [u8]) -> Option<u32> + '_ {
    move |address, out| {
        let address = usize::try_from(address).ok()?;
        (gate.confirm(address, out.len())
            && unsafe { seh_read_exact(address, out.as_mut_ptr(), out.len()) })
        .then(|| gate.region_ordinal())
    }
}

/// `rounds` gated copies of `plan`, each with a fresh gate (as the bracket's
/// is): microseconds, sorted, and the queries one copy made.
fn copy_samples(plan: &mut Domain43SnapshotPlan, budget: u32, rounds: usize) -> (Vec<u64>, u32) {
    let mut samples = Vec::with_capacity(rounds);
    let mut queries = 0;
    for _ in 0..rounds {
        let mut gate = Domain43RegionGate::new(budget);
        let started = Instant::now();
        let receipt = plan.capture_with(gated_copy(&mut gate));
        samples.push(started.elapsed().as_micros() as u64);
        assert_eq!(receipt.failure_region_ordinal, None);
        assert!(!gate.exhausted());
        queries = gate.queries();
    }
    samples.sort_unstable();
    (samples, queries)
}

fn percentile_line(name: &str, regions: usize, bytes: usize, queries: u32, samples: &[u64]) -> String {
    let n = samples.len();
    let p95 = samples[((n * 95).div_ceil(100)).saturating_sub(1)];
    format!(
        "{{\"shape\":\"{name}\",\"regions\":{regions},\"bytes\":{bytes},\"gate_queries\":{queries},\"samples\":{n},\"p50_us\":{},\"p95_us\":{p95},\"max_us\":{}}}",
        samples[n / 2],
        samples[n - 1]
    )
}

fn uncapped_limits(merge_gap: u64) -> Domain43SnapshotLimits {
    Domain43SnapshotLimits {
        merge_gap,
        max_regions: usize::MAX,
        max_bytes: usize::MAX,
        ..Domain43SnapshotLimits::default()
    }
}

/// A plan of `regions` regions of `len` bytes each, `stride` apart, over a
/// private heap block: the worst legal plan at a cap, on the production copy.
fn synthetic_plan(regions: usize, len: usize, stride: usize) -> (Vec<u8>, Domain43SnapshotPlan) {
    let backing = vec![0x5Au8; regions * stride + stride];
    let base = (backing.as_ptr() as usize + 15) & !15;
    let mut trace = Domain43AccessTrace::with_capacity(regions);
    for index in 0..regions {
        trace.push(Domain43ReadTrace {
            address: (base + index * stride) as u64,
            len: len as u32,
            succeeded: true,
        });
    }
    let plan = Domain43SnapshotPlan::from_trace(trace, uncapped_limits(0)).expect("a synthetic plan");
    assert_eq!(plan.region_count(), regions);
    (backing, plan)
}

/// `count` pages, each its own reservation, so each is its own operating-
/// system region: the worst legal plan at the query budget.
struct SeparatePages(Vec<usize>);

impl SeparatePages {
    fn allocate(count: usize) -> Self {
        let mut pages = Vec::with_capacity(count);
        for _ in 0..count {
            let page = unsafe {
                VirtualAlloc(ptr::null_mut(), PAGE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
            };
            assert!(!page.is_null(), "a page");
            unsafe { ptr::write_bytes(page as *mut u8, 0x5A, PAGE) };
            pages.push(page as usize);
        }
        Self(pages)
    }

    fn plan(&self) -> Domain43SnapshotPlan {
        let mut trace = Domain43AccessTrace::with_capacity(self.0.len());
        for page in &self.0 {
            trace.push(Domain43ReadTrace {
                address: *page as u64,
                len: 8,
                succeeded: true,
            });
        }
        let plan = Domain43SnapshotPlan::from_trace(trace, uncapped_limits(0)).expect("a page plan");
        assert_eq!(plan.region_count(), self.0.len());
        plan
    }
}

impl Drop for SeparatePages {
    fn drop(&mut self) {
        for page in &self.0 {
            release_pages(*page);
        }
    }
}

/// Gated-copy cost receipt: the production bracket copy over the P4-shaped
/// plan (scattered layout) and over the worst LEGAL plans at the caps: every
/// region at the region cap, the byte cap in few regions, and one operating-
/// system region per region at the query budget. This machine, this build's
/// optimisation level, the fleet running: a bound on the bracket's copy, not
/// the P-run. Writes `runs/perf_d43_market/GATE_COST.json`.
#[test]
#[ignore]
fn d43_gated_copy_bench_receipt() {
    let _rc_counters = super::super::lock_rc_counters();
    let limits = Domain43SnapshotLimits::default();
    let mut lines = Vec::new();

    let world = build_world_with(P4_SHAPE, Layout::Scattered);
    let state = world.state();
    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        finish_compact_market_trade_snapshot(capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        ))
    };
    let _ = read(&mut trackers);
    let armed = read(&mut trackers);
    assert_eq!(armed.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(armed.snapshot.plan_refused, 0);
    let mut bracket_samples = Vec::new();
    let mut gate_queries = 0;
    for _ in 0..50 {
        let started = Instant::now();
        page_cache_new_frame();
        let captured = capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        );
        bracket_samples.push(started.elapsed().as_micros() as u64);
        let out = finish_compact_market_trade_snapshot(captured);
        assert_eq!(out.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
        assert_eq!(out.status_flags, 0);
        gate_queries = out.snapshot.gate_queries as u32;
    }
    bracket_samples.sort_unstable();
    lines.push(percentile_line(
        "p4_world_scattered_bracket_half",
        armed.snapshot.region_count as usize,
        armed.snapshot.byte_count as usize,
        gate_queries,
        &bracket_samples,
    ));

    // Eight bytes every 1024: every region on its own cache lines, four to a
    // page, as far from the friendly case as a private block gets.
    let (_region_backing, mut region_plan) = synthetic_plan(limits.max_regions, 8, 1024);
    let (samples, queries) = copy_samples(&mut region_plan, limits.max_os_regions, 20);
    lines.push(percentile_line(
        "region_cap_worst_legal",
        region_plan.region_count(),
        region_plan.byte_count(),
        queries,
        &samples,
    ));
    let byte_regions = 1024usize;
    let (_byte_backing, mut byte_plan) = synthetic_plan(
        byte_regions,
        limits.max_bytes / byte_regions - 16,
        limits.max_bytes / byte_regions,
    );
    let (samples, queries) = copy_samples(&mut byte_plan, limits.max_os_regions, 20);
    lines.push(percentile_line(
        "byte_cap_worst_legal",
        byte_plan.region_count(),
        byte_plan.byte_count(),
        queries,
        &samples,
    ));
    let pages = SeparatePages::allocate(limits.max_os_regions as usize);
    let mut page_plan = pages.plan();
    let (samples, queries) = copy_samples(&mut page_plan, limits.max_os_regions, 20);
    assert_eq!(queries, limits.max_os_regions, "one query per page");
    lines.push(percentile_line(
        "os_region_cap_worst_legal",
        page_plan.region_count(),
        page_plan.byte_count(),
        queries,
        &samples,
    ));
    drop(page_plan);
    drop(pages);

    let json = format!(
        "{{\n  \"schema\": \"hoi4.perf_d43.snapshot_gate_cost.v2\",\n  \"measured_by\": \"collector test attach_log::tests::d43_snapshot::d43_gated_copy_bench_receipt (ignored; cargo nextest run --release -p collector --lib -E 'test(d43_gated_copy_bench_receipt)' --run-ignored ignored-only --no-capture): the production bracket copy (Domain43RegionGate::confirm, one VirtualQuery per operating-system region, then seh_read_exact, through Domain43SnapshotPlan::capture_with) on this machine with the lane fleet running; p50 = middle sample, p95 = nearest rank, sorted ascending\",\n  \"optimized\": {},\n  \"caps\": {{\"max_regions\":{},\"max_bytes\":{},\"max_os_regions\":{}}},\n  \"shapes\": [{}]\n}}\n",
        !cfg!(debug_assertions),
        limits.max_regions,
        limits.max_bytes,
        limits.max_os_regions,
        lines.join(", ")
    );
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../runs/perf_d43_market/GATE_COST.json");
    std::fs::write(path, &json).expect("GATE_COST.json is written");
    println!("{json}");
}

/// One layout's demand: the live and traced walks, the trace, plans at five
/// merge gaps, and five snapshot frames through the bracket.
fn measure_layout(layout: Layout) -> String {
    let world = build_world_with(P4_SHAPE, layout);
    let state = world.state();
    let mut market = CompactMarketTradeTracker::default();
    page_cache_new_frame();
    let live_started = Instant::now();
    let live = read_compact_market_trade(&state, 0, &mut market);
    let live_us = live_started.elapsed().as_micros();
    assert_eq!(live.status_flags, 0);
    assert_all_families(&live, P4_SHAPE, layout.name());

    page_cache_new_frame();
    let tracing = Domain43TracingMemory::new(
        &D81LiveMemory,
        Domain43AccessTrace::with_capacity(4 << 20),
    );
    let traced_started = Instant::now();
    let traced = with_d81_memory_override(&tracing, || {
        read_compact_market_trade(&state, 0, &mut market)
    });
    let traced_us = traced_started.elapsed().as_micros();
    assert_eq!(traced.status_flags, 0);
    let trace = tracing.into_trace();
    let trace_reads = trace.reads().len();
    let trace_bytes: u64 = trace.reads().iter().map(|read| u64::from(read.len)).sum();
    let failed_reads = trace.reads().iter().filter(|read| !read.succeeded).count();

    let mut gaps = Vec::new();
    for gap in [0u64, 64, 256, 1024, 4096] {
        let mut copy = Domain43AccessTrace::with_capacity(trace.reads().len());
        for read in trace.reads() {
            copy.push(*read);
        }
        let plan_started = Instant::now();
        let plan = Domain43SnapshotPlan::from_trace(copy, uncapped_limits(gap)).expect("uncapped plan");
        let plan_us = plan_started.elapsed().as_micros();
        gaps.push(format!(
            "{{\"merge_gap\":{gap},\"regions\":{},\"bytes\":{},\"plan_us\":{plan_us}}}",
            plan.region_count(),
            plan.byte_count()
        ));
    }

    let mut trackers = CompactPerfGateTrackers::default();
    let mut read = |trackers: &mut CompactPerfGateTrackers| {
        page_cache_new_frame();
        let started = Instant::now();
        let captured = capture_compact_market_trade_snapshot(
            &state,
            0,
            &mut trackers.market_trade_live,
            &mut trackers.market_trade_live_snapshot,
        );
        let capture_us = started.elapsed().as_micros();
        let out = finish_compact_market_trade_snapshot(captured);
        (out, capture_us, started.elapsed().as_micros())
    };
    let _ = read(&mut trackers);
    let (armed, _, _) = read(&mut trackers);
    assert_eq!(armed.snapshot.mode, D43_SNAPSHOT_MODE_ARMING);
    assert_eq!(armed.snapshot.plan_refused, 0, "the default caps admit this world");
    let mut samples = Vec::new();
    for _ in 0..5 {
        let (out, capture_us, whole_us) = read(&mut trackers);
        assert_eq!(out.snapshot.mode, D43_SNAPSHOT_MODE_SNAPSHOT);
        assert_eq!(out.status_flags, 0);
        assert_all_families(&out, P4_SHAPE, layout.name());
        samples.push(format!(
            "{{\"capture_us\":{capture_us},\"whole_us\":{whole_us},\"regions\":{},\"bytes\":{},\"gate_queries\":{}}}",
            out.snapshot.copied_region_count, out.snapshot.copied_byte_count, out.snapshot.gate_queries
        ));
    }
    format!(
        "{{\"layout\":\"{}\",\"live_walk_us\":{live_us},\"traced_walk_us\":{traced_us},\"trace_reads\":{trace_reads},\"trace_failed_reads\":{failed_reads},\"trace_bytes\":{trace_bytes},\"armed_regions\":{},\"armed_bytes\":{},\"armed_probe_regions\":{},\"armed_os_regions\":{},\"plans_by_merge_gap\":[{}],\"snapshot_frames\":[{}]}}",
        layout.name(),
        armed.snapshot.region_count,
        armed.snapshot.byte_count,
        armed.snapshot.probe_regions,
        armed.snapshot.os_regions,
        gaps.join(", "),
        samples.join(", ")
    )
}

/// Demand measurement over the P4-shaped world in both layouts: trace reads,
/// regions and bytes at five merge gaps, the arena copy and the decode.
/// Writes `runs/perf_d43_market/SNAPSHOT_DEMAND.json`; the caps are justified
/// against the scattered layout.
#[test]
#[ignore]
fn d43_snapshot_demand_measurement() {
    let _rc_counters = super::super::lock_rc_counters();
    let layouts = [
        measure_layout(Layout::Contiguous),
        measure_layout(Layout::Scattered),
    ];
    let limits = Domain43SnapshotLimits::default();
    let json = format!(
        "{{\n  \"schema\": \"hoi4.perf_d43.snapshot_demand.v2\",\n  \"measured_by\": \"collector test attach_log::tests::d43_snapshot::d43_snapshot_demand_measurement (ignored; cargo nextest run [--release] -p collector --lib -E 'test(d43_snapshot_demand_measurement)' --run-ignored ignored-only --no-capture) over build_world_with(P4_SHAPE, layout)\",\n  \"optimized\": {},\n  \"world\": {{\"countries\":{},\"resources\":{},\"contracts\":{},\"routes\":{},\"stockpile_items\":{},\"units\":{},\"fleets\":{},\"task_forces\":{},\"wings_per_arm\":{},\"idle_wings_per_arm\":{},\"lines_per_country\":{},\"entries_per_line\":{}}},\n  \"world_source\": \"runs/perf_d43_market/P4_D43_FRAME_UNITS.json (P4 pair_1_session_2 receipts: contract_snapshot_countries, oil_selfcheck_land/fleet/task_force_elements, oil_wings_visited/idle_path, native_session_manifest market_trade_live rows)\",\n  \"layouts_note\": \"contiguous packs the objects at 16-byte alignment; scattered puts 64..4160 bytes between consecutive objects so no two merge at the production gap -- the region count a scattered heap costs this shape\",\n  \"layouts\": [{}],\n  \"caps\": {{\"merge_gap\":{},\"max_regions\":{},\"max_bytes\":{},\"max_trace_reads\":{},\"trace_reserve_reads\":{},\"max_os_regions\":{}}}\n}}\n",
        !cfg!(debug_assertions),
        P4_SHAPE.countries,
        P4_SHAPE.resources,
        P4_SHAPE.contracts,
        P4_SHAPE.routes,
        P4_SHAPE.stockpile_items,
        P4_SHAPE.units,
        P4_SHAPE.fleets,
        P4_SHAPE.task_forces,
        P4_SHAPE.wings_per_arm,
        P4_SHAPE.idle_wings_per_arm,
        P4_SHAPE.lines_per_country,
        P4_SHAPE.entries_per_line,
        layouts.join(",\n    "),
        limits.merge_gap,
        limits.max_regions,
        limits.max_bytes,
        limits.max_trace_reads,
        limits.trace_reserve_reads,
        limits.max_os_regions,
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../runs/perf_d43_market/SNAPSHOT_DEMAND.json"
    );
    std::fs::write(path, &json).expect("SNAPSHOT_DEMAND.json is written");
    println!("{json}");
}
