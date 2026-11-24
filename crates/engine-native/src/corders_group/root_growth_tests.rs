use super::*;
use engine_native_memory::memory::SliceMemory;
use engine_native_parts::reference_bucket;
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
            world.put(group, &(VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(group + HANDLE, &handle.to_le_bytes());
            world.put(group + COUNTRY, &(index as u32 + 1).to_le_bytes());
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
        if address == registry::LOW_ROOTS_RVA.0 + REFERENCE_TYPE as usize * 8 && output.len() == 8 {
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
    for count in [50, 75] {
        let world = World::new(count);
        let source = walk_roots(&world, 0, &mut roots);
        assert_eq!(source, Outcome::Rows { len: count });
        assert_eq!(*world.header_reads.borrow(), 1);
        assert_eq!(roots[count - 1].country, count as u32);
        engine_allocation::resize_default(&mut counters, count).unwrap();
        let outcome = walk_counter_inputs(&world, 0, &roots[..count], source, &mut counters);
        assert_eq!(outcome, Outcome::Rows { len: count });
        assert_eq!(counters[count - 1].group, roots[count - 1].handle);
    }
}

#[test]
fn worker_growth_does_not_retry_a_failed_original_group_identity() {
    let mut world = World::new(50);
    world.fault = Some(GROUPS + 2 * 0x1000);
    assert_eq!(
        walk_roots(&world, 0, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "orders group identity unreadable"
        }
    );
    assert_eq!(*world.header_reads.borrow(), 1);
}

