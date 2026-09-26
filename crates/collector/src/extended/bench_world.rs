//! Synthetic many-country worlds for the ignored L3b capture-cost benches.
//!
//! Scale per country, chosen as a busy late-game average rather than a
//! vanilla count: one land theatre, two orders groups in its roster, two
//! orders per group. A faction per 25 countries has three theatres of six
//! regions, eight countries and two hidden tags. With `named`, each group's
//! first order is a move with the six name strings (five inline keys and a
//! 20-byte heap unique text) that succeeds into the second, and each group
//! has a 24-byte heap name; otherwise both orders are unlinked lines and the
//! group name is empty. With `theatre_links`, each group points at its
//! theatre, so the order of battle stages that theatre's roster.
//!
//! The world is laid out at its own buffer's address, module image first, so
//! every stored pointer is a real host address: the benches read it through
//! `SliceMemory` and through the production `CountedMemory<LiveMemory>`, whose
//! guarded copies cost what they cost in the game.
use crate::read_accounting::TableCounts;
use engine::{
    ccountry, cfaction, cfaction_system, cfaction_theater_manager as faction_theatres, cgame_state,
    corder_instance, corders_group, creference_registry as registry, ctheatre,
    memory::{CountedMemory, LiveMemory, ReadCounts, SliceMemory},
    msvc_string, reference_bucket as bucket,
};
use replay::rows::TableId;

/// The game state's offset from the module base.
const GAME_STATE: usize = 0x10000;
/// Above every module RVA the readers touch.
const HEAP: usize = 0x0350_0000;

pub(crate) struct World {
    /// Never grows after the build: `base` is its first byte's address.
    pub bytes: Vec<u8>,
    pub base: usize,
    pub game_state: usize,
    pub groups: u32,
    pub orders: u32,
    pub faction_theatres: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Shape {
    pub named: bool,
    pub theatre_links: bool,
}

struct Heap {
    bytes: Vec<u8>,
    /// The address of offset zero, the module base.
    origin: usize,
}

impl Heap {
    fn alloc(&mut self, size: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(16);
        self.bytes.resize(at + size, 0);
        at
    }
    /// The address of `offset`; offset zero stays a null pointer.
    fn ptr(&self, offset: usize) -> u64 {
        if offset == 0 {
            0
        } else {
            (self.origin + offset) as u64
        }
    }
    fn rva(&self, rva: usize) -> u64 {
        (self.origin + rva) as u64
    }
    fn put(&mut self, at: usize, value: &[u8]) {
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, self.ptr(data));
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn pointers(&mut self, at: usize, values: &[usize]) {
        let data = self.alloc(values.len().max(1) * 8);
        for (index, value) in values.iter().enumerate() {
            self.u64(data + index * 8, self.ptr(*value));
        }
        self.array(at, data, values.len() as u32);
    }
    fn words(&mut self, at: usize, values: &[u32]) {
        let data = self.alloc(values.len().max(1) * 4);
        for (index, value) in values.iter().enumerate() {
            self.u32(data + index * 4, *value);
        }
        self.array(at, data, values.len() as u32);
    }
    fn string(&mut self, at: usize, text: &[u8]) {
        if text.len() <= msvc_string::INLINE_CAPACITY as usize {
            self.put(at, text);
            self.u64(at + msvc_string::CAPACITY, msvc_string::INLINE_CAPACITY);
        } else {
            let data = self.alloc(text.len() + 1);
            self.put(data, text);
            self.u64(at, self.ptr(data));
            self.u64(at + msvc_string::CAPACITY, text.len() as u64);
        }
        self.u64(at + msvc_string::LENGTH, text.len() as u64);
    }
}

pub(crate) fn build(countries: u32, shape: Shape) -> World {
    // The layout does not depend on the origin: size it once, then lay it out
    // in a buffer that never reallocates.
    let len = build_in(countries, shape, Vec::new()).bytes.len();
    let world = build_in(countries, shape, Vec::with_capacity(len));
    assert_eq!(world.bytes.len(), len);
    world
}

fn build_in(countries: u32, shape: Shape, mut bytes: Vec<u8>) -> World {
    let origin = bytes.as_ptr() as usize;
    bytes.resize(HEAP, 0);
    let mut heap = Heap { bytes, origin };
    let slots = countries as usize + 1;
    let table = heap.alloc(slots * 8);
    let links = heap.alloc(slots * 4);
    heap.array(GAME_STATE + cgame_state::COUNTRIES, table, slots as u32);
    heap.array(GAME_STATE + cgame_state::COUNTRY_LINKS, links, slots as u32);
    let mut roots = Vec::new();
    let mut orders = 0;
    for slot in 1..slots {
        let country = heap.alloc(0x200);
        heap.u64(table + slot * 8, heap.ptr(country));
        heap.u32(links + slot * 4, slot as u32);
        heap.u64(country, heap.rva(ccountry::VTABLE_RVA));
        heap.u32(country + ccountry::ID, slot as u32);
        heap.array(country + ccountry::AIR_THEATRES, 0, 0);
        let theatre = heap.alloc(ctheatre::SIZE);
        heap.pointers(country + ccountry::THEATRES, &[theatre]);
        heap.u64(theatre, heap.rva(ctheatre::VTABLE_RVA));
        heap.u64(
            theatre + ctheatre::HANDLE,
            ((slot as u64) << 32) | u64::from(ctheatre::REFERENCE_TYPE),
        );
        heap.u32(theatre + ctheatre::OWNER, slot as u32);
        let mut roster = Vec::new();
        for _ in 0..2 {
            let group = heap.alloc(corders_group::SIZE + 0x20);
            let handle =
                ((roots.len() as u64 + 1) << 32) | u64::from(corders_group::REFERENCE_TYPE);
            heap.u64(group, heap.rva(corders_group::VTABLE_RVA));
            heap.u64(group + corders_group::HANDLE, handle);
            heap.u32(group + corders_group::COUNTRY, slot as u32);
            if shape.theatre_links {
                heap.u64(group + corders_group::THEATRE, heap.ptr(theatre));
            }
            heap.u32(group + corders_group::COLOUR, 0x3f80_0000);
            heap.string(
                group + corders_group::NAME,
                if shape.named {
                    b"12. Armee (Heeresgruppe)"
                } else {
                    b""
                },
            );
            let first = heap.alloc(0x400);
            let second = heap.alloc(0x400);
            for (index, order) in [first, second].into_iter().enumerate() {
                heap.u64(order, heap.rva(corder_instance::VTABLE_RVA));
                heap.u64(order + corder_instance::OWNER, heap.ptr(group));
                heap.u32(order + corder_instance::ID, index as u32 + 1);
                let move_order = shape.named && index == 0;
                heap.u32(
                    order + corder_instance::TYPE,
                    if move_order {
                        corder_instance::TYPE_MOVE
                    } else {
                        corder_instance::TYPE_LINE
                    } as u32,
                );
                if move_order {
                    for (name, offset) in corder_instance::NAME_STRINGS.into_iter().enumerate() {
                        let text: &[u8] = match name {
                            4 => b"Operation Barbarossa",
                            0 => b"o_operation",
                            _ => b"o_key",
                        };
                        heap.string(order + offset, text);
                    }
                }
            }
            if shape.named {
                heap.pointers(first + corder_instance::SUCCESSOR_DATA, &[second]);
                heap.pointers(second + corder_instance::PREDECESSOR_DATA, &[first]);
            }
            heap.pointers(group + corders_group::PRIMARY_ORDER_DATA, &[first, second]);
            orders += 2;
            roster.push(group);
            roots.push((group, handle));
        }
        heap.pointers(theatre + ctheatre::ORDERS_GROUP_DATA, &roster);
    }
    // The type-53 registry: every group in its own bucket.
    let capacity = (roots.len() * 2).next_power_of_two().max(8);
    let exponent = capacity.trailing_zeros();
    let distance = exponent + 2;
    let registry_table = heap.alloc(registry::SIZE);
    let buckets = heap.alloc((capacity + distance as usize + 1) * bucket::STRIDE);
    heap.u64(
        registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8,
        heap.ptr(registry_table),
    );
    heap.u64(registry_table + registry::ENTRIES, heap.ptr(buckets));
    heap.u32(registry_table + registry::COUNT, roots.len() as u32);
    heap.u32(registry_table + registry::MASK, capacity as u32 - 1);
    heap.put(
        registry_table + registry::OVERFLOW_DISTANCE,
        &[distance as u8],
    );
    for (index, (group, handle)) in roots.iter().enumerate() {
        let entry = buckets + index * bucket::STRIDE;
        heap.put(entry + bucket::DISTANCE, &[1]);
        heap.u64(entry + bucket::TYPE, *handle);
        heap.u64(entry + bucket::OBJECT, heap.ptr(*group));
    }
    heap.put(
        buckets + (capacity + distance as usize) * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    // Factions and their theatres.
    let factions = (countries / 25).max(1);
    let system = heap.alloc(0x40);
    heap.u64(GAME_STATE + cgame_state::FACTION_SYSTEM, heap.ptr(system));
    heap.u64(system, heap.rva(cfaction_system::VTABLE_RVA));
    let mut pointers = Vec::new();
    for index in 0..factions {
        let faction = heap.alloc(cfaction::THEATER_MANAGER + 0x20);
        heap.u64(faction, heap.rva(cfaction::VTABLE_RVA));
        heap.u64(
            faction + cfaction::REFERENCE,
            (u64::from(index + 1) << 32) | u64::from(cfaction::REFERENCE_TYPE),
        );
        heap.string(faction + cfaction::ICON, b"GFX_faction_ax");
        let manager = faction + cfaction::THEATER_MANAGER;
        heap.u64(manager, heap.rva(faction_theatres::VTABLE_RVA));
        heap.u64(manager + faction_theatres::OWNER, heap.ptr(faction));
        let entries = heap.alloc(3 * faction_theatres::THEATRE_STRIDE);
        heap.array(manager + faction_theatres::THEATRES, entries, 3);
        for theatre in 0..3 {
            let entry = entries + theatre * faction_theatres::THEATRE_STRIDE;
            heap.words(entry + faction_theatres::COUNTRIES, &[1; 8]);
            heap.words(entry + faction_theatres::REGIONS, &[7, 8, 9, 10, 11, 12]);
            heap.words(entry + faction_theatres::HIDDEN_FOR, &[3, 4]);
        }
        pointers.push(faction);
    }
    heap.pointers(system + cfaction_system::FACTIONS, &pointers);
    World {
        base: origin,
        game_state: origin + GAME_STATE,
        bytes: heap.bytes,
        groups: roots.len() as u32,
        orders,
        faction_theatres: factions * 3,
    }
}

/// Median and 95th percentile of `runs` timed calls after three warm-ups, in
/// microseconds.
pub(crate) fn time(runs: usize, mut call: impl FnMut()) -> (f64, f64) {
    for _ in 0..3 {
        call();
    }
    let mut samples: Vec<f64> = (0..runs)
        .map(|_| {
            let started = std::time::Instant::now();
            call();
            started.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    (samples[runs / 2], samples[(runs * 95 / 100).min(runs - 1)])
}

impl World {
    /// The world as plain owned bytes.
    pub(crate) fn slice(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.base,
            bytes: &self.bytes,
        }
    }

    /// The world through the Orders worker's source: guarded copies behind the
    /// read counter.
    pub(crate) fn live(&self) -> CountedMemory<LiveMemory> {
        // SAFETY: the world's bytes are owned, immutable and alive for every
        // read the bench makes through this handle.
        CountedMemory::new(unsafe { LiveMemory::new() })
    }

    /// The source requests of one `work` call per table, made through the
    /// counted slice. Counts do not depend on load or on the source kind.
    pub(crate) fn counted<R>(
        &self,
        work: impl FnOnce(&CountedMemory<SliceMemory<'_>>) -> R,
    ) -> (R, TableCounts) {
        let memory = CountedMemory::new(self.slice());
        let capture = crate::read_accounting::Capture::new();
        let result = work(&memory);
        (result, capture.finish())
    }
}

/// The sum of `tables`' source requests.
pub(crate) fn total(counts: &TableCounts, tables: &[TableId]) -> ReadCounts {
    let mut sum = ReadCounts::ZERO;
    for table in tables {
        sum += counts[table.index()];
    }
    sum
}

/// One table's source requests as a bench field.
pub(crate) fn work(counts: ReadCounts) -> String {
    format!(
        "calls {} (copy {} records {} scattered {} plans {}) spans {} bytes {}",
        counts.copy_calls + counts.record_calls + counts.scattered_calls + counts.plan_calls,
        counts.copy_calls,
        counts.record_calls,
        counts.scattered_calls,
        counts.plan_calls,
        counts.spans,
        counts.bytes
    )
}
