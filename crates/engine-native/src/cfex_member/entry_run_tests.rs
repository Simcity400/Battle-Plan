use super::*;
use crate::memory::SliceMemory;
use std::cell::{Cell, RefCell};

const BASE: usize = 0x1_4000_0000;
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

    fn fresh(&self) -> Self {
        Self::new(self.bytes, self.fault)
    }

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
            &((BASE + engine_native_naval::naval_combat_hit::VTABLE_RVA.0) as u64).to_le_bytes(),
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
            &((BASE + engine_native_naval::naval_combat_air_hit::VTABLE_RVA.0) as u64).to_le_bytes(),
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

type ReadFn<T> = fn(&World<'_>, usize, usize) -> Result<T, &'static str>;
type EntryReadFn<T> = fn(&EntryMemory<'_, World<'_>>, usize, usize) -> Result<T, &'static str>;

type Appended<T> = (
    Result<[usize; 2], &'static str>,
    usize,
    Vec<T>,
    usize,
    Vec<T>,
);

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
            Ok(_) => assert_eq!(runs, reference, "one-touch fault at {fault:#x}"),
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
) -> Result<engine_native_naval::naval_combat_hit::Row, &'static str> {
    engine_native_naval::naval_combat_hit::read(memory, base, entry)
}
fn naval_entry(
    memory: &EntryMemory<'_, World<'_>>,
    base: usize,
    entry: usize,
) -> Result<engine_native_naval::naval_combat_hit::Row, &'static str> {
    engine_native_naval::naval_combat_hit::read(memory, base, entry)
}
fn air_read(
    memory: &World<'_>,
    base: usize,
    entry: usize,
) -> Result<engine_native_naval::naval_combat_air_hit::Row, &'static str> {
    engine_native_naval::naval_combat_air_hit::read(memory, base, entry)
}
fn air_entry(
    memory: &EntryMemory<'_, World<'_>>,
    base: usize,
    entry: usize,
) -> Result<engine_native_naval::naval_combat_air_hit::Row, &'static str> {
    engine_native_naval::naval_combat_air_hit::read(memory, base, entry)
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

#[test]
fn naval_hit_runs_keep_the_parent_walks_result_under_every_lasting_fault() {
    check(
        &naval_world(),
        8,
        Some(&engine_native_naval::naval_combat_hit::RECORD),
        naval_read,
        naval_entry,
    );
}

#[test]
fn air_hit_runs_keep_the_parent_walks_result_under_every_lasting_fault() {
    check(
        &air_world(),
        8,
        Some(&engine_native_naval::naval_combat_air_hit::RECORD),
        air_read,
        air_entry,
    );
}

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
        Some(&engine_native_naval::naval_combat_hit::RECORD),
        naval_read,
        naval_entry,
    );
}

#[test]
fn air_hit_runs_are_never_harsher_under_faults_that_clear() {
    check_once(
        &air_world(),
        8,
        Some(&engine_native_naval::naval_combat_air_hit::RECORD),
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
