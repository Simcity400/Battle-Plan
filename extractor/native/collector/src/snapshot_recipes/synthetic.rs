//! Synthetic memory: a [`SnapshotMemory`] backed by fixture bytes.
//!
//! The substrate reads through the [`SnapshotMemory`] abstraction for the same
//! reason the lanes read through their `LaneMemory` traits — so a test can lay
//! out an object graph, hand it to the real engine, and get a real answer
//! without a game running. Everything this module offers is the fixture side;
//! it has no counterpart in a shipped capture.
//!
//! Behaviour that matters for the tests that use it:
//!
//!  * Reads are all-or-nothing. A copy that straddles the end of a region is
//!    refused whole, exactly like a guarded read against an unmapped page.
//!  * Every copy is counted, so a test can assert that deduplication actually
//!    removed a transaction rather than merely producing the same bytes twice.
//!  * A region can be marked unreadable to exercise the memory-refusal path,
//!    and its contents can be mutated between the opening and closing waves to
//!    exercise witness invalidation.

use std::cell::Cell;

use super::{MemoryRefusal, SnapshotMemory, Span};

#[derive(Clone, Debug)]
struct Region {
    base: u64,
    bytes: Vec<u8>,
    readable: bool,
}

/// A fake address space.
///
/// `Clone` so a test can run the SAME staged world through two observations —
/// a quiet one and a mutated one — and compare them. A rebuilt world is not the
/// same world: the two would differ in whatever the builder does not pin.
#[derive(Clone, Debug, Default)]
pub struct SyntheticMemory {
    regions: Vec<Region>,
    /// Region indices sorted by base address, built on the first read and
    /// dropped whenever the regions change. See [`SyntheticMemory::region_for`]
    /// — this changes how long a lookup takes and never which region it finds.
    index: std::cell::RefCell<Option<Vec<u32>>>,
    copies: Cell<u64>,
    bytes_moved: Cell<u64>,
}

impl SyntheticMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Map `bytes` at `base`.
    pub fn map(&mut self, base: u64, bytes: Vec<u8>) -> &mut Self {
        self.regions.push(Region {
            base,
            bytes,
            readable: true,
        });
        self.invalidate_index();
        self
    }

    /// Map `len` zeroed bytes at `base`.
    pub fn map_zeroed(&mut self, base: u64, len: usize) -> &mut Self {
        self.map(base, vec![0u8; len])
    }

    /// Make an already-mapped region refuse every read.
    pub fn set_readable(&mut self, base: u64, readable: bool) -> &mut Self {
        if let Some(region) = self.regions.iter_mut().find(|r| r.base == base) {
            region.readable = readable;
        }
        self
    }

    pub fn write(&mut self, addr: u64, bytes: &[u8]) -> &mut Self {
        let region = self
            .regions
            .iter_mut()
            .find(|r| addr >= r.base && (addr - r.base) as usize + bytes.len() <= r.bytes.len())
            .expect("synthetic write outside every mapped region");
        let offset = (addr - region.base) as usize;
        region.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        self
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> &mut Self {
        self.write(addr, &value.to_le_bytes())
    }

    pub fn write_u64(&mut self, addr: u64, value: u64) -> &mut Self {
        self.write(addr, &value.to_le_bytes())
    }

    pub fn copies(&self) -> u64 {
        self.copies.get()
    }

    pub fn bytes_moved(&self) -> u64 {
        self.bytes_moved.get()
    }

    pub fn reset_counters(&self) {
        self.copies.set(0);
        self.bytes_moved.set(0);
    }

    /// The FIRST region, in mapping order, that wholly contains `span`.
    ///
    /// # Why this is not just `iter().find(..)` any more
    ///
    /// It was, and at the sizes every gate in this repo stages — a few dozen
    /// regions — that was right. Lane GL stages the d81 world at production's
    /// 3,512 province slots, which is 17,574 regions, and the plan issues
    /// 84,226 copies against it: the scan alone is three quarters of a billion
    /// comparisons and it swamped the substrate it was there to measure. That
    /// is a property of the FIXTURE and not of anything a capture runs, so the
    /// lookup is indexed and the fixture stops being the subject.
    ///
    /// THE ANSWER IS UNCHANGED, and the fallback is what makes that true rather
    /// than hoped. A binary search over bases can only be the mapping-order
    /// answer when at most one region can contain any address — so the index is
    /// built ONLY if the regions are pairwise disjoint, and a fixture that maps
    /// one region over another keeps the linear scan and its first-wins rule.
    fn region_for(&self, span: Span) -> Option<&Region> {
        // A ZERO-LENGTH SPAN IS THE ONE CASE TWO DISJOINT REGIONS CAN BOTH
        // CONTAIN (Codex 019fef9d, finding 4): at the boundary between
        // `[a, b)` and `[b, c)`, an empty span at `b` sits inside both by the
        // `start >= base && end <= end` test below, so "the first one mapped"
        // is a real choice and the binary search would make the other one. It
        // is rare enough not to be worth indexing and the scan is exact, so
        // an empty span takes the scan.
        if span.len == 0 {
            return self.regions.iter().find(|region| {
                let end = region.base as u128 + region.bytes.len() as u128;
                span.start >= region.base && span.end_excl() <= end
            });
        }
        let position = {
            let mut cached = self.index.borrow_mut();
            if cached.is_none() {
                *cached = Some(self.sorted_disjoint_index());
            }
            let order = cached.as_ref().expect("just built");
            if order.is_empty() && !self.regions.is_empty() {
                // Overlapping regions: the index cannot express first-wins.
                None
            } else {
                // The last region whose base is at or below the span's start;
                // no other can contain it, because they are disjoint.
                let at = order
                    .partition_point(|slot| self.regions[*slot as usize].base <= span.start);
                at.checked_sub(1).map(|at| order[at] as usize)
            }
        };
        let contains = |region: &Region| {
            let end = region.base as u128 + region.bytes.len() as u128;
            span.start >= region.base && span.end_excl() <= end
        };
        match position {
            Some(slot) => {
                let region = &self.regions[slot];
                if contains(region) {
                    Some(region)
                } else {
                    None
                }
            }
            None => self.regions.iter().find(|region| contains(region)),
        }
    }

    /// Region indices sorted by base, or EMPTY if any two regions overlap.
    ///
    /// An empty vector for a non-empty region list is the "cannot index this"
    /// signal; an empty region list indexes to an empty vector too, and both
    /// answer `None` for every span, so the two do not have to be told apart.
    fn sorted_disjoint_index(&self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.regions.len() as u32).collect();
        order.sort_unstable_by_key(|slot| self.regions[*slot as usize].base);
        for pair in order.windows(2) {
            let left = &self.regions[pair[0] as usize];
            let right = &self.regions[pair[1] as usize];
            if left.base as u128 + left.bytes.len() as u128 > right.base as u128 {
                return Vec::new();
            }
        }
        order
    }

    /// Drop the lookup index. Called by every `&mut self` entry point that can
    /// move a region, so a fixture staged in several steps cannot read through
    /// an index built before the last of them.
    fn invalidate_index(&mut self) {
        *self.index.borrow_mut() = None;
    }
}

impl SnapshotMemory for SyntheticMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        let span = Span::new(addr, out.len() as u64);
        let region = self.region_for(span).ok_or(MemoryRefusal { span })?;
        if !region.readable {
            return Err(MemoryRefusal { span });
        }
        let offset = (addr - region.base) as usize;
        out.copy_from_slice(&region.bytes[offset..offset + out.len()]);
        self.copies.set(self.copies.get() + 1);
        self.bytes_moved
            .set(self.bytes_moved.get() + out.len() as u64);
        Ok(())
    }
}

/// A [`SnapshotMemory`] that applies a pending mutation partway through the
/// observation. Interior mutability lives in the wrapper, not in
/// [`SyntheticMemory`], so the fixture stays a plain value.
#[derive(Debug)]
pub struct MutatingMemory {
    inner: std::cell::RefCell<SyntheticMemory>,
    copies: Cell<u64>,
    trigger: u64,
    edits: Vec<(u64, Vec<u8>)>,
    applied: Cell<bool>,
}

impl MutatingMemory {
    /// Apply `edits` immediately BEFORE the copy numbered `trigger`
    /// (1-indexed), so a test can place a mutation exactly between the opening
    /// and closing images.
    pub fn new(memory: SyntheticMemory, trigger: u64, edits: Vec<(u64, Vec<u8>)>) -> Self {
        Self {
            inner: std::cell::RefCell::new(memory),
            copies: Cell::new(0),
            trigger,
            edits,
            applied: Cell::new(false),
        }
    }

    pub fn copies(&self) -> u64 {
        self.copies.get()
    }

    pub fn mutation_applied(&self) -> bool {
        self.applied.get()
    }
}

impl SnapshotMemory for MutatingMemory {
    fn copy_into(&self, addr: u64, out: &mut [u8]) -> Result<(), MemoryRefusal> {
        let next = self.copies.get() + 1;
        if next >= self.trigger && !self.applied.get() {
            self.applied.set(true);
            let mut inner = self.inner.borrow_mut();
            for (at, bytes) in &self.edits {
                inner.write(*at, bytes);
            }
        }
        let result = self.inner.borrow().copy_into(addr, out);
        if result.is_ok() {
            self.copies.set(next);
        }
        result
    }
}
