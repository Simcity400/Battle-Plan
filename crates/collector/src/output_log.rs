//! Shared bounded atomic-log protocol for production and construction output.
use crate::opinion_changes::Padded;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub const CAPACITY: usize = 1 << 16;

pub struct Log {
    /// Payload slots, changed only at an empty, writer-free drain boundary.
    pub capacity: AtomicUsize,
    pub cursor: Padded<AtomicU64>,
    pub epoch: AtomicU64,
    pub enabled: AtomicBool,
    pub active: Padded<AtomicUsize>,
    pub consumed: AtomicU64,
}

pub struct Call<'a>(&'a Log);
impl Drop for Call<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drained {
    pub epoch: u64,
    /// First copied index; past lost entries after overflow.
    pub first: u64,
    /// Entries before this sample have indices below this cursor.
    pub cursor: u64,
    pub copied: usize,
    /// Storage needed to take all waiting entries.
    pub pending: usize,
}
#[cfg(test)]
impl Drained {
    pub(crate) fn complete(&self) -> bool {
        self.first + self.copied as u64 >= self.cursor
    }
}

pub struct DrainCounters<'a> {
    pub drains: &'a AtomicU64,
    pub copied: &'a AtomicU64,
    pub overflow: &'a AtomicU64,
    pub unobserved: &'a AtomicU64,
}

impl Default for Log {
    fn default() -> Self {
        Self::new(CAPACITY)
    }
}

impl Log {
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity: AtomicUsize::new(capacity),
            cursor: Padded(AtomicU64::new(0)),
            epoch: AtomicU64::new(1),
            enabled: AtomicBool::new(false),
            active: Padded(AtomicUsize::new(0)),
            consumed: AtomicU64::new(0),
        }
    }
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn enter(&self) -> Call<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        Call(self)
    }
    /// Callers keep an active guard through all atomic payload stores.
    pub fn reserve(&self) -> (u64, usize) {
        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        let capacity = self.capacity.load(Ordering::Acquire) as u64;
        (index, (index % capacity) as usize)
    }
    pub fn finish(&self, index: u64) {
        // A stalled writer may have overwritten a newer entry in its slot.
        if self.cursor.load(Ordering::SeqCst).wrapping_sub(index)
            > self.capacity.load(Ordering::Acquire) as u64
        {
            self.invalidate();
        }
    }
    pub fn drain<T>(
        &self,
        output: &mut [T],
        entry: impl Fn(usize) -> T,
        unavailable: impl FnOnce() -> Option<&'static str>,
        restarted: &'static str,
        counters: DrainCounters<'_>,
    ) -> Result<Drained, &'static str> {
        counters.drains.fetch_add(1, Ordering::Relaxed);
        let mut consumed = self.consumed.load(Ordering::Relaxed);
        // Read the cursor before ACTIVE: every earlier index is settled when
        // ACTIVE is zero. New indices cannot overwrite these slots unnoticed.
        let cursor = self.cursor.load(Ordering::SeqCst);
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(reason) = unavailable() {
            counters.unobserved.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(cursor, Ordering::Relaxed);
            return Err(reason);
        }
        let capacity = self.capacity.load(Ordering::Acquire) as u64;
        let settled = self.active.load(Ordering::SeqCst) == 0;
        let pending = cursor.wrapping_sub(consumed);
        if pending > capacity {
            counters.overflow.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(cursor, Ordering::Relaxed);
            return Ok(Drained {
                epoch,
                first: cursor,
                cursor,
                copied: 0,
                pending: 0,
            });
        }
        let pending = pending as usize;
        let first = consumed;
        let copied = if settled {
            pending.min(output.len())
        } else {
            0
        };
        for (offset, out) in output[..copied].iter_mut().enumerate() {
            *out = entry(((first + offset as u64) % capacity) as usize);
        }
        let now = self.cursor.load(Ordering::SeqCst);
        if now.wrapping_sub(first) > capacity {
            counters.overflow.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(now, Ordering::Relaxed);
            return Ok(Drained {
                epoch,
                first: now,
                cursor,
                copied: 0,
                pending: 0,
            });
        }
        if self.epoch.load(Ordering::SeqCst) != epoch {
            counters.unobserved.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(cursor, Ordering::Relaxed);
            return Err(restarted);
        }
        consumed += copied as u64;
        self.consumed.store(consumed, Ordering::Relaxed);
        counters.copied.fetch_add(copied as u64, Ordering::Relaxed);
        Ok(Drained {
            epoch,
            first,
            cursor,
            copied,
            pending,
        })
    }
}
