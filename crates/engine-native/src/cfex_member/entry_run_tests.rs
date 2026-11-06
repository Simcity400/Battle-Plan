//! The runs of [`super::append_entries`] against the parent's per-entry walk
//! ([`super::serial_reference`]): equal on healthy memory and under a lasting
//! fault at every byte either reads, and never harsher under a fault that
//! clears after its first touch. Arrays cross the run boundary twice (three
//! runs), with duplicate pointers, shared air-hit variants and inline and
//! heap hit names.
use super::*;
use crate::memory::SliceMemory;
use std::cell::{Cell, RefCell};

const BASE: usize = 0x1_4000_0000;
/// Three runs: the first, a middle and a short last one.
const COUNT: u32 = 2 * ENTRY_RUN as u32 + 12;
const ARRAY: usize = 0x1000;
const OBJECTS: usize = 0x4000;
const OBJECT: usize = 0x60;
const VARIANTS: usize = 0x2_0000;
const TEXTS: usize = 0x2_1000;
const SIZE: usize = 0x3_0000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lasting,
    Once,
}

/// The world's bytes, recording every read; reads overlapping `fault` fail
/// every time, or only the first in time.
struct World<'a> {
    bytes: &'a [u8],
    fault: Option<(usize, Mode)>,
    failures: Cell<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl<'a> World<'a> {
    fn new(bytes: &'a [u8], fault: Option<(usize, Mode)>) -> Self {
        Self {
            bytes,
            fault,
            failures: Cell::new(0),
            reads: RefCell::default(),
        }
    }

    /// The same bytes and fault, with its own reads and its one-shot fault
    /// unspent: a second walk meets the fault as the first did.
    fn fresh(&self) -> Self {
        Self::new(self.bytes, self.fault)
    }

    /// Every byte a read touched.
    fn touched(&self) -> Vec<usize> {
        let mut bytes: Vec<usize> = self
            .reads
            .borrow()
            .iter()
            .flat_map(|&(address, len)| address..address + len)
            .collect();
        bytes.sort_unstable();
        bytes.dedup();
        bytes
    }
}

impl Memory for World<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if let Some((at, mode)) = self.fault
            && address <= at
            && at - address < destination.len()
            && (mode == Mode::Lasting || self.failures.get() == 0)
        {
            self.failures.set(self.failures.get() + 1);
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

/// An MSVC string header at `at` holding `text`: inline up to 15 bytes,
/// otherwise on the heap at `heap`, each with its terminator.
fn string(bytes: &mut [u8], at: usize, text: &[u8], heap: usize) {
    if text.len() <= 15 {
        put(bytes, at, text);
        bytes[at + text.len()] = 0;
        put(bytes, at + 24, &15_u64.to_le_bytes());
    } else {
        put(bytes, at, &(heap as u64).to_le_bytes());
        put(bytes, heap, text);
        bytes[heap + text.len()] = 0;
        put(bytes, at + 24, &(text.len() as u64 + 8).to_le_bytes());
    }
    put(bytes, at + 16, &(text.len() as u64).to_le_bytes());
}

fn array() -> Array {
    Array {
        data: ARRAY,
        count: COUNT,
    }
}

/// The objects entry `index` of a pointer array leads to: every seventh
/// repeats the previous entry's object.
fn object(index: usize) -> usize {
    let index = if index % 7 == 6 { index - 1 } else { index };
    OBJECTS + index * OBJECT
}

fn naval_world() -> Vec<u8> {
    let mut bytes = vec![0; SIZE];
    for index in 0..COUNT as usize {
        let hit = object(index);
        put(&mut bytes, ARRAY + index * 8, &(hit as u64).to_le_bytes());
        put(
            &mut bytes,
            hit,
            &((BASE + crate::naval_combat_hit::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, hit + 8, &(index as u32 * 3).to_le_bytes());
        let name: Vec<u8> = (0..(index % 3) * 9)
            .map(|byte| b'a' + (byte % 26) as u8)
            .collect();
        string(&mut bytes, hit + 0x10, &name, TEXTS + index * 0x40);
        bytes[hit + 0x30] = (index % 2) as u8;
        put(
            &mut bytes,
            hit + 0x38,
            &(-(index as i64) * 77).to_le_bytes(),
        );
        put(&mut bytes, hit + 0x40, &(index as i64 + 1).to_le_bytes());
        bytes[hit + 0x48] = (index % 5 == 0) as u8;
    }
    bytes
}

fn air_world() -> Vec<u8> {
    let mut bytes = vec![0; SIZE];
    for variant in 0..4 {
        let at = VARIANTS + variant * 0x20;
        put(
            &mut bytes,
            at,
            &((BASE + crate::cequipment_variant::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, at + 8, &(0x1000 + variant as u64).to_le_bytes());
    }
    for index in 0..COUNT as usize {
        let hit = object(index);
        put(&mut bytes, ARRAY + index * 8, &(hit as u64).to_le_bytes());
        put(
            &mut bytes,
            hit,
            &((BASE + crate::naval_combat_air_hit::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            hit + 8,
            &((VARIANTS + (index % 4) * 0x20) as u64).to_le_bytes(),
        );
        put(&mut bytes, hit + 0x10, &(-(index as i32)).to_le_bytes());
        put(&mut bytes, hit + 0x14, &(index as u32 % 11).to_le_bytes());
    }
    bytes
}

fn damage_world() -> Vec<u8> {
    let mut bytes = vec![0; SIZE];
    for index in 0..COUNT as usize {
        let at = ARRAY + index * crate::cfex_damage_entry::STRIDE;
        put(
            &mut bytes,
            at,
            &((BASE + crate::cfex_damage_entry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + crate::cfex_damage_entry::SOURCE_HANDLE,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + crate::cfex_damage_entry::COUNTRY,
            &(index as u32 % 9).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + crate::cfex_damage_entry::DAMAGE,
            &(-(index as i64) * 1_000).to_le_bytes(),
        );
    }
    bytes
}

/// The parent's reader of an entry, and the runs' reader of one.
type ReadFn<T> = fn(&World<'_>, usize, usize) -> Result<T, &'static str>;
type EntryReadFn<T> = fn(&EntryMemory<'_, World<'_>>, usize, usize) -> Result<T, &'static str>;

/// What one append leaves: its result or error, the length, and every arena
/// row (owned rows, including those pushed before a failure; and a slice
/// arena, including rows written past the unchanged length).
type Appended<T> = (
    Result<[usize; 2], &'static str>,
    usize,
    Vec<T>,
    usize,
    Vec<T>,
);

/// Appends through the runs (`batched`) or the parent's walk, into an owned
/// arena and into a slice arena already holding one sentinel row. The slice
/// walk reads its own [`World::fresh`] copy, so a fault met only once fails
/// both walks alike instead of being spent by the first; `memory` records
/// the owned walk's reads, which are the slice walk's too.
fn append<T: Clone + Default>(
    memory: &World<'_>,
    batched: bool,
    stride: usize,
    object: Option<&'static [crate::memory::RecordField]>,
    read: ReadFn<T>,
    read_entry: EntryReadFn<T>,
) -> Appended<T> {
    let mut owned = Vec::new();
    let mut owned_length = 0;
    let mut slice = vec![T::default(); COUNT as usize + 1];
    let mut slice_length = 1;
    let second = memory.fresh();
    let (result, slice_result) = if batched {
        (
            append_entries(
                memory,
                array(),
                stride,
                EntryOutput::Owned(&mut owned),
                &mut owned_length,
                |memory, entry| read_entry(memory, BASE, entry),
                object,
            ),
            append_entries(
                &second,
                array(),
                stride,
                EntryOutput::Slice(&mut slice),
                &mut slice_length,
                |memory, entry| read_entry(memory, BASE, entry),
                object,
            ),
        )
    } else {
        (
            serial_reference::append_entries(
                memory,
                array(),
                stride,
                EntryOutput::Owned(&mut owned),
                &mut owned_length,
                |entry| read(memory, BASE, entry),
                object.is_some(),
            ),
            serial_reference::append_entries(
                &second,
                array(),
                stride,
                EntryOutput::Slice(&mut slice),
                &mut slice_length,
                |entry| read(&second, BASE, entry),
                object.is_some(),
            ),
        )
    };
    assert_eq!(result.is_ok(), slice_result.is_ok());
    (result, owned_length, owned, slice_length, slice)
}

/// The three entry kinds: (world, stride, object fields, parent reader, run reader).
fn check<T: Clone + Default + PartialEq + std::fmt::Debug>(
    bytes: &[u8],
    stride: usize,
    object: Option<&'static [crate::memory::RecordField]>,
    read: ReadFn<T>,
    read_entry: EntryReadFn<T>,
) {
    let healthy = |batched| {
        let memory = World::new(bytes, None);
        let appended = append(&memory, batched, stride, object, read, read_entry);
        (appended, memory.touched())
    };
    let (reference, serial_touched) = healthy(false);
    let (runs, runs_touched) = healthy(true);
    assert!(reference.0.is_ok(), "{:?}", reference.0);
    assert_eq!(runs, reference, "healthy");
    let mut faults: Vec<usize> = serial_touched.into_iter().chain(runs_touched).collect();
    faults.sort_unstable();
    faults.dedup();
    // Every run is reached: faults fall in the first, middle and last runs.
    let run_bytes = ENTRY_RUN * stride;
    for run in 0..3 {
        assert!(faults.contains(&(ARRAY + run * run_bytes)), "run {run}");
    }
    for &fault in &faults {
        let lasting = |batched| {
            append(
                &World::new(bytes, Some((fault, Mode::Lasting))),
                batched,
                stride,
                object,
                read,
                read_entry,
            )
        };
        assert_eq!(lasting(true), lasting(false), "lasting fault at {fault:#x}");
    }
}

/// The same faults met only once: the runs never refuse or withhold more than
/// the parent's walk.
fn check_once<T: Clone + Default + PartialEq + std::fmt::Debug>(
    bytes: &[u8],
    stride: usize,
    object: Option<&'static [crate::memory::RecordField]>,
    read: ReadFn<T>,
    read_entry: EntryReadFn<T>,
) {
    let memory = World::new(bytes, None);
    let healthy = append(&memory, false, stride, object, read, read_entry);
    let runs = World::new(bytes, None);
    let _ = append(&runs, true, stride, object, read, read_entry);
    let mut faults: Vec<usize> = memory.touched().into_iter().chain(runs.touched()).collect();
    faults.sort_unstable();
    faults.dedup();
    for fault in faults {
        let once = |batched| {
            append(
                &World::new(bytes, Some((fault, Mode::Once))),
                batched,
                stride,
                object,
                read,
                read_entry,
            )
        };
        let (runs, reference) = (once(true), once(false));
        match reference.0 {
            // Where the walk completes, so do the runs, with its rows.
            Ok(_) => assert_eq!(runs, reference, "one-touch fault at {fault:#x}"),
            // Where it refuses, the runs refuse as it does or complete
            // with the healthy rows.
            Err(_) => assert!(
                runs.0.is_err() || runs == healthy,
                "one-touch fault at {fault:#x}: {:?}",
                runs.0
            ),
        }
    }
}

fn naval_read(
    memory: &World<'_>,
    base: usize,
    entry: usize,
) -> Result<crate::naval_combat_hit::Row, &'static str> {
    crate::naval_combat_hit::read(memory, base, entry)
}
fn naval_entry(
    memory: &EntryMemory<'_, World<'_>>,
    base: usize,
    entry: usize,
) -> Result<crate::naval_combat_hit::Row, &'static str> {
    crate::naval_combat_hit::read(memory, base, entry)
}
fn air_read(
    memory: &World<'_>,
    base: usize,
    entry: usize,
) -> Result<crate::naval_combat_air_hit::Row, &'static str> {
    crate::naval_combat_air_hit::read(memory, base, entry)
}
fn air_entry(
    memory: &EntryMemory<'_, World<'_>>,
    base: usize,
    entry: usize,
) -> Result<crate::naval_combat_air_hit::Row, &'static str> {
    crate::naval_combat_air_hit::read(memory, base, entry)
}
fn damage_read(
    memory: &World<'_>,
    base: usize,
    entry: usize,
) -> Result<crate::cfex_damage_entry::Row, &'static str> {
    crate::cfex_damage_entry::read(memory, base, entry)
}
fn damage_entry(
    memory: &EntryMemory<'_, World<'_>>,
    base: usize,
    entry: usize,
) -> Result<crate::cfex_damage_entry::Row, &'static str> {
    crate::cfex_damage_entry::read(memory, base, entry)
}

/// Naval hits: pointer slots, every record span, inline and heap names.
#[test]
fn naval_hit_runs_keep_the_parent_walks_result_under_every_lasting_fault() {
    check(
        &naval_world(),
        8,
        Some(&crate::naval_combat_hit::RECORD),
        naval_read,
        naval_entry,
    );
}

/// Air hits: pointer slots, the record, the shared variants' identity and
/// handle.
#[test]
fn air_hit_runs_keep_the_parent_walks_result_under_every_lasting_fault() {
    check(
        &air_world(),
        8,
        Some(&crate::naval_combat_air_hit::RECORD),
        air_read,
        air_entry,
    );
}

/// Damage entries: the strided element runs, the gaps between fields included.
#[test]
fn damage_runs_keep_the_parent_walks_result_under_every_lasting_fault() {
    check(
        &damage_world(),
        crate::cfex_damage_entry::STRIDE,
        None,
        damage_read,
        damage_entry,
    );
}

#[test]
fn naval_hit_runs_are_never_harsher_under_faults_that_clear() {
    check_once(
        &naval_world(),
        8,
        Some(&crate::naval_combat_hit::RECORD),
        naval_read,
        naval_entry,
    );
}

#[test]
fn air_hit_runs_are_never_harsher_under_faults_that_clear() {
    check_once(
        &air_world(),
        8,
        Some(&crate::naval_combat_air_hit::RECORD),
        air_read,
        air_entry,
    );
}

#[test]
fn damage_runs_are_never_harsher_under_faults_that_clear() {
    check_once(
        &damage_world(),
        crate::cfex_damage_entry::STRIDE,
        None,
        damage_read,
        damage_entry,
    );
}
