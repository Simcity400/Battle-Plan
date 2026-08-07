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
#[derive(Debug, Default)]
pub struct SyntheticMemory {
    regions: Vec<Region>,
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

    fn region_for(&self, span: Span) -> Option<&Region> {
        self.regions.iter().find(|region| {
            let end = region.base as u128 + region.bytes.len() as u128;
            span.start >= region.base && span.end_excl() <= end
        })
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
