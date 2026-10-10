//! Decode-side replay of owned raw spans.

use super::{RawLog, Span, is_canonical_range};
use engine::Memory;
use std::cell::RefCell;
use std::hash::Hasher;
use std::sync::atomic::{AtomicU64, Ordering};

// Worker-side index preparation only. No timers or atomic updates are added
// to memory lookups or to the game-thread copy path.
static INDEX_REUSES: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILDS: AtomicU64 = AtomicU64::new(0);
static INDEX_REUSE_NS: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILD_NS: AtomicU64 = AtomicU64::new(0);
static INDEX_REUSED_SPANS: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILT_SPANS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn index_diagnostics() -> impl serde::Serialize {
    crate::status::status!({
        "scope": "shared raw indexes since attach; preparation only, not row decoding",
        "reuses": INDEX_REUSES.load(Ordering::Relaxed),
        "rebuilds": INDEX_REBUILDS.load(Ordering::Relaxed),
        "reuse_ns": INDEX_REUSE_NS.load(Ordering::Relaxed),
        "rebuild_ns": INDEX_REBUILD_NS.load(Ordering::Relaxed),
        "reused_spans": INDEX_REUSED_SPANS.load(Ordering::Relaxed),
        "rebuilt_spans": INDEX_REBUILT_SPANS.load(Ordering::Relaxed),
    })
}

/// Decode-side address space: serves exactly the recorded spans and fails
/// every other read, so the replayed walk observes the identical bytes and
/// faults the copy observed. Empty reads succeed and null or overflowing
/// addresses fail without a lookup, matching `Memory`. Logs with bulk spans
/// additionally serve any request fully enclosed in one bulk span.
///
/// Repeated takes of one span are served in record order: the replay issues
/// the same read sequence the copy recorded, so witness-before/after pairs
/// (and any other repeated read) observe their own values rather than the
/// last write. Once the recorded multiplicity is exhausted, the last value
/// keeps serving: shared spans (one census feeding several replays) are
/// taken once and requested many times with identical bytes, while a span
/// the mirror never took has no entry at all and still fails loudly.
pub struct Recorded<'a> {
    pub(super) index: RecordedIndex,
    recycle: &'a RefCell<Option<RecordedIndex>>,
    spans: &'a [Span],
    bytes: &'a [u8],
    pub(super) next_exact: std::cell::Cell<usize>,
    pub(super) last_bulk: std::cell::Cell<usize>,
}

#[derive(Debug, Default)]
pub(super) struct RecordedIndex {
    has_bulk: bool,
    // Every exact key, including failed reads and whole bulk chunks. Widths
    // 0..=254 have individual bits; larger widths share a conservative bucket.
    pub(super) exact_lengths: [u64; 4],
    // Empty buckets use usize::MAX; occupied buckets hold an observation
    // cursor. Advancing that cursor never changes its exact address/length key.
    pub(super) slots: Vec<std::cell::Cell<usize>>,
    pub(super) distinct: usize,
    // Globally unique keys use usize::MAX. Repeated observations form a
    // forward chain whose final entry points to itself, never to that sentinel.
    pub(super) next: Vec<usize>,
    // Only duplicate keys need a saved first observation for reader reset.
    // Bucket, first observation. Bucket positions are resolved after the final
    // build-time rehash, then remain stable throughout reuse and reading.
    pub(super) repeated: Vec<(usize, usize)>,
    // Exact structural comparison, not a hash witness. Payload bytes and
    // byte-pool offsets always come from the current refresh's spans.
    pub(super) shape: Vec<(usize, u32, bool, bool)>,
    // Start, end, current-log span index, maximum end through this entry.
    pub(super) bulk: Vec<(usize, usize, usize, usize)>,
    capacity_charge: crate::raw_memory::Charge,
}

/// Process-local integer read keys, not untrusted strings or file identity.
/// Exact address/length equality still resolves every bucket collision. This
/// cheaper mixer affects lookup cost only; it is never a correctness witness.
#[derive(Debug, Default)]
pub(super) struct ReadAddressHasher(u64);

impl Hasher for ReadAddressHasher {
    fn finish(&self) -> u64 {
        let mut value = self.0;
        value ^= value >> 32;
        value = value.wrapping_mul(0xd6e8_feb8_6659_fd93);
        value ^ (value >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
}

impl RecordedIndex {
    #[inline]
    pub(super) fn mark_exact_length(&mut self, len: u32) {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    pub(super) fn may_have_exact_length(&self, len: u32) -> bool {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    #[inline]
    pub(super) fn hash(address: usize, len: u32) -> usize {
        let mut hash = ReadAddressHasher::default();
        hash.write_usize(address);
        hash.write_u32(len);
        hash.finish() as usize
    }

    /// Return the matching bucket or the first empty bucket. No deletions occur
    /// within a build, so an empty bucket terminates a lookup. Bound the scan
    /// even if an internal invariant were to leave a completely full table.
    #[inline]
    pub(super) fn slot(&self, spans: &[Span], address: usize, len: u32) -> Option<usize> {
        let mask = self.slots.len().checked_sub(1)?;
        let mut slot = Self::hash(address, len) & mask;
        for _ in 0..self.slots.len() {
            let current = self.slots[slot].get();
            if current == usize::MAX {
                return Some(slot);
            }
            let span = &spans[current];
            if span.addr == address && span.len == len {
                return Some(slot);
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    #[inline]
    pub(super) fn cursor(
        &self,
        spans: &[Span],
        address: usize,
        len: u32,
    ) -> Option<&std::cell::Cell<usize>> {
        let cursor = &self.slots[self.slot(spans, address, len)?];
        (cursor.get() != usize::MAX).then_some(cursor)
    }

    fn grow_slots(&mut self, spans: &[Span]) -> std::io::Result<()> {
        let size = if self.slots.is_empty() {
            8
        } else {
            self.slots.len().saturating_mul(2)
        };
        // Allocations and the temporary old table live only on the worker.
        // The retained-capacity diagnostic excludes transient realloc overlap.
        let mut slots = Vec::new();
        crate::allocation::reserve_raw(&mut slots, size, "raw index buckets")?;
        slots.resize(size, std::cell::Cell::new(usize::MAX));
        let old = std::mem::replace(&mut self.slots, slots);
        for cursor in old {
            let current = cursor.get();
            if current != usize::MAX {
                let span = &spans[current];
                let slot = self
                    .slot(spans, span.addr, span.len)
                    .expect("grown raw index must have an empty bucket");
                self.slots[slot].set(current);
            }
        }
        Ok(())
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            index_vectors: vector_bytes(&self.slots)
                + vector_bytes(&self.next)
                + vector_bytes(&self.repeated)
                + vector_bytes(&self.shape)
                + vector_bytes(&self.bulk),
            ..Default::default()
        });
    }

    pub(super) fn prepare(&mut self, spans: &[Span], has_bulk: bool) -> std::io::Result<bool> {
        if self.has_bulk == has_bulk
            && self.shape.len() == spans.len()
            && self
                .shape
                .iter()
                .zip(spans)
                .all(|(shape, span)| *shape == (span.addr, span.len, span.ok, span.bulk))
        {
            for &(slot, first) in &self.repeated {
                self.slots[slot].set(first);
            }
            return Ok(true);
        }
        // Every following append is bounded by the span census. Reserve the
        // whole batch before changing the reusable index's logical shape.
        crate::allocation::reserve_raw(&mut self.next, spans.len(), "raw index next")?;
        crate::allocation::reserve_raw(&mut self.repeated, spans.len(), "raw index repeated")?;
        crate::allocation::reserve_raw(&mut self.shape, spans.len(), "raw index shape")?;
        if has_bulk {
            crate::allocation::reserve_raw(&mut self.bulk, spans.len(), "raw index bulk")?;
        }
        self.slots.fill(std::cell::Cell::new(usize::MAX));
        self.distinct = 0;
        self.has_bulk = has_bulk;
        self.exact_lengths = [0; 4];
        // Grow for distinct read keys, not repeated observations. In the
        // calibration matrix those counts can differ by orders of magnitude.
        self.next.clear();
        self.next.resize(spans.len(), usize::MAX);
        self.repeated.clear();
        self.bulk.clear();
        self.shape.clear();
        for (index, span) in spans.iter().enumerate() {
            self.shape.push((span.addr, span.len, span.ok, span.bulk));
            self.mark_exact_length(span.len);
            if self.slots.is_empty() {
                self.grow_slots(spans)?;
            }
            let mut slot = self
                .slot(spans, span.addr, span.len)
                .expect("raw index must have an empty bucket");
            let previous = self.slots[slot].get();
            if previous == usize::MAX {
                // At most half full. Repeated observations must not trigger
                // growth, even when their count dwarfs the distinct key count.
                if self.distinct == self.slots.len() / 2 {
                    self.grow_slots(spans)?;
                    slot = self
                        .slot(spans, span.addr, span.len)
                        .expect("grown raw index must have an empty bucket");
                }
                self.slots[slot].set(index);
                self.distinct += 1;
            } else {
                // During construction each cursor holds the last observation.
                // Record the root once, but not its as-yet-unstable bucket.
                if self.next[previous] == usize::MAX {
                    self.repeated.push((usize::MAX, previous));
                }
                self.next[previous] = index;
                self.next[index] = index;
                self.slots[slot].set(index);
            }
            // Only flagged bulk chunks serve sub-ranges: small exact
            // spans inside a chunk must never shadow it.
            if span.ok && self.has_bulk && span.bulk && span.len > 0 {
                self.bulk
                    .push((span.addr, span.addr + span.len as usize, index, 0));
            }
        }
        // The same cell is the authoritative read cursor after preparation.
        // Unique entries already name their sole span and need no reset.
        for root in 0..self.repeated.len() {
            let first = self.repeated[root].1;
            let span = &spans[first];
            let slot = self
                .slot(spans, span.addr, span.len)
                .expect("raw index must retain every repeated key");
            self.repeated[root].0 = slot;
            self.slots[slot].set(first);
        }
        if !self.bulk.is_empty() {
            self.bulk.sort_unstable();
            let mut maximum_end = 0;
            for span in &mut self.bulk {
                maximum_end = maximum_end.max(span.1);
                span.3 = maximum_end;
            }
        }
        self.update_capacity_charge();
        Ok(false)
    }
}

impl<'a> Recorded<'a> {
    pub fn build<const N: usize>(log: &'a RawLog<N>) -> Self {
        Self::build_range(log, 0..log.spans.len()).unwrap_or_else(|_| Self {
            index: RecordedIndex::default(),
            recycle: &log.index,
            spans: &[],
            bytes: &[],
            next_exact: std::cell::Cell::new(0),
            last_bulk: std::cell::Cell::new(usize::MAX),
        })
    }

    /// Borrow only this acquisition phase's observations. Masks are fault
    /// attribution, not lookup scope; another phase cannot heal a failure.
    pub fn build_range<const N: usize>(
        log: &'a RawLog<N>,
        range: std::ops::Range<usize>,
    ) -> Result<Self, &'static str> {
        let source_spans = log.spans.get(range).ok_or("recorded span range invalid")?;
        let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
        let started = std::time::Instant::now();
        let mut index = log
            .index
            .borrow_mut()
            .take()
            .unwrap_or_else(|| RecordedIndex {
                capacity_charge: crate::raw_memory::Charge::empty_on(log.capacity_charge.tracker()),
                ..Default::default()
            });
        // Range scoping narrows eligibility; it cannot enable a capability
        // the owning log withdrew, including when reusing a pooled index.
        let has_bulk = log.has_bulk && source_spans.iter().any(|span| span.bulk);
        let reused = match index.prepare(source_spans, has_bulk) {
            Ok(reused) => reused,
            Err(_) => {
                // Owned replay reads fail closed; no partially built cursor
                // can certify a complete frame after allocation failure.
                return Err("recorded span index allocation failed");
            }
        };
        let ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let (count, ticks, spans) = if reused {
            (&INDEX_REUSES, &INDEX_REUSE_NS, &INDEX_REUSED_SPANS)
        } else {
            (&INDEX_REBUILDS, &INDEX_REBUILD_NS, &INDEX_REBUILT_SPANS)
        };
        ticks.fetch_add(ns, Ordering::Relaxed);
        spans.fetch_add(source_spans.len() as u64, Ordering::Relaxed);
        count.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            index,
            recycle: &log.index,
            spans: source_spans,
            bytes: &log.bytes,
            next_exact: std::cell::Cell::new(0),
            last_bulk: std::cell::Cell::new(usize::MAX),
        })
    }

    /// Inspect one unique exact observation and its immediate predecessor in
    /// this reader's admitted span range, without consuming either cursor.
    /// The OOB child source uses its unique motorization observation to select
    /// that root batch's flag, rather than a repeated parent-reference flag.
    pub(crate) fn unique_observation(
        &self,
        address: usize,
        len: usize,
    ) -> Option<(&Span, Option<&Span>)> {
        let len = u32::try_from(len).ok()?;
        let index = self.index.cursor(self.spans, address, len)?.get();
        if self.index.next.get(index) != Some(&usize::MAX) {
            return None;
        }
        Some((
            self.spans.get(index)?,
            index
                .checked_sub(1)
                .and_then(|previous| self.spans.get(previous)),
        ))
    }

    #[inline(always)]
    fn serve(&self, address: usize, destination: &mut [u8]) -> bool {
        let hint = self.next_exact.get();
        let len = destination.len() as u32;
        let exact = if self.spans.get(hint).is_some_and(|span| {
            span.addr == address && span.len == len && self.index.next[hint] == usize::MAX
        }) {
            // A globally unique exact key has no observation cursor to
            // advance. Repeated keys always use the authoritative hash cursor,
            // even when a later occurrence happens to match this hint.
            Some(hint)
        } else if self.index.has_bulk && !self.index.may_have_exact_length(len) {
            // No exact observation of this width can override a bulk range.
            // A negative lookup must not advance either exact-reader cursor.
            None
        } else {
            self.index.cursor(self.spans, address, len).map(|cursor| {
                let current = cursor.get();
                let next = self.index.next[current];
                if next != usize::MAX {
                    cursor.set(next);
                }
                current
            })
        };
        if let Some(index) = exact {
            // Resynchronize on ordinary exact lookups too. An exact failure
            // retains its priority over bulk and still advances the hint.
            self.next_exact.set(index + 1);
            let span = &self.spans[index];
            if !span.ok {
                return false;
            }
            let start = span.start as usize;
            let Some(source) = self.bytes.get(start..start + destination.len()) else {
                return false;
            };
            destination.copy_from_slice(source);
            return true;
        }
        if self.index.bulk.is_empty() {
            return false;
        }
        let end = address.checked_add(destination.len());
        let end = match end {
            Some(end) => end,
            None => return false,
        };
        let cached = self.last_bulk.get();
        let selected = if self.index.bulk.get(cached).is_some_and(|span| {
            span.0 <= address
                && end <= span.1
                && self
                    .index
                    .bulk
                    .get(cached + 1)
                    .is_none_or(|next| next.0 > address)
        }) {
            // Reuse only while this is still the nearest starting span. A
            // nested/overlapping span beginning at or before the new request
            // forces the full search, even if this cached span encloses it.
            cached
        } else {
            // Prefer the nearest enclosing span. A smaller nested array must
            // not hide later parent fields or requests extending beyond it.
            let index = self.index.bulk.partition_point(|span| span.0 <= address);
            let enclosing = self.index.bulk[..index]
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, span)| span.3 >= end)
                .find(|(_, span)| span.1 >= end);
            let Some((selected, _)) = enclosing else {
                return false;
            };
            self.last_bulk.set(selected);
            selected
        };
        let (start, _, source_index, _) = self.index.bulk[selected];
        let offset = self.spans[source_index].start as usize + (address - start);
        let Some(source) = self.bytes.get(offset..offset + destination.len()) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

impl Drop for Recorded<'_> {
    fn drop(&mut self) {
        // Simultaneous readers get independent cursors. Only one cached index
        // is retained per payload; any additional index is dropped off-thread.
        let mut recycled = self.recycle.borrow_mut();
        if recycled.is_none() {
            *recycled = Some(std::mem::take(&mut self.index));
        }
    }
}

impl Memory for Recorded<'_> {
    fn immutable_view(
        &self,
        address: usize,
        len: usize,
    ) -> Option<engine::memory::SliceMemory<'_>> {
        if !self.index.has_bulk || address == 0 || len == 0 || !is_canonical_range(address, len) {
            return None;
        }
        let end = address.checked_add(len)?;
        let mut enclosing = None;
        for span in self.spans {
            if span.len == 0 {
                continue;
            }
            let span_end = span.addr.checked_add(span.len as usize)?;
            if span_end <= address || end <= span.addr {
                continue;
            }
            // Exactly one observation may overlap the requested view. This
            // rejects ordinary exact reads (including failures), duplicates,
            // and nested/overlapping/tied bulk winners without cursor access.
            if enclosing.is_some()
                || !span.ok
                || !span.bulk
                || span.addr == 0
                || !is_canonical_range(span.addr, span.len as usize)
                || span.addr > address
                || span_end < end
                // A whole bulk is itself an exact key. Keep that observation
                // on the ordinary path even when its current key is unique.
                || (span.addr == address && span_end == end)
            {
                return None;
            }
            enclosing = Some(span);
        }
        let span = enclosing?;
        let start = span.start as usize;
        let source = self
            .bytes
            .get(start..start.checked_add(span.len as usize)?)?;
        let offset = address - span.addr;
        Some(engine::memory::SliceMemory {
            base: address,
            bytes: source.get(offset..offset.checked_add(len)?)?,
        })
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        if address == 0 || address.checked_add(destination.len()).is_none() {
            return false;
        }
        // Mirror the live canonical gate so the replay fails the same way for
        // a noncanonical derived pointer even if a log ever contained it.
        // `take_inner` never records such spans; `take_records` may log one
        // as failed, but `copy_records` stops before reaching this read.
        if !is_canonical_range(address, destination.len()) {
            return false;
        }
        if destination.len() > u32::MAX as usize {
            return false;
        }
        self.serve(address, destination)
    }
}
