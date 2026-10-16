//! First exact-span observations of one stability walk. Guarded speculative
//! batches retain only complete successes; a failed batch yields to the serial
//! readers once. Their authoritative failures remain memoized without retries.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, hash_map::Entry},
    hash::{BuildHasherDefault, Hasher},
};

use crate::{
    Memory,
    memory::{RecordLayout, RecordSources},
};

// Transfer geometry only: later chunks and larger payloads continue normally.
const BATCH_FIELDS: usize = 64;
const BATCH_BYTES: usize = 8192;

/// Internal integer keys only. Avalanche both the low bucket-index bits and
/// the high tag bits: aligned native addresses must not cluster by low bits.
#[derive(Default)]
struct SpanHasher(u64);

impl Hasher for SpanHasher {
    fn finish(&self) -> u64 {
        let mut hash = self.0;
        hash = (hash ^ (hash >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash = (hash ^ (hash >> 33)).wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        hash ^ (hash >> 33)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

/// Shared static observations of one hourly Stability job. The caller clears
/// this before every hour, including failed, skipped, load and full hours.
/// Static modifier binding selects the shared CStaticModifiers; Static modifier
/// lookup owns their +0x1A8 names. Neither country-owned operands nor
/// source-selection pointers enter this memo.
#[derive(Debug, Default)]
pub struct Memo {
    spans: HashMap<(usize, usize), Option<usize>, BuildHasherDefault<SpanHasher>>,
    bytes: Vec<u8>,
    // Pending gather keys deduplicate immediately, but their bytes are not
    // observations until the whole chunk succeeds. Prepared never follows
    // a control whose speculative copy has not completed.
    acquired: usize,
}

impl Memo {
    pub fn clear(&mut self) {
        self.spans.clear();
        self.bytes.clear();
        self.acquired = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.spans.capacity() * std::mem::size_of::<((usize, usize), Option<usize>)>()
    }

    #[cfg(test)]
    pub(super) fn retained(&self) -> (usize, usize) {
        (self.bytes.len(), self.spans.len())
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> Option<bool> {
        let start = *self.spans.get(&(address, destination.len()))?;
        match start {
            Some(start) if start < self.acquired => {
                destination.copy_from_slice(&self.bytes[start..start + destination.len()]);
                Some(true)
            }
            Some(_) => None,
            None => Some(false),
        }
    }

    /// Most walks reuse capacity. Enter the advisory scope only for actual
    /// growth; every entry insertion then has checked, fallible map storage.
    fn reserve(&mut self, spans: usize, bytes: usize) -> bool {
        if spans <= self.spans.capacity() - self.spans.len()
            && bytes <= self.bytes.capacity() - self.bytes.len()
        {
            return true;
        }
        crate::allocation::advisory(|| {
            crate::allocation::reserve_map(&mut self.spans, spans).ok()?;
            crate::allocation::reserve(&mut self.bytes, bytes).ok()?;
            Some(())
        })
        .is_some()
    }

    /// Room for `key`'s entry. An existing key needs none, so a full map grows
    /// (and a denied growth stops speculation) only for a new span.
    fn room_for(&mut self, key: (usize, usize)) -> bool {
        self.spans.len() < self.spans.capacity()
            || self.spans.contains_key(&key)
            || self.reserve(1, 0)
    }

    /// Keep the first observation of an exact span. Retention is optional
    /// storage: when it cannot allocate, nothing (no partial bytes) is kept
    /// and the observation stands as acquired, so a later use reads the span
    /// as the walk without the memo did. A failed copy ends the country's
    /// walk, so within a walk it is never read again either way.
    fn retain(&mut self, address: usize, bytes: &[u8], copied: bool) -> bool {
        let key = (address, bytes.len());
        if bytes.is_empty() {
            return true;
        }
        if !self.room_for(key) {
            return false;
        }
        let Entry::Vacant(entry) = self.spans.entry(key) else {
            return true;
        };
        let start = if copied {
            if bytes.len() > self.bytes.capacity() - self.bytes.len()
                && crate::allocation::advisory(|| {
                    crate::allocation::reserve(&mut self.bytes, bytes.len())
                })
                .is_err()
            {
                return false;
            }
            let start = self.bytes.len();
            self.bytes.extend_from_slice(bytes);
            self.acquired = self.bytes.len();
            Some(start)
        } else {
            None
        };
        entry.insert(start);
        true
    }
}

struct Chunk {
    spans: [(usize, usize); BATCH_FIELDS],
    count: usize,
    start: usize,
}

impl Chunk {
    fn new(start: usize) -> Self {
        Self {
            spans: [(0, 0); BATCH_FIELDS],
            count: 0,
            start,
        }
    }

    fn discard(&self, memo: &mut Memo) {
        for key in &self.spans[..self.count] {
            memo.spans.remove(key);
        }
        memo.bytes.truncate(self.start);
    }
}

/// Ordinary observations are country-local. Only an explicitly selected
/// static's source or identity reader uses `shared`; subsequent consumers may
/// reuse those exact spans. Context::read already reads slots, defines,
/// modifier metadata and language once per hour.
pub(super) struct Observed<'a, M> {
    source: &'a M,
    local: RefCell<&'a mut Memo>,
    shared: RefCell<&'a mut Memo>,
    fallbacks: Cell<u64>,
    stopped: Cell<bool>,
}

impl<'a, M: Memory> Observed<'a, M> {
    pub(super) fn new(source: &'a M, local: &'a mut Memo, shared: &'a mut Memo) -> Self {
        local.clear();
        Self {
            source,
            local: RefCell::new(local),
            shared: RefCell::new(shared),
            fallbacks: Cell::new(0),
            stopped: Cell::new(false),
        }
    }

    pub(super) fn shared(&self) -> Shared<'_, 'a, M> {
        Shared(self)
    }

    /// Preparation may follow retained controls, but must never acquire a
    /// missing scalar ahead of the authoritative serial object reader.
    pub(super) fn prepared(&self) -> Prepared<'_, 'a, M> {
        Prepared(self)
    }

    pub(super) fn fallbacks(&self) -> u64 {
        self.fallbacks.get()
    }

    /// Exact spans reached through acquired pointers/headers. Bounded transfer
    /// pages are throughput units, never limits on objects, entries or text.
    /// Reserve optional retention BEFORE reading: if it cannot be kept, the
    /// original reader acquires the span itself, with no speculative attempt.
    pub(super) fn gather(&self, spans: impl IntoIterator<Item = (usize, usize)>, shared: bool) {
        // Overlapping later preparation must not speculate on a failed batch
        // again. The rest of this country uses its original readers; successful
        // observations already retained still stand. No retry loop is possible.
        if self.stopped.get() {
            return;
        }
        // No metadata is initialized until a missing, admissible span is
        // reached. Iterator preparation may itself consult retained controls,
        // so release both memo borrows before advancing it.
        let mut chunk: Option<Chunk> = None;
        for (address, len) in spans {
            if len == 0 || len > BATCH_BYTES || address == 0 || address.checked_add(len).is_none() {
                continue;
            }
            let (target, other) = if shared {
                (&self.shared, &self.local)
            } else {
                (&self.local, &self.shared)
            };
            if other.borrow().spans.contains_key(&(address, len)) {
                continue;
            }
            let mut memo = target.borrow_mut();
            if !memo.room_for((address, len)) {
                if let Some(chunk) = chunk.as_ref() {
                    chunk.discard(&mut memo);
                }
                self.stopped.set(true);
                return;
            }
            let memo = &mut **memo;
            let Entry::Vacant(entry) = memo.spans.entry((address, len)) else {
                continue;
            };
            if len > memo.bytes.capacity() - memo.bytes.len()
                && crate::allocation::advisory(|| crate::allocation::reserve(&mut memo.bytes, len))
                    .is_err()
            {
                // The vacant entry holds no key yet. Remove only this pending
                // chunk; earlier completed chunks remain observations.
                if let Some(chunk) = chunk.as_ref() {
                    chunk.discard(memo);
                }
                self.stopped.set(true);
                return;
            }
            let start = memo.bytes.len();
            let pending = chunk.get_or_insert_with(|| Chunk::new(start));
            memo.bytes.resize(start + len, 0);
            entry.insert(Some(start));
            pending.spans[pending.count] = (address, len);
            pending.count += 1;
            if pending.count == BATCH_FIELDS {
                if !self.gather_chunk(memo, pending) {
                    return;
                }
                chunk = None;
            }
        }
        if let Some(chunk) = chunk {
            let mut memo = if shared {
                self.shared.borrow_mut()
            } else {
                self.local.borrow_mut()
            };
            self.gather_chunk(&mut memo, &chunk);
        }
    }

    fn gather_chunk(&self, memo: &mut Memo, chunk: &Chunk) -> bool {
        let addresses = std::array::from_fn(|index| chunk.spans[index].0);
        let mut remaining = &mut memo.bytes[chunk.start..];
        let mut destinations: [&mut [u8]; BATCH_FIELDS] = std::array::from_fn(|index| {
            let (destination, tail) =
                std::mem::take(&mut remaining).split_at_mut(chunk.spans[index].1);
            remaining = tail;
            destination
        });
        if self.source.copy_scattered(addresses, &mut destinations) != addresses.len() {
            // Nothing from the failed chunk, including its copied prefix, is
            // installed. Earlier successful chunks stand as acquired.
            // The callers' ordinary per-object readers run once, in their own
            // order, locating a persistent failure or recovering a one-shot.
            chunk.discard(memo);
            self.fallbacks.set(self.fallbacks.get().saturating_add(1));
            self.stopped.set(true);
            return false;
        }
        memo.acquired = memo.bytes.len();
        true
    }

    fn copy_observed(&self, address: usize, destination: &mut [u8], shared: bool) -> bool {
        if destination.is_empty() {
            return self.source.copy(address, destination);
        }
        // A shared read still prefers an earlier local observation, preserving
        // the original memo precedence even for overlapping native operands.
        if shared && let Some(copied) = self.local.borrow().copy(address, destination) {
            return copied;
        }
        let target = if shared { &self.shared } else { &self.local };
        let mut memo = target.borrow_mut();
        if !memo.room_for((address, destination.len())) {
            self.stopped.set(true);
            return memo
                .copy(address, destination)
                .or_else(|| {
                    (!shared)
                        .then(|| self.shared.borrow().copy(address, destination))
                        .flatten()
                })
                .unwrap_or_else(|| self.source.copy(address, destination));
        }
        let memo = &mut **memo;
        match memo.spans.entry((address, destination.len())) {
            Entry::Occupied(entry) => match *entry.get() {
                Some(start) => {
                    destination.copy_from_slice(&memo.bytes[start..start + destination.len()]);
                    true
                }
                None => false,
            },
            Entry::Vacant(entry) => {
                if !shared && let Some(copied) = self.shared.borrow().copy(address, destination) {
                    return copied;
                }
                let copied = self.source.copy(address, destination);
                let start = if copied {
                    if destination.len() > memo.bytes.capacity() - memo.bytes.len()
                        && crate::allocation::advisory(|| {
                            crate::allocation::reserve(&mut memo.bytes, destination.len())
                        })
                        .is_err()
                    {
                        self.stopped.set(true);
                        return copied;
                    }
                    let start = memo.bytes.len();
                    memo.bytes.extend_from_slice(destination);
                    memo.acquired = memo.bytes.len();
                    Some(start)
                } else {
                    None
                };
                entry.insert(start);
                copied
            }
        }
    }

    /// Child modifier cloning owns these fields.
    /// Vtable and accumulator controls are consumed from the acquired head
    /// only; matches/same_text and emit re-read the display header alone.
    pub(super) fn copy_child_heads(
        &self,
        sources: RecordSources<'_>,
        destination: &mut [u8],
    ) -> usize {
        let layout = &super::CHILD_HEAD_LAYOUT;
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let completed = self.source.copy_records(sources, layout, destination);
        let fields = layout.fields();
        let mut memo = self.local.borrow_mut();
        for index in (2..count.min(completed.saturating_add(1))).step_by(fields.len()) {
            let row = index / fields.len();
            let field = fields[2];
            let Some(address) = sources
                .pointer(row)
                .filter(|&p| p != 0)
                .and_then(|p| p.checked_add(field.source_offset))
                .filter(|p| p.checked_add(field.len).is_some())
            else {
                break;
            };
            let start = row * layout.stride() + field.destination_offset;
            if !memo.retain(
                address,
                &destination[start..start + field.len],
                index < completed,
            ) {
                self.stopped.set(true);
            }
        }
        completed
    }
}

impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copy_observed(address, destination, false)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        // Keep existing batches and their failure prefixes intact. Save only
        // each completed exact operand (and the attempted failure), in order.
        let completed = self.source.copy_scattered(addresses, destinations);
        let mut memo = self.local.borrow_mut();
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter())
            .take(completed.saturating_add(1))
            .enumerate()
        {
            if !memo.retain(address, destination, index < completed) {
                self.stopped.set(true);
            }
        }
        completed
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let completed = self.source.copy_records(sources, layout, destination);
        let fields = layout.fields();
        let mut memo = self.local.borrow_mut();
        for index in 0..count.min(completed.saturating_add(1)) {
            let row = index / fields.len();
            let field = fields[index % fields.len()];
            let Some(address) = sources
                .pointer(row)
                .filter(|&p| p != 0)
                .and_then(|p| p.checked_add(field.source_offset))
                .filter(|p| p.checked_add(field.len).is_some())
            else {
                break;
            };
            let start = row * layout.stride() + field.destination_offset;
            if !memo.retain(
                address,
                &destination[start..start + field.len],
                index < completed,
            ) {
                self.stopped.set(true);
            }
        }
        completed
    }
}

pub(super) struct Shared<'a, 'b, M>(&'a Observed<'b, M>);

impl<M: Memory> Memory for Shared<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.copy_observed(address, destination, true)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.0.source.prefetch(address, len);
    }
}

pub(super) struct Prepared<'a, 'b, M>(&'a Observed<'b, M>);

impl<M: Memory> Memory for Prepared<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.0.stopped.get() {
            return false;
        }
        self.0
            .local
            .borrow()
            .copy(address, destination)
            .or_else(|| self.0.shared.borrow().copy(address, destination))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One scattered call: its width and its nonempty (address, len) spans.
    type Call = (usize, Vec<(usize, usize)>);

    #[derive(Default)]
    struct GatherMemory {
        calls: RefCell<Vec<Call>>,
        copies: Cell<usize>,
        fail_chunk: Option<usize>,
    }

    impl Memory for GatherMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            destination.fill(address as u8);
            true
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            let chunk = self.calls.borrow().len();
            self.calls.borrow_mut().push((
                N,
                addresses
                    .iter()
                    .zip(destinations.iter())
                    .filter(|(_, bytes)| !bytes.is_empty())
                    .map(|(&address, bytes)| (address, bytes.len()))
                    .collect(),
            ));
            for (index, (&address, bytes)) in
                addresses.iter().zip(destinations.iter_mut()).enumerate()
            {
                if self.fail_chunk == Some(chunk) && index == 3 {
                    // Even a failed operand may have overwritten part of its
                    // destination. Nothing from this chunk can become a memo.
                    bytes.fill(0xee);
                    return index;
                }
                bytes.fill(address as u8);
            }
            N
        }
    }

    #[test]
    fn stability_gather_uses_bounded_chunks_and_keeps_only_complete_chunks() {
        for shared_target in [false, true] {
            for failure in [false, true] {
                let source = GatherMemory {
                    fail_chunk: failure.then_some(1),
                    ..Default::default()
                };
                let mut local = Memo::default();
                let mut shared = Memo::default();
                let memory = Observed::new(&source, &mut local, &mut shared);
                let spans = || (0..130).map(|index| (0x1000 + index * 16, 8));
                memory.gather(spans(), shared_target);
                let calls = source.calls.borrow();
                assert_eq!(
                    calls.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                    if failure {
                        vec![64, 64]
                    } else {
                        vec![64, 64, 64]
                    }
                );
                assert_eq!(
                    calls
                        .iter()
                        .map(|(_, spans)| spans.len())
                        .collect::<Vec<_>>(),
                    if failure {
                        vec![64, 64]
                    } else {
                        vec![64, 64, 2]
                    }
                );
                drop(calls);
                assert_eq!(memory.fallbacks(), u64::from(failure));
                {
                    let memo = if shared_target {
                        memory.shared.borrow()
                    } else {
                        memory.local.borrow()
                    };
                    let retained = if failure { 64 } else { 130 };
                    assert_eq!(memo.retained(), (retained * 8, retained));
                    for (index, (address, len)) in spans().enumerate() {
                        let mut bytes = [0; 8];
                        assert_eq!(
                            memo.copy(address, &mut bytes),
                            (index < retained).then_some(true)
                        );
                        if index < retained {
                            assert_eq!(bytes, [address as u8; 8]);
                            assert_eq!(memo.spans.get(&(address, len)), Some(&Some(index * 8)));
                        }
                    }
                }
                memory.gather(spans(), shared_target);
                assert_eq!(source.calls.borrow().len(), if failure { 2 } else { 3 });
                let mut bytes = [0; 8];
                assert!(memory.copy(0x1000, &mut bytes));
                assert_eq!(
                    source.copies.get(),
                    0,
                    "the completed first chunk still stands"
                );
                if failure {
                    assert!(memory.copy(0x1000 + 65 * 16, &mut bytes));
                    assert_eq!(
                        source.copies.get(),
                        1,
                        "the failed chunk yields once to the serial reader"
                    );
                    assert!(memory.copy(0x1000 + 65 * 16, &mut bytes));
                    assert_eq!(source.copies.get(), 1);
                }
            }
        }
    }

    #[test]
    fn stability_gather_deduplicates_pending_and_other_memo_spans() {
        for shared_target in [false, true] {
            let source = GatherMemory::default();
            let mut local = Memo::default();
            let mut shared = Memo::default();
            let memory = Observed::new(&source, &mut local, &mut shared);
            let mut bytes = [0; 8];
            if shared_target {
                assert!(memory.copy(0x2000, &mut bytes));
            } else {
                assert!(memory.shared().copy(0x2000, &mut bytes));
            }
            // The iterator observes the immediately accepted key before its
            // read, then after its chunk completes. Only complete bytes may
            // authorize later speculative controls.
            let mut index = 0;
            let spans = std::iter::from_fn(|| {
                let span = match index {
                    0 | 1 | 65 => (0x1000, 8),
                    2 | 66 => (0x2000, 8),
                    3..=64 => (0x1000 + (index - 2) * 16, 8),
                    67 => (0x1000 + 63 * 16, 8),
                    68 => (0x1000 + 64 * 16, 8),
                    _ => return None,
                };
                if index == 1 {
                    assert!(!memory.prepared().copy(0x1000, &mut [0; 8]));
                }
                if index == 68 {
                    assert!(memory.prepared().copy(0x1000, &mut [0; 8]));
                }
                index += 1;
                Some(span)
            });
            memory.gather(spans, shared_target);
            let calls = source.calls.borrow();
            assert_eq!(
                calls
                    .iter()
                    .map(|(n, spans)| (*n, spans.len()))
                    .collect::<Vec<_>>(),
                vec![(64, 64), (64, 1)]
            );
            let requested: Vec<_> = calls.iter().flat_map(|(_, spans)| spans).copied().collect();
            assert_eq!(requested.iter().filter(|&&(at, _)| at == 0x1000).count(), 1);
            assert!(requested.iter().all(|&(at, _)| at != 0x2000));
            assert_eq!(source.copies.get(), 1);
            assert_eq!(memory.fallbacks(), 0);
        }
    }

    /// Counts native reads without allocating, for allocation-denied runs.
    #[cfg(feature = "alloc-fault-injection")]
    #[derive(Default)]
    struct Quiet {
        copies: Cell<usize>,
        scattered: Cell<usize>,
    }

    #[cfg(feature = "alloc-fault-injection")]
    impl Memory for Quiet {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            destination.fill(address as u8);
            true
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.scattered.set(self.scattered.get() + 1);
            for (&address, bytes) in addresses.iter().zip(destinations.iter_mut()) {
                bytes.fill(address as u8);
            }
            N
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    #[ignore = "run alone: allocation hooks are process-global"]
    fn stability_full_memo_hit_under_denial_needs_no_growth() {
        for shared_target in [false, true] {
            let source = Quiet::default();
            let mut local = Memo::default();
            let mut shared = Memo::default();
            let memory = Observed::new(&source, &mut local, &mut shared);
            let read = |address: usize, bytes: &mut [u8; 8]| {
                if shared_target {
                    memory.shared().copy(address, bytes)
                } else {
                    memory.copy(address, bytes)
                }
            };
            let full = || {
                let memo = if shared_target {
                    memory.shared.borrow()
                } else {
                    memory.local.borrow()
                };
                memo.spans.len() == memo.spans.capacity()
            };
            let mut bytes = [0; 8];
            // An empty map is also "full" (no capacity): read before testing.
            let mut next = 0x1000;
            loop {
                assert!(read(next, &mut bytes));
                next += 16;
                if full() {
                    break;
                }
            }
            let copies = source.copies.get();
            crate::allocation::test_support::with_allocations_denied(|| {
                assert!(read(0x1000, &mut bytes));
                memory.gather([(0x1000, 8)], shared_target);
            });
            assert_eq!(bytes, [0x00; 8]);
            assert_eq!(source.copies.get(), copies);
            assert_eq!(source.scattered.get(), 0);
            assert!(!memory.stopped.get(), "a hit must not stop speculation");
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    #[ignore = "run alone: allocation hooks are process-global"]
    fn stability_denied_growth_discards_only_the_pending_chunk() {
        for shared_target in [false, true] {
            let source = Quiet::default();
            let mut local = Memo::default();
            let mut shared = Memo::default();
            for memo in [&mut local, &mut shared] {
                memo.spans.reserve(256);
                // A completed chunk and six pending spans fit; the seventh
                // pending span needs byte growth.
                memo.bytes = Vec::with_capacity(70 * 8);
            }
            let memory = Observed::new(&source, &mut local, &mut shared);
            let spans = || (0..80).map(|index| (0x1000 + index * 16, 8));
            crate::allocation::test_support::with_allocations_denied(|| {
                memory.gather(spans(), shared_target);
            });
            assert_eq!(source.scattered.get(), 1);
            assert_eq!(memory.fallbacks(), 0);
            assert!(memory.stopped.get());
            {
                let memo = if shared_target {
                    memory.shared.borrow()
                } else {
                    memory.local.borrow()
                };
                assert_eq!(memo.retained(), (64 * 8, 64));
                for (index, key) in spans().enumerate() {
                    assert_eq!(memo.spans.contains_key(&key), index < 64);
                }
            }
            let mut bytes = [0; 8];
            crate::allocation::test_support::with_allocations_denied(|| {
                assert!(memory.copy(0x1000 + 63 * 16, &mut bytes));
                assert_eq!(source.copies.get(), 0, "the completed chunk stands");
                assert!(memory.copy(0x1000 + 64 * 16, &mut bytes));
                assert_eq!(source.copies.get(), 1, "a discarded span reads serially");
            });
            assert_eq!(bytes, [(0x1000 + 64 * 16) as u8; 8]);
        }
    }
}
