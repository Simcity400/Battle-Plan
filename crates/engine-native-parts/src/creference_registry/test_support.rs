use super::*;
use engine_native_memory::Memory;
use crate::reference_bucket as bucket;
use std::collections::BTreeMap;

pub(super) const TABLE: usize = 0x1000;
pub(super) const ENTRIES_ADDRESS: usize = 0x2000;

#[derive(Default)]
pub struct Fixture {
    pub(super) bytes: BTreeMap<usize, u8>,
}

impl Fixture {
    pub fn put(&mut self, at: usize, value: &[u8]) {
        self.bytes
            .extend(value.iter().enumerate().map(|(i, &b)| (at + i, b)));
    }

    pub(super) fn root(kind: u32) -> usize {
        if kind < LOW_ROOT_COUNT {
            LOW_ROOTS_RVA.0 + kind as usize * 8
        } else if kind < MID_TYPE_LIMIT {
            MID_ROOT_RVA.0
        } else {
            HIGH_ROOT_RVA.0
        }
    }

    pub fn new(kind: u32) -> Self {
        let mut result = Self::default();
        result.put(Self::root(kind), &(TABLE as u64).to_le_bytes());
        result.put(TABLE + ENTRIES, &(ENTRIES_ADDRESS as u64).to_le_bytes());
        result.put(TABLE + COUNT, &0_u32.to_le_bytes());
        result.put(TABLE + MASK, &7_u32.to_le_bytes());
        result.put(TABLE + OVERFLOW_DISTANCE, &[5]);
        result.put(ENTRIES_ADDRESS, &[0; 14 * bucket::STRIDE]);
        result.put(
            ENTRIES_ADDRESS + 13 * bucket::STRIDE + bucket::DISTANCE,
            &[255],
        );
        result
    }

    pub fn index(handle: u64) -> usize {
        let hash = handle.rotate_left(32).wrapping_mul(HASH_MULTIPLIER);
        (((hash >> 32) as u32 ^ hash as u32) & 7) as usize
    }

    pub fn entry(&mut self, index: usize, distance: u8, handle: u64, object: u64) {
        let at = ENTRIES_ADDRESS + index * bucket::STRIDE;
        self.put(at + bucket::DISTANCE, &[distance]);
        self.put(at + bucket::TYPE, &handle.to_le_bytes());
        self.put(at + bucket::OBJECT, &object.to_le_bytes());
        self.put(at + bucket::HASH, &0xdead_beef_u32.to_le_bytes());
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (i, b) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + i)) else {
                return false;
            };
            *b = *value;
        }
        true
    }
}
