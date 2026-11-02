use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub const OWNER_COUNT: usize = 11;
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Owner {
    Other,
    Staging,
    WriterBlocks,
    Codec,
    Dictionary,
    ReaderBlocks,
    PayloadPages,
    GateDecoder,
    Metadata,
    CurrentValues,
    Reasons,
}
#[derive(Clone)]
pub struct Budget(Arc<Inner>, usize);
struct Inner {
    limit: u64,
    used: AtomicU64,
    peak: AtomicU64,
    owner_used: [AtomicU64; OWNER_COUNT],
    owner_peak: [AtomicU64; OWNER_COUNT],
    misses: [AtomicU64; OWNER_COUNT],
    spilled: [AtomicU64; OWNER_COUNT],
    reloads: [AtomicU64; OWNER_COUNT],
}
pub struct Reservation {
    budget: Budget,
    bytes: u64,
}

impl Budget {
    pub fn new(limit: u64) -> Self {
        Self(
            Arc::new(Inner {
                limit,
                used: AtomicU64::new(0),
                peak: AtomicU64::new(0),
                owner_used: std::array::from_fn(|_| AtomicU64::new(0)),
                owner_peak: std::array::from_fn(|_| AtomicU64::new(0)),
                misses: std::array::from_fn(|_| AtomicU64::new(0)),
                spilled: std::array::from_fn(|_| AtomicU64::new(0)),
                reloads: std::array::from_fn(|_| AtomicU64::new(0)),
            }),
            0,
        )
    }
    pub fn owner(&self, owner: Owner) -> Self {
        Self(self.0.clone(), owner as usize)
    }
    pub fn owner_peaks(&self) -> [u64; OWNER_COUNT] {
        std::array::from_fn(|i| self.0.owner_peak[i].load(Ordering::Relaxed))
    }
    pub fn paging(&self) -> [(u64, u64, u64); OWNER_COUNT] {
        std::array::from_fn(|i| {
            (
                self.0.misses[i].load(Ordering::Relaxed),
                self.0.reloads[i].load(Ordering::Relaxed),
                self.0.spilled[i].load(Ordering::Relaxed),
            )
        })
    }
    pub(crate) fn page_miss(&self, reload: bool) {
        self.0.misses[self.1].fetch_add(1, Ordering::Relaxed);
        if reload {
            self.0.reloads[self.1].fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn spill(&self, bytes: u64) {
        self.0.spilled[self.1].fetch_add(bytes, Ordering::Relaxed);
    }
    pub fn used(&self) -> u64 {
        self.0.used.load(Ordering::Relaxed)
    }
    pub fn peak(&self) -> u64 {
        self.0.peak.load(Ordering::Relaxed)
    }
    pub fn limit(&self) -> u64 {
        self.0.limit
    }
    fn charge(&self, bytes: u64) -> io::Result<()> {
        let old = self
            .0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| {
                io::Error::other("core-spool working-set or disk reservation exhausted")
            })?;
        self.0.peak.fetch_max(old + bytes, Ordering::Relaxed);
        let owner = self.0.owner_used[self.1].fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.0.owner_peak[self.1].fetch_max(owner, Ordering::Relaxed);
        Ok(())
    }
    pub fn reserve(&self, bytes: u64) -> io::Result<Reservation> {
        self.charge(bytes)?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
    // Disk charges survive handles: files still occupy space after a writer closes.
    pub fn grow_disk(&self, bytes: u64) -> io::Result<()> {
        self.charge(bytes)
    }
    pub(crate) fn release_disk(&self, bytes: u64) {
        self.0.owner_used[self.1].fetch_sub(bytes, Ordering::AcqRel);
        self.0.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.owner_used[self.budget.1].fetch_sub(self.bytes, Ordering::AcqRel);
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub(crate) struct Buffer {
    data: Vec<u8>,
    _held: Reservation,
}
impl Buffer {
    pub fn new(bytes: usize, budget: &Budget) -> io::Result<Self> {
        let held = budget.reserve(bytes as u64)?;
        let mut data = Vec::new();
        data.try_reserve_exact(bytes).map_err(io::Error::other)?;
        if data.capacity() != bytes {
            return Err(io::Error::other("allocator exceeded reserved capacity"));
        }
        Ok(Self { data, _held: held })
    }
    pub fn clear(&mut self) {
        self.data.clear();
    }
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > self.data.capacity() - self.data.len() {
            return Err(io::Error::other("block buffer reservation exhausted"));
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    pub fn resize(&mut self, size: usize) -> io::Result<()> {
        if size > self.data.capacity() {
            return Err(io::Error::other("block exceeds admitted working set"));
        }
        self.data.resize(size, 0);
        Ok(())
    }
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }
}
impl std::ops::Deref for Buffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.data
    }
}
impl std::ops::DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
