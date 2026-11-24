use engine_native_memory::{
    Memory,
    memory::{LiveMemory, Raw, RecordLayout, RecordSources},
};
use crate::{ccountry, cgame_state};
use std::{cell::Cell, time::Instant};

pub const IMAGE: usize = 0x0351_0000;
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
    pub fn rva(&self, rva: engine_sites::Rva) -> u64 {
        (self.base() + rva.0) as u64
    }
    pub fn vector(&mut self, header: usize, data: usize, count: u32) {
        self.u64(header, data as u64);
        self.u32(header + 8, count);
        self.u32(header + 12, count);
    }
}

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

pub struct Counting {
    live: LiveMemory,
    pub calls: Cell<u64>,
    pub bytes: Cell<u64>,
    census: Option<(
        Vec<Option<engine_native_memory::country_identity::Country>>,
        [engine_native_memory::cpdx_array::Array; 2],
    )>,
}

impl Counting {
    #[expect(
        clippy::new_without_default,
        reason = "new attaches to live process memory through unsafe code, which a Default impl would hide"
    )]
    pub fn new() -> Self {
        Self {
            live: unsafe { LiveMemory::new() },
            calls: Cell::new(0),
            bytes: Cell::new(0),
            census: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_census(mut self, game_state: usize, objects: &[usize]) -> Self {
        let n = objects.len() as u32;
        let read = |at: usize| engine_native_memory::cpdx_array::Array {
            data: engine_native_memory::read::<u64>(&self.live, game_state + at).unwrap() as usize,
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
                (slot != 0).then_some(engine_native_memory::country_identity::Country {
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
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.census
            .as_ref()
            .map(|(slots, arrays)| engine_native_memory::country_identity::AuthenticatedRoster::new(slots, *arrays))
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

