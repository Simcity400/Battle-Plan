//! Lifetime-owned raw payload, observer store and index capacity accounting.
//!
//! Charges move with their allocations, so idle, callback-owned, queued,
//! decoding and retired storage is counted once without ownership-event or
//! per-read instrumentation. Constructors, growth and index preparation run
//! off the game thread; dropping an empty fallback charge does no work.

use std::mem::size_of;
use std::sync::Mutex;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capacity {
    pub payload_vectors: u64,
    pub auxiliary_vectors: u64,
    pub index_vectors: u64,
    pub index_map_entries_lower_bound: u64,
}

impl Capacity {
    fn total(self) -> u64 {
        self.payload_vectors
            + self.auxiliary_vectors
            + self.index_vectors
            + self.index_map_entries_lower_bound
    }

    fn replace(&mut self, previous: Self, next: Self) {
        self.payload_vectors =
            self.payload_vectors - previous.payload_vectors + next.payload_vectors;
        self.auxiliary_vectors =
            self.auxiliary_vectors - previous.auxiliary_vectors + next.auxiliary_vectors;
        self.index_vectors = self.index_vectors - previous.index_vectors + next.index_vectors;
        self.index_map_entries_lower_bound = self.index_map_entries_lower_bound
            - previous.index_map_entries_lower_bound
            + next.index_map_entries_lower_bound;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub current: Capacity,
    pub allocated_index_owners: u64,
    pub peak_total_lower_bound: u64,
}

#[derive(Debug)]
pub(crate) struct Tracker {
    snapshot: Mutex<Snapshot>,
    #[cfg(test)]
    updates: std::sync::atomic::AtomicU64,
}

impl Tracker {
    pub(crate) const fn new() -> Self {
        Self {
            snapshot: Mutex::new(Snapshot {
                current: Capacity {
                    payload_vectors: 0,
                    auxiliary_vectors: 0,
                    index_vectors: 0,
                    index_map_entries_lower_bound: 0,
                },
                allocated_index_owners: 0,
                peak_total_lower_bound: 0,
            }),
            #[cfg(test)]
            updates: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn replace(&self, previous: Capacity, next: Capacity) {
        #[cfg(test)]
        self.updates
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Count allocation owners at the same checkpoints as their vector
        // charges. Moving or reusing a charge does not create another owner;
        // an empty logical shape still counts while its capacity is retained.
        match (previous.index_vectors != 0, next.index_vectors != 0) {
            (false, true) => snapshot.allocated_index_owners += 1,
            (true, false) => snapshot.allocated_index_owners -= 1,
            _ => {}
        }
        snapshot.current.replace(previous, next);
        snapshot.peak_total_lower_bound = snapshot
            .peak_total_lower_bound
            .max(snapshot.current.total());
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        *self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

static TRACKER: Tracker = Tracker::new();

/// A non-cloneable accounting owner, not an allocation or a data wrapper.
/// Moving it transfers the charge unchanged; replacement and destruction
/// release only this owner's prior capacity. Keep it beside the owned vectors.
#[derive(Debug)]
pub(crate) struct Charge {
    tracker: &'static Tracker,
    capacity: Capacity,
}

impl Default for Charge {
    fn default() -> Self {
        Self::empty_on(&TRACKER)
    }
}

impl Charge {
    pub(crate) fn empty_on(tracker: &'static Tracker) -> Self {
        Self {
            tracker,
            capacity: Capacity::default(),
        }
    }

    pub(crate) fn tracker(&self) -> &'static Tracker {
        self.tracker
    }

    pub(crate) fn set(&mut self, capacity: Capacity) {
        if capacity != self.capacity {
            self.tracker.replace(self.capacity, capacity);
            self.capacity = capacity;
        }
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.set(Capacity::default());
    }
}

/// Element capacity, including unused reserved elements; not allocator rounding.
pub(crate) fn vector_bytes<T>(values: &Vec<T>) -> u64 {
    values.capacity() as u64 * size_of::<T>() as u64
}

pub(crate) fn diagnostics() -> impl serde::Serialize {
    // Copy the tracker counters while its lock is held.
    let snapshot = TRACKER.snapshot();
    let current = snapshot.current;
    crate::status::status!({
        "scope": "owned raw payload, tracked observer stores and decode-index capacities across idle, in-flight and retired objects; not process memory or an allocator-level bound",
        "current": {
            "payload_vector_capacity_bytes": current.payload_vectors,
            "auxiliary_vector_capacity_bytes": current.auxiliary_vectors,
            "index_vector_capacity_bytes": current.index_vectors,
            "allocated_index_owners": snapshot.allocated_index_owners,
            "index_map_entry_capacity_lower_bound_bytes": current.index_map_entries_lower_bound,
            "total_capacity_lower_bound_bytes": current.total(),
        },
        "peak_total_capacity_lower_bound_bytes": snapshot.peak_total_lower_bound,
        "peak_scope": "aggregate live charges after constructor/growth/index-preparation checkpoints; not a sum of component peaks",
        "excluded": [
            "HashMap bucket/control storage beyond the entry-capacity lower bound",
            "allocator metadata, rounding, and transient overlap during reallocation",
            "inline job/index objects, pool-vector backing storage, decode/retirement queues and other static/container metadata",
            "typed frame pools, spool/encoder allocations, worker stacks and game/process memory",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_follow_moves_replacement_growth_and_drop_without_summing_peaks() {
        static LOCAL: Tracker = Tracker::new();
        let mut payload = Charge::empty_on(&LOCAL);
        payload.set(Capacity {
            payload_vectors: 100,
            ..Capacity::default()
        });
        let mut moved = Some(payload);
        assert_eq!(LOCAL.snapshot().current.total(), 100);
        let mut payload = moved.take().unwrap();
        drop(moved);
        payload.set(Capacity {
            payload_vectors: 160,
            ..Capacity::default()
        });
        payload.set(Capacity::default());
        let mut index = Charge::empty_on(&LOCAL);
        index.set(Capacity {
            index_vectors: 70,
            index_map_entries_lower_bound: 40,
            ..Capacity::default()
        });
        assert_eq!(LOCAL.snapshot().peak_total_lower_bound, 160);
        payload = index;
        assert_eq!(LOCAL.snapshot().current.total(), 110);
        drop(payload);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert_eq!(LOCAL.snapshot().peak_total_lower_bound, 160);
    }

    #[test]
    fn empty_and_unchanged_charges_do_not_acquire_the_tracker_lock() {
        use std::sync::atomic::Ordering;
        static LOCAL: Tracker = Tracker::new();
        let mut empty = Charge::empty_on(&LOCAL);
        let mut charged = Charge::empty_on(&LOCAL);
        let capacity = Capacity {
            auxiliary_vectors: 32,
            ..Capacity::default()
        };
        charged.set(capacity);
        let updates = LOCAL.updates.load(Ordering::Relaxed);
        empty.set(Capacity::default());
        drop(empty);
        charged.set(capacity);
        assert_eq!(LOCAL.updates.load(Ordering::Relaxed), updates);
        drop(charged);
        assert_eq!(LOCAL.updates.load(Ordering::Relaxed), updates + 1);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

    #[test]
    fn vector_accounting_uses_capacity_including_cleared_and_zero_sized_storage() {
        let mut values = Vec::<u32>::with_capacity(19);
        let bytes = vector_bytes(&values);
        values.extend([1, 2, 3]);
        assert_eq!(vector_bytes(&values), bytes);
        values.clear();
        assert_eq!(vector_bytes(&values), bytes);
        assert_eq!(bytes, values.capacity() as u64 * 4);
        assert_eq!(vector_bytes(&Vec::<()>::with_capacity(10)), 0);
    }
}
