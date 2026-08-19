use std::cell::Cell;

use super::{MemoryRefusal, SnapshotMemory, Span};

#[derive(Clone, Debug)]
struct Region {
    base: u64,
    bytes: Vec<u8>,
    readable: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SyntheticMemory {
    regions: Vec<Region>,
    index: std::cell::RefCell<Option<Vec<u32>>>,
    copies: Cell<u64>,
    bytes_moved: Cell<u64>,
}

impl SyntheticMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(&mut self, base: u64, bytes: Vec<u8>) -> &mut Self {
        self.regions.push(Region {
            base,
            bytes,
            readable: true,
        });
        self.invalidate_index();
        self
    }

    pub fn map_zeroed(&mut self, base: u64, len: usize) -> &mut Self {
        self.map(base, vec![0u8; len])
    }

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
                None
            } else {
                let at =
                    order.partition_point(|slot| self.regions[*slot as usize].base <= span.start);
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

#[derive(Debug)]
pub struct MutatingMemory {
    inner: std::cell::RefCell<SyntheticMemory>,
    copies: Cell<u64>,
    trigger: u64,
    edits: Vec<(u64, Vec<u8>)>,
    applied: Cell<bool>,
}

impl MutatingMemory {
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
