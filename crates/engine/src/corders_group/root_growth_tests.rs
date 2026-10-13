use super::*;
use crate::{memory::SliceMemory, reference_bucket};
use std::cell::RefCell;

struct World {
    bytes: Vec<u8>,
    header_reads: RefCell<usize>,
    fault: Option<usize>,
}

const HEAP: usize = 0x1_0000;
const TABLE: usize = HEAP;
const BUCKETS: usize = HEAP + 0x100;
const GROUPS: usize = HEAP + 0x10_0000;

impl World {
    fn new(count: usize) -> Self {
        // Native growth never builds fewer than 2^MINIMUM_EXPONENT buckets.
        let buckets = count
            .next_power_of_two()
            .max(1 << registry::MINIMUM_EXPONENT);
        let distance = buckets.trailing_zeros() as u8 + 2;
        let mut world = Self {
            bytes: vec![0; GROUPS - HEAP + count * 0x1000],
            header_reads: RefCell::new(0),
            fault: None,
        };
        world.put(TABLE + registry::ENTRIES, &(BUCKETS as u64).to_le_bytes());
        world.put(TABLE + registry::COUNT, &(count as u32).to_le_bytes());
        world.put(
            TABLE + registry::MASK,
            &((buckets - 1) as u32).to_le_bytes(),
        );
        world.put(TABLE + registry::OVERFLOW_DISTANCE, &[distance]);
        world.put(
            BUCKETS
                + (buckets + distance as usize) * reference_bucket::STRIDE
                + reference_bucket::DISTANCE,
            &[reference_bucket::TERMINAL_DISTANCE],
        );
        for index in 0..count {
            let group = GROUPS + index * 0x1000;
            let handle = ((index as u64 + 1) << 32) | REFERENCE_TYPE as u64;
            let bucket = BUCKETS + index * reference_bucket::STRIDE;
            world.put(bucket + reference_bucket::DISTANCE, &[1]);
            world.put(bucket + reference_bucket::TYPE, &handle.to_le_bytes());
            world.put(
                bucket + reference_bucket::OBJECT,
                &(group as u64).to_le_bytes(),
            );
            world.put(group, &(VTABLE_RVA as u64).to_le_bytes());
            world.put(group + HANDLE, &handle.to_le_bytes());
            world.put(group + COUNTRY, &((index % 5_000 + 1) as u32).to_le_bytes());
        }
        world
    }

    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes[address - HEAP..address - HEAP + bytes.len()].copy_from_slice(bytes);
    }
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.fault == Some(address) {
            return false;
        }
        if address == registry::LOW_ROOTS_RVA + REFERENCE_TYPE as usize * 8 && output.len() == 8 {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
            return true;
        }
        if address == TABLE + registry::ENTRIES && output.len() == 17 {
            *self.header_reads.borrow_mut() += 1;
        }
        SliceMemory {
            base: HEAP,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

#[test]
fn registry_and_counter_rows_grow_on_first_and_later_unchanged_country_population() {
    let mut roots = vec![Root::default(); 1];
    let mut counters = Vec::new();
    for count in [5_000, 7_500] {
        let world = World::new(count);
        let source = walk_roots_growing(&world, 0, &mut roots);
        assert_eq!(source, Outcome::Rows { len: count });
        assert_eq!(*world.header_reads.borrow(), 1);
        assert_eq!(roots[count - 1].country, ((count - 1) % 5_000 + 1) as u32);
        crate::allocation::resize_default(&mut counters, count).unwrap();
        let outcome = walk_counter_inputs(&world, 0, &roots[..count], source, &mut counters);
        assert_eq!(outcome, Outcome::Rows { len: count });
        assert_eq!(counters[count - 1].group, roots[count - 1].handle);
    }
    let world = World::new(5_000);
    assert_eq!(
        walk_roots(&world, 0, &mut vec![Root::default(); 4_096]),
        Outcome::Truncated {
            len: 4_096,
            unread_rows: 904
        }
    );
}

#[test]
fn worker_growth_does_not_retry_a_failed_original_group_identity() {
    let mut world = World::new(5_000);
    world.fault = Some(GROUPS + 2 * 0x1000);
    assert_eq!(
        walk_roots_growing(&world, 0, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "orders group identity unreadable"
        }
    );
    assert_eq!(*world.header_reads.borrow(), 1);
}

/// The census takes each group's identity and country in one guarded gather.
#[test]
fn the_census_gathers_each_group_identity_and_country_once() {
    /// Logs a copy's address and a gather's source pointers: one entry per
    /// guarded call into a group.
    struct Calls<'a>(&'a World, RefCell<Vec<usize>>);
    impl Memory for Calls<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.1.borrow_mut().push(address);
            self.0.copy(address, output)
        }
        fn copy_records(
            &self,
            sources: crate::memory::RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            if let crate::memory::RecordSources::Pointers(pointers) = sources {
                self.1.borrow_mut().extend_from_slice(pointers);
            }
            self.0.copy_records(sources, layout, destination)
        }
    }
    let world = World::new(3);
    let calls = Calls(&world, RefCell::default());
    let mut roots = Vec::new();
    assert_eq!(
        walk_roots_growing(&calls, 0, &mut roots),
        Outcome::Rows { len: 3 }
    );
    for (index, root) in roots.iter().take(3).enumerate() {
        let group = GROUPS + index * 0x1000;
        assert_eq!(root.country, index as u32 + 1);
        assert_eq!(
            calls
                .1
                .borrow()
                .iter()
                .filter(|&&at| at == group || at == group + COUNTRY)
                .count(),
            1
        );
    }
}

/// Logs each gather's source pointers.
struct Gathers<'a, M>(&'a M, RefCell<Vec<Vec<usize>>>);
impl<M: Memory> Memory for Gathers<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
    fn copy_records(
        &self,
        sources: crate::memory::RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        if let crate::memory::RecordSources::Pointers(pointers) = sources {
            self.1.borrow_mut().push(pointers.to_vec());
        }
        self.0.copy_records(sources, layout, destination)
    }
}

/// Fails the first copy that covers `at`, then reads it as the world does.
struct OneShot<'a> {
    world: &'a World,
    at: usize,
    spent: std::cell::Cell<bool>,
}
impl Memory for OneShot<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address <= self.at && self.at < address + output.len() && !self.spent.replace(true) {
            return false;
        }
        self.world.copy(address, output)
    }
}

/// A chunk's groups are gathered in one guarded copy, in bucket order.
#[test]
fn a_chunks_groups_are_gathered_in_one_copy_in_bucket_order() {
    let world = World::new(3);
    let memory = Gathers(&world, RefCell::default());
    assert_eq!(
        walk_roots_growing(&memory, 0, &mut Vec::new()),
        Outcome::Rows { len: 3 }
    );
    let groups: Vec<usize> = (0..3).map(|index| GROUPS + index * 0x1000).collect();
    assert_eq!(memory.1.into_inner(), [groups]);
}

/// At 5,000 groups, one gather per chunk of 64 buckets gives every root:
/// handle-sorted, with its pointer and country, as one gather per group did.
#[test]
fn five_thousand_groups_take_one_gather_per_chunk_and_every_root() {
    let world = World::new(5_000);
    let memory = Gathers(&world, RefCell::default());
    let mut roots = Vec::new();
    assert_eq!(
        walk_roots_growing(&memory, 0, &mut roots),
        Outcome::Rows { len: 5_000 }
    );
    for (index, root) in roots[..5_000].iter().enumerate() {
        assert_eq!(
            *root,
            Root {
                pointer: GROUPS + index * 0x1000,
                handle: ((index as u64 + 1) << 32) | REFERENCE_TYPE as u64,
                country: (index % 5_000 + 1) as u32,
            }
        );
    }
    let gathers = memory.1.into_inner();
    assert_eq!(gathers.len(), 5_000_usize.div_ceil(64));
    assert!(gathers.iter().all(|pointers| pointers.len() <= 64));
}

/// A persistent fault in a group's country gives the single walk's failure;
/// one that clears after its first touch refuses nothing: the chunk's
/// groups from the failed one on are read alone, once, and the census is
/// complete.
#[test]
fn a_gather_fault_falls_back_to_the_single_reads_once() {
    let at = GROUPS + 70 * 0x1000 + COUNTRY;
    let mut world = World::new(200);
    world.fault = Some(at);
    assert_eq!(
        walk_roots_growing(&world, 0, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "orders group country unreadable"
        }
    );
    world.fault = None;
    let one_shot = OneShot {
        world: &world,
        at,
        spent: Default::default(),
    };
    let memory = Gathers(&one_shot, RefCell::default());
    let before = crate::batch_fallback::count(crate::batch_fallback::Batch::OrderRoots);
    let mut roots = Vec::new();
    assert_eq!(
        walk_roots_growing(&memory, 0, &mut roots),
        Outcome::Rows { len: 200 }
    );
    assert_eq!(
        crate::batch_fallback::count(crate::batch_fallback::Batch::OrderRoots),
        before + 1
    );
    assert_eq!(roots[70].country, 71);
    // The second chunk's gather stopped at group 70; groups 70 to 127 were
    // then read alone, and the other chunks' groups only in their gathers.
    let gathers = memory.1.into_inner();
    let alone: Vec<usize> = gathers
        .iter()
        .filter(|pointers| pointers.len() == 1)
        .map(|pointers| pointers[0])
        .collect();
    assert_eq!(
        alone,
        (70..128)
            .map(|index| GROUPS + index * 0x1000)
            .collect::<Vec<_>>()
    );
}
