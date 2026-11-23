use super::*;
use engine_native_parts::{
    acclimatization_category as category,
    acclimatization_data as data,
    acclimatization_entry as entry,
    csub_unit_definition as stats,
};
use engine_native_memory::memory::LiveMemory;
use std::{hint::black_box, time::Instant};

const IMAGE_BYTES: usize = 0x0346_0000;
const RUNS: usize = 25;

struct Arena {
    bytes: Vec<u8>,
    next: usize,
}

impl Arena {
    fn new(bytes: usize) -> Self {
        Self {
            bytes: vec![0; IMAGE_BYTES + bytes],
            next: IMAGE_BYTES,
        }
    }

    fn base(&self) -> usize {
        self.bytes.as_ptr() as usize
    }

    fn alloc(&mut self, len: usize) -> usize {
        let at = self.next.next_multiple_of(16);
        self.next = at + len;
        assert!(self.next <= self.bytes.len());
        self.base() + at
    }

    fn put(&mut self, address: usize, bytes: &[u8]) {
        let at = address - self.base();
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn word(&mut self, address: usize, word: u64) {
        self.put(address, &word.to_le_bytes());
    }

    fn vector(&mut self, address: usize, data: usize, count: usize) {
        self.word(address, data as u64);
        self.put(address + 8, &(count as u32).to_le_bytes());
        self.put(address + 12, &(count as u32).to_le_bytes());
    }
}

struct World {
    arena: Arena,
    state: usize,
    seeds: Vec<DivisionSeed>,
    held: Vec<Option<stats::DivisionMaxima>>,
}

impl World {
    fn new(countries: usize, divisions: usize) -> Self {
        let mut arena = Arena::new(divisions * 0x900 + countries * 0x1000 + 0x10000);
        let base = arena.base();
        let state = arena.alloc(0x1000);
        let slots = arena.alloc((countries + 1) * 8);
        let links = arena.alloc((countries + 1) * 4);
        arena.vector(state + cgame_state::COUNTRIES, slots, countries + 1);
        arena.vector(state + cgame_state::COUNTRY_LINKS, links, countries + 1);
        for slot in 1..=countries {
            let country = arena.alloc(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x40);
            arena.word(country, (base + ccountry::VTABLE_RVA.0) as u64);
            arena.put(country + ccountry::ID, &(slot as u32).to_le_bytes());
            arena.word(slots + slot * 8, country as u64);
            arena.put(links + slot * 4, &(slot as u32).to_le_bytes());
        }
        arena.word(base + COUNTER_ATTRITION_THRESHOLD_RVA.0, 50_000);
        arena.word(base + data::CAMO_SHOW_AT_RVA.0, 35_000);
        let registry = arena.alloc(16);
        let categories = arena.alloc(16);
        let climate: [usize; 2] = std::array::from_fn(|index| {
            let object = arena.alloc(0x40);
            arena.put(object + category::TOKEN, &(index as u32 + 1).to_le_bytes());
            arena.word(
                object + data::CATEGORY_FACTOR,
                40_000 + index as u64 * 30_000,
            );
            arena.word(categories + index * 8, object as u64);
            object
        });
        arena.vector(registry, categories, climate.len());
        arena.word(base + category::REGISTRY_RVA.0, registry as u64);
        let blocks: Vec<_> = (0..divisions.div_ceil(10))
            .map(|index| {
                let pointer = arena.alloc(0x600);
                let maxima = stats::DivisionMaxima {
                    pointer,
                    organisation: 6_000_000 + index as i64,
                    strength: 2_500_000 + index as i64,
                    can_exfiltrate_from_coast: (index % 3) as u8,
                };
                arena.word(pointer, (base + stats::VTABLE_RVA.0) as u64);
                arena.word(
                    pointer + stats::MAXIMUM_ORGANISATION,
                    maxima.organisation as u64,
                );
                arena.word(pointer + stats::MAXIMUM_STRENGTH, maxima.strength as u64);
                arena.put(
                    pointer + stats::CAN_EXFILTRATE_FROM_COAST,
                    &[maxima.can_exfiltrate_from_coast],
                );
                maxima
            })
            .collect();
        let mut held = Vec::with_capacity(divisions);
        let seeds = (0..divisions)
            .map(|index| {
                let army = arena.alloc(0x500);
                let count = if index.is_multiple_of(257) { 33 } else { 2 };
                let entries = arena.alloc(count * entry::STRIDE);
                for ordinal in 0..count {
                    let at = entries + ordinal * entry::STRIDE;
                    arena.word(at + entry::CATEGORY, climate[ordinal % 2] as u64);
                    arena.word(at + entry::WEIGHT, 50_000 + (ordinal % 2) as u64 * 10_000);
                }
                let mut object = [0; 0x40];
                object[..8].copy_from_slice(&((base + data::VTABLE_RVA.0) as u64).to_le_bytes());
                object[data::DATA..data::DATA + 8].copy_from_slice(&(entries as u64).to_le_bytes());
                object[data::CAPACITY..data::CAPACITY + 4]
                    .copy_from_slice(&(count as u32).to_le_bytes());
                object[data::COUNT..data::COUNT + 4].copy_from_slice(&(count as u32).to_le_bytes());
                object[data::ACTIVE..data::ACTIVE + 8]
                    .copy_from_slice(&(climate[1] as u64).to_le_bytes());
                object[data::MAXIMUM..data::MAXIMUM + 8]
                    .copy_from_slice(&100_000_u64.to_le_bytes());
                let maxima = blocks[index / 10];
                held.push((!index.is_multiple_of(333)).then_some(maxima));
                let division = ((index as u64 + 1) << 32) | 9;
                DivisionSeed {
                    pointer: army,
                    division,
                    primary_vtable: Some((base + VTABLE_RVA.0) as u64),
                    handle: Some(division),
                    stat_block: Some(maxima.pointer),
                    roster_owner: Some((index % countries + 1) as u32),
                    expeditionary_owner: Some(0),
                    orders_group: Some(0),
                    counter_attrition: Some(index as i64 % 1000),
                    entrenchment: Some(30_000),
                    entrenchment_cap: Some(50_000),
                    counter_supply_status: Some(90_000),
                    supply_grace_maximum: Some(72_000_000),
                    supply_grace_remaining: Some(36_000_000),
                    invalid_orders: Some(0),
                    strength_factor: Some(95_000),
                    exiled: Some(0),
                    noncore_supply_bypass: Some(0),
                    model_number: Some(index as i32 % 8),
                    acclimatization: Some(object),
                    ..DivisionSeed::default()
                }
            })
            .collect();
        Self {
            arena,
            state,
            seeds,
            held,
        }
    }
}

fn maxima_pass(
    packet: &mut StatusPacket,
    memory: &impl Memory,
    base: usize,
    seeds: &[DivisionSeed],
    held: &[Option<stats::DivisionMaxima>],
) -> usize {
    packet.clear();
    let mut batch = stats::MaximaBatch::new();
    let mut checksum = 0;
    for (chunk, seeds) in seeds.chunks(64).enumerate() {
        let start = chunk * 64;
        let held = held
            .get(start..held.len().min(start + seeds.len()))
            .unwrap_or(&[]);
        let selected = packet.stage_maxima(memory, base, seeds, held, |_| false, &mut batch);
        for (index, seed) in seeds.iter().enumerate() {
            checksum ^= packet
                .maxima(
                    memory,
                    base,
                    seed.stat_block.unwrap(),
                    selected[index],
                    held.get(index).copied().flatten(),
                    None,
                    &batch,
                )
                .unwrap();
        }
    }
    black_box(checksum)
}

fn measure(label: &str, countries: usize, divisions: usize, mut capture: impl FnMut()) {
    for _ in 0..3 {
        capture();
    }
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let begin = Instant::now();
        capture();
        samples.push(begin.elapsed().as_nanos() as f64 / divisions as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "DIVISION_STATUS_BENCH path={label} countries={countries} divisions={divisions} ns_per_division={:.1}",
        samples[RUNS / 2]
    );
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_division_status_and_maxima() {
    if cfg!(debug_assertions) {
        panic!("run with --release");
    }
    for countries in [1_500, 5_000] {
        for divisions in [20_000, 60_000] {
            let world = World::new(countries, divisions);
            let base = world.arena.base();
            let memory = unsafe { LiveMemory::new() };
            let mut packet = StatusPacket::with_capacities(StatusCapacities {
                acclimatization_categories: 2,
                modifiers: engine_native_parts::modifier_accumulator::SnapshotCapacity {
                    nodes: countries,
                    entries: 0,
                },
                ..StatusCapacities::new(divisions)
            })
            .unwrap();
            let mut rows = vec![StatusRow::default(); divisions];
            let mut fresh = Vec::new();
            for (form, held) in [("fresh", &[][..]), ("carried", world.held.as_slice())] {
                measure(&format!("maxima_{form}"), countries, divisions, || {
                    black_box(maxima_pass(&mut packet, &memory, base, &world.seeds, held));
                });
                measure(&format!("status_{form}"), countries, divisions, || {
                    let outcome = packet.capture_holding(
                        &memory,
                        base,
                        world.state,
                        &world.seeds,
                        Outcome::Rows { len: divisions },
                        held,
                        &[],
                    );
                    assert_eq!(outcome, Outcome::Rows { len: divisions });
                    assert_eq!(packet.project(&mut rows), outcome);
                    black_box(&rows);
                });
                assert!(
                    rows.iter()
                        .all(|row| row.camouflage.is_some() && row.weather_factor.is_some())
                );
                if fresh.is_empty() {
                    fresh = rows.clone();
                } else {
                    assert_eq!(rows, fresh);
                }
            }
        }
    }
}
