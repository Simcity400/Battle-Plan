//! Repeated reads of one accumulator's tree. [`capture`](super::capture)
//! walks the same accumulator once per requested key: every key reads the
//! same node headers, child pointers, display strings and accumulator entry
//! arrays again. Within the quiescent hourly callback a read that succeeded
//! returns the same bytes when repeated, so a repeat is served from the
//! bytes the first read returned. Nothing is read earlier or in another
//! order: a read the walk makes for the first time goes to the source as it
//! did, and a read that failed is not kept, so its repeat meets the source
//! again, as it did. Storage is advisory: a read it cannot keep is simply
//! made again.

use crate::{
    Memory,
    memory::{RecordLayout, RecordSources},
    modifier_accumulator::{self, Value},
};
use std::cell::RefCell;

/// A kept read: `len` bytes from `address` at `bytes[start..]`, or, with
/// [`ARRAY`], an accumulator's entry array of `len` bytes. Tag zero: free.
#[derive(Clone, Copy, Default)]
struct Slot {
    address: usize,
    tag: u32,
    start: usize,
    len: usize,
}

/// The tag of a kept accumulator entry array.
const ARRAY: u32 = u32::MAX;

/// The reads one accumulator's walks repeat, kept between hours so their
/// storage is reused. [`Self::clear`] it between accumulators.
#[derive(Default)]
pub struct RepeatCache {
    /// Open addressing by address and tag, a power of two in length.
    slots: Vec<Slot>,
    used: usize,
    bytes: Vec<u8>,
    /// An entry array being read.
    scratch: Vec<u8>,
}

impl RepeatCache {
    pub fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot>() + self.bytes.capacity() + self.scratch.capacity()
    }

    /// Forgets every kept read; the storage stays.
    pub fn clear(&mut self) {
        if self.used != 0 {
            self.slots.fill(Slot::default());
        }
        self.used = 0;
        self.bytes.clear();
    }

    fn slot(&self, address: usize, tag: u32) -> usize {
        let mask = self.slots.len() - 1;
        let hash = (address ^ (tag as usize).rotate_left(29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut at = (hash >> 7) & mask;
        while self.slots[at].tag != 0
            && (self.slots[at].address != address || self.slots[at].tag != tag)
        {
            at = (at + 1) & mask;
        }
        at
    }

    fn get(&self, address: usize, tag: u32) -> Option<&[u8]> {
        if self.slots.is_empty() {
            return None;
        }
        let slot = self.slots[self.slot(address, tag)];
        (slot.tag != 0).then(|| &self.bytes[slot.start..slot.start + slot.len])
    }

    /// Keeps `bytes` for `(address, tag)`, which holds nothing; nothing when
    /// the storage cannot grow.
    fn keep(&mut self, address: usize, tag: u32, bytes: &[u8]) {
        if (self.used + 1) * 2 > self.slots.len() && !self.grow() {
            return;
        }
        if crate::allocation::advisory(|| crate::allocation::reserve(&mut self.bytes, bytes.len()))
            .is_err()
        {
            return;
        }
        let at = self.slot(address, tag);
        self.slots[at] = Slot {
            address,
            tag,
            start: self.bytes.len(),
            len: bytes.len(),
        };
        self.bytes.extend_from_slice(bytes);
        self.used += 1;
    }

    /// Doubles the slots, keeping every kept read.
    fn grow(&mut self) -> bool {
        let len = (self.slots.len() * 2).max(64);
        let Ok(slots) =
            crate::allocation::advisory(|| crate::allocation::filled(len, Slot::default()))
        else {
            return false;
        };
        let old = std::mem::replace(&mut self.slots, slots);
        for slot in old.into_iter().filter(|slot| slot.tag != 0) {
            let at = self.slot(slot.address, slot.tag);
            self.slots[at] = slot;
        }
        true
    }
}

/// `inner` with the reads [`RepeatCache`] keeps served again.
pub(super) struct Repeats<'a, M> {
    pub inner: &'a M,
    pub cache: RefCell<&'a mut RepeatCache>,
}

impl<M: Memory> Memory for Repeats<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(tag) = u32::try_from(destination.len())
            .ok()
            .filter(|&len| len != 0 && len != ARRAY)
        else {
            return self.inner.copy(address, destination);
        };
        let mut cache = self.cache.borrow_mut();
        if let Some(bytes) = cache.get(address, tag) {
            destination.copy_from_slice(bytes);
            return true;
        }
        if !self.inner.copy(address, destination) {
            return false;
        }
        cache.keep(address, tag, destination);
        true
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
        self.inner.modifier_definitions()
    }

    /// An accumulator's entry array, read whole by the first lookup there
    /// (`modifier_accumulator::lookup_many_at`, the reads `lookup_at`
    /// makes) and searched for later keys as that lookup would find them.
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        if let Some(result) = self.inner.modifier_accumulator_value(address, key) {
            return Some(result);
        }
        let mut cache = self.cache.borrow_mut();
        if let Some(entries) = cache.get(address, ARRAY) {
            return Some(modifier_accumulator::lookup_snapshot(entries, key));
        }
        let mut entries = std::mem::take(&mut cache.scratch);
        entries.clear();
        let mut kept = true;
        let mut values = [Value::default()];
        let result = modifier_accumulator::lookup_many_at_keeping(
            self.inner,
            address,
            &[key],
            &mut values,
            &mut |batch| {
                kept = kept
                    && crate::allocation::advisory(|| {
                        crate::allocation::reserve(&mut entries, batch.len())
                    })
                    .is_ok();
                if kept {
                    entries.extend_from_slice(batch);
                }
            },
        );
        if result.is_ok() && kept {
            cache.keep(address, ARRAY, &entries);
        }
        cache.scratch = entries;
        Some(result.map(|()| values[0]))
    }
}
