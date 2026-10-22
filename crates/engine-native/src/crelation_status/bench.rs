use crate::{
    Memory, ccountry, cgame_state,
    memory::{LiveMemory, Raw, RecordLayout, RecordSources},
};
use std::{cell::Cell, time::Instant};

/// Covers the highest RVA the benched walkers read (the reference roots).
pub const IMAGE: usize = 0x0344_0000;
pub const RUNS: usize = 20;

pub struct Arena {
    bytes: Vec<u8>,
    next: usize,
}

impl Arena {
    pub fn new(data: usize) -> Self {
        Self {
            bytes: vec![0; IMAGE + data],
            next: IMAGE,
        }
    }
    pub fn base(&self) -> usize {
        self.bytes.as_ptr() as usize
    }
    /// A 16-byte aligned, zeroed span; returns its address.
    pub fn alloc(&mut self, len: usize) -> usize {
        let at = (self.next + 15) & !15;
        self.next = at + len;
        assert!(self.next <= self.bytes.len(), "bench arena exhausted");
        self.base() + at
    }
    fn put(&mut self, address: usize, value: &[u8]) {
        let at = address - self.base();
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }
    pub fn u8(&mut self, address: usize, value: u8) {
        self.put(address, &[value]);
    }
    pub fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    pub fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    /// `base + rva`, the value a native vtable or global slot holds.
    pub fn rva(&self, rva: usize) -> u64 {
        (self.base() + rva) as u64
    }
    /// A CPdxArray header with equal capacity and count.
    pub fn vector(&mut self, header: usize, data: usize, count: u32) {
        self.u64(header, data as u64);
        self.u32(header + 8, count);
        self.u32(header + 12, count);
    }
}

/// Countries in slots 1..=n with identity links; returns the game state
/// and each slot's object (index 0 unused).
pub fn countries(arena: &mut Arena, n: usize, size: usize) -> (usize, Vec<usize>) {
    let game_state = arena.alloc(0x1000);
    let table = arena.alloc((n + 1) * 8);
    let links = arena.alloc((n + 1) * 4);
    arena.vector(game_state + cgame_state::COUNTRIES, table, n as u32 + 1);
    arena.vector(game_state + cgame_state::COUNTRY_LINKS, links, n as u32 + 1);
    let vtable = arena.rva(ccountry::VTABLE_RVA);
    let mut objects = vec![0];
    for slot in 0..=n {
        arena.u32(links + slot * 4, slot as u32);
        if slot == 0 {
            continue;
        }
        let country = arena.alloc(size);
        arena.u64(table + slot * 8, country as u64);
        arena.u64(country, vtable);
        arena.u32(country + ccountry::ID, slot as u32);
        objects.push(country);
    }
    (game_state, objects)
}

/// Counts every guarded call and the bytes it copied, forwarding to the
/// real SEH-guarded reader.
pub struct Counting {
    live: LiveMemory,
    pub calls: Cell<u64>,
    pub bytes: Cell<u64>,
    /// The collector's authenticated country census, when the bench
    /// models a job that receives it (every slot, both table headers).
    census: Option<(
        Vec<Option<ccountry::Country>>,
        [crate::cpdx_array::Array; 2],
    )>,
}

impl Counting {
    #[allow(clippy::new_without_default)] // Existing constructor shared by synthetic benchmarks.
    pub fn new() -> Self {
        Self {
            // SAFETY: only the immutable bench arena is read.
            live: unsafe { LiveMemory::new() },
            calls: Cell::new(0),
            bytes: Cell::new(0),
            census: None,
        }
    }

    /// Serves the country census the collector's jobs are given for
    /// countries in slots 1..=n with identity links, as `countries` builds.
    #[cfg(test)]
    pub fn with_census(mut self, game_state: usize, objects: &[usize]) -> Self {
        let n = objects.len() as u32;
        let read = |at: usize| crate::cpdx_array::Array {
            data: crate::read::<u64>(&self.live, game_state + at).unwrap() as usize,
            count: n,
        };
        let arrays = [
            read(cgame_state::COUNTRIES),
            read(cgame_state::COUNTRY_LINKS),
        ];
        let slots = objects
            .iter()
            .enumerate()
            .map(|(slot, &pointer)| {
                (slot != 0).then_some(ccountry::Country {
                    pointer,
                    canonical_slot: slot as u32,
                    id: slot as u32,
                })
            })
            .collect();
        self.census = Some((slots, arrays));
        self
    }
    fn count(&self, bytes: usize) {
        self.calls.set(self.calls.get() + 1);
        self.bytes.set(self.bytes.get() + bytes as u64);
    }
}

impl Memory for Counting {
    fn authenticated_countries(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.census
            .as_ref()
            .map(|(slots, arrays)| ccountry::AuthenticatedRoster::new(slots, *arrays))
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let copied = self.live.copy(address, destination);
        self.count(if copied { destination.len() } else { 0 });
        copied
    }
    fn prefetch(&self, address: usize, len: usize) {
        self.live.prefetch(address, len);
    }
    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        let value = self.live.read_scalar::<T>(address);
        self.count(if value.is_some() {
            std::mem::size_of::<T>()
        } else {
            0
        });
        value
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.live.copy_scattered(addresses, destinations);
        self.count(destinations[..copied].iter().map(|d| d.len()).sum());
        copied
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.live.copy_scattered_prefetched(addresses, destinations);
        self.count(destinations[..copied].iter().map(|d| d.len()).sum());
        copied
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let copied = self.live.copy_records(sources, layout, destination);
        self.count(layout.completed_bytes(copied));
        copied
    }
}

/// Runs `walk` [`RUNS`] times and prints the median wall time with the last
/// run's guarded calls, bytes and the walk's own summary.
pub fn report(table: &str, countries: usize, memory: &Counting, mut walk: impl FnMut() -> String) {
    let mut times = Vec::with_capacity(RUNS);
    let mut summary = String::new();
    for _ in 0..RUNS {
        memory.calls.set(0);
        memory.bytes.set(0);
        let started = Instant::now();
        summary = walk();
        times.push(started.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "BENCH | {table} | {countries} | {:.3} ms | {} calls | {} bytes | {summary}",
        times[RUNS / 2],
        memory.calls.get(),
        memory.bytes.get(),
    );
}

/// World war of two sides of n/2 (every country at war with the whole
/// other side, allied to its own), one military access relation from each
/// country to the next.
fn relation_world(n: usize) -> (Arena, usize) {
    use super::*;
    let mut arena = Arena::new(n * 0x1c00 + (4 * n + 64) * 8 + 0x10_0000);
    let (game_state, countries) = countries(&mut arena, n, 0x1000);
    let half = n / 2;
    // Each side's member links, shared by every member as enemies/allies.
    let sides = [arena.alloc(half * 4), arena.alloc((n - half) * 4)];
    for slot in 1..=n {
        let (side, index) = if slot <= half {
            (0, slot - 1)
        } else {
            (1, slot - 1 - half)
        };
        arena.u32(sides[side] + index * 4, slot as u32);
    }
    // Matrix rows overlap: row i starts two entries after row i-1, and the
    // only entries read, toward i-1 and i+1, land on distinct cells 3i-1 and 3i+1.
    let matrix = arena.alloc((3 * n + 8) * 8);
    let relations: Vec<usize> = (0..n)
        .map(|slot| {
            if slot == 0 || slot == n {
                return 0;
            }
            let relation = arena.alloc(0x20);
            arena.u64(relation, arena.rva(cmilitary_access_relation::VTABLE_RVA));
            arena.u32(
                relation + crelation::TYPE_ID,
                cmilitary_access_relation::TYPE,
            );
            arena.u32(relation + crelation::FIRST_COUNTRY, slot as u32);
            arena.u32(relation + crelation::SECOND_COUNTRY, slot as u32 + 1);
            relation
        })
        .collect();
    for (slot, &country) in countries.iter().enumerate().skip(1) {
        let diplomacy = arena.alloc(0x300);
        arena.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        arena.u64(diplomacy, arena.rva(cdiplomacy_status::VTABLE_RVA));
        arena.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        arena.u8(country + ccountry::CAN_OCCUPY_NON_WAR, 1);
        let row = matrix + slot * 16;
        arena.vector(diplomacy + cdiplomacy_status::RELATIONS, row, n as u32 + 1);
        let own = usize::from(slot > half);
        let enemy = 1 - own;
        let counts = [half, n - half];
        arena.vector(
            diplomacy + cdiplomacy_status::ENEMY_COUNTRIES,
            sides[enemy],
            counts[enemy] as u32,
        );
        arena.vector(
            diplomacy + cdiplomacy_status::WARTIME_ALLIES,
            sides[own],
            counts[own] as u32,
        );
        let mut owned = Vec::new();
        for (partner, relation) in [
            (
                slot.wrapping_sub(1),
                relations.get(slot - 1).copied().unwrap_or(0),
            ),
            (slot + 1, relations.get(slot).copied().unwrap_or(0)),
        ] {
            if relation == 0 {
                continue;
            }
            owned.push(relation);
            let status = arena.alloc(0x300);
            arena.u64(row + partner * 8, status as u64);
            arena.u64(status, arena.rva(VTABLE_RVA));
            arena.u64(status + DIPLOMACY_STATUS, diplomacy as u64);
            arena.u32(status + TARGET_COUNTRY, partner as u32);
            // The first country's status toward the second holds the access.
            if partner == slot + 1 {
                arena.u64(status + MILITARY_ACCESS, relation as u64);
            }
        }
        let data = arena.alloc(owned.len() * 8);
        for (index, relation) in owned.iter().enumerate() {
            arena.u64(data + index * 8, *relation as u64);
        }
        arena.vector(
            diplomacy + cdiplomacy_status::OWNED_RELATIONS,
            data,
            owned.len() as u32,
        );
    }
    (arena, game_state)
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_relation_links_and_country_wars() {
    use super::*;
    for n in [1_500, 5_000] {
        let (arena, game_state) = relation_world(n);
        let memory = Counting::new();
        let mut rows = vec![LinkRow::default(); 2 * n];
        let mut wars = vec![WarRow::default(); n];
        let mut links_out = vec![0_u32; n * n];
        let mut resolver = Resolver::with_capacity(2 * n + 2);
        report("RelationLinks+CountryWars", n, &memory, || {
            let walk = links(
                &memory,
                arena.base(),
                game_state,
                &mut rows,
                WarOutput {
                    rows: &mut wars,
                    countries: &mut links_out,
                },
                &mut resolver,
            );
            assert_eq!(walk.links, Outcome::Rows { len: n - 1 });
            assert_eq!(walk.wars, Outcome::Rows { len: n });
            format!(
                "{} link rows, {} war rows, {} war links",
                walk.links.len(),
                walk.wars.len(),
                walk.countries_len
            )
        });
    }
}
