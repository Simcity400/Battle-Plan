//! Shared bounded atomic-log protocol for production and construction output.
use crate::opinion_changes::Padded;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[cfg(test)]
pub const CAPACITY: usize = 1 << 16;

pub struct Log {
    /// Payload slots, changed only at an empty, writer-free drain boundary.
    pub capacity: AtomicUsize,
    pub cursor: Padded<AtomicU64>,
    pub epoch: AtomicU64,
    pub enabled: AtomicBool,
    pub active: Padded<AtomicUsize>,
    pub consumed: AtomicU64,
    /// Full attempted burst since consumption, including overwritten slots.
    pub required: AtomicUsize,
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

impl Log {
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity: AtomicUsize::new(capacity),
            cursor: Padded(AtomicU64::new(0)),
            epoch: AtomicU64::new(1),
            enabled: AtomicBool::new(false),
            active: Padded(AtomicUsize::new(0)),
            consumed: AtomicU64::new(0),
            required: AtomicUsize::new(0),
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
    #[cfg(test)]
    pub fn reserve(&self) -> (u64, usize) {
        let (index, slot) = self.reserve_optional();
        (index, slot.expect("published output log has no storage"))
    }
    pub fn reserve_optional(&self) -> (u64, Option<usize>) {
        let consumed = self.consumed.load(Ordering::Acquire);
        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        self.required.fetch_max(
            index.wrapping_add(1).wrapping_sub(consumed) as usize,
            Ordering::Relaxed,
        );
        let capacity = self.capacity.load(Ordering::Acquire) as u64;
        (index, (capacity != 0).then(|| (index % capacity) as usize))
    }
    pub fn finish(&self, index: u64) {
        // A stalled writer may have overwritten a newer entry in its slot.
        if self.cursor.load(Ordering::SeqCst).wrapping_sub(index)
            > self.capacity.load(Ordering::Acquire) as u64
        {
            self.invalidate();
        }
    }
    #[cfg(test)]
    pub fn drain<T: Default>(
        &self,
        output: &mut [T],
        entry: impl Fn(usize) -> T,
        unavailable: impl FnOnce() -> Option<&'static str>,
        restarted: &'static str,
        counters: DrainCounters<'_>,
    ) -> Result<Drained, &'static str> {
        self.drain_buffer(
            engine::walk::Buffer::Fixed(output),
            entry,
            unavailable,
            restarted,
            counters,
        )
    }
    pub fn drain_growing<T: Default>(
        &self,
        output: &mut Vec<T>,
        entry: impl Fn(usize) -> T,
        unavailable: impl FnOnce() -> Option<&'static str>,
        restarted: &'static str,
        counters: DrainCounters<'_>,
    ) -> Result<Drained, &'static str> {
        self.drain_buffer(
            engine::walk::Buffer::Growing(output),
            entry,
            unavailable,
            restarted,
            counters,
        )
    }
    pub(crate) fn drain_buffer<T: Default>(
        &self,
        mut output: engine::walk::Buffer<'_, T>,
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
        self.required.fetch_max(pending as usize, Ordering::Relaxed);
        if pending > capacity {
            counters.overflow.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(cursor, Ordering::Relaxed);
            return Ok(Drained {
                epoch,
                first: cursor,
                cursor,
                copied: 0,
                pending: pending as usize,
            });
        }
        let pending = pending as usize;
        if settled {
            output
                .ensure(pending)
                .map_err(|_| "output log drain allocation failed")?;
        }
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
            let required = now.wrapping_sub(first) as usize;
            self.required.fetch_max(required, Ordering::Relaxed);
            counters.overflow.fetch_add(1, Ordering::Relaxed);
            self.consumed.store(now, Ordering::Relaxed);
            return Ok(Drained {
                epoch,
                first: now,
                cursor,
                copied: 0,
                pending: required,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Counters {
        drains: AtomicU64,
        copied: AtomicU64,
        overflow: AtomicU64,
        unobserved: AtomicU64,
    }
    impl Counters {
        fn borrowed(&self) -> DrainCounters<'_> {
            DrainCounters {
                drains: &self.drains,
                copied: &self.copied,
                overflow: &self.overflow,
                unobserved: &self.unobserved,
            }
        }
    }

    #[test]
    fn worker_drain_sizes_empty_and_recycled_output_from_the_original_cursor() {
        let log = Log::new(10003);
        let counters = Counters::default();
        let mut frame = Vec::new();
        for count in [5000, 10003] {
            let first = log.cursor.load(Ordering::SeqCst);
            for _ in 0..count {
                let (index, _) = log.reserve();
                log.finish(index);
            }
            let drained = log
                .drain_growing(
                    &mut frame,
                    |slot| slot as u64,
                    || None,
                    "restarted",
                    counters.borrowed(),
                )
                .unwrap();
            assert_eq!(drained.first, first);
            assert_eq!(drained.pending, count);
            assert_eq!(drained.copied, count);
            for (offset, row) in frame[..count].iter().enumerate() {
                assert_eq!(*row, (first + offset as u64) % 10003);
            }
        }
        assert_eq!(counters.overflow.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fixed_short_output_preserves_pending_rows() {
        let log = Log::new(3);
        let counters = Counters::default();
        for _ in 0..3 {
            log.reserve();
        }
        let mut output = [0u64; 1];
        let drained = log
            .drain(
                &mut output,
                |slot| slot as u64,
                || None,
                "restarted",
                counters.borrowed(),
            )
            .unwrap();
        assert_eq!((drained.pending, drained.copied), (3, 1));
        assert!(!drained.complete());
        let mut growing = Vec::new();
        let drained = log
            .drain_growing(
                &mut growing,
                |slot| slot as u64,
                || None,
                "restarted",
                counters.borrowed(),
            )
            .unwrap();
        assert_eq!((drained.pending, drained.copied), (2, 2));
        assert_eq!(growing, [1, 2]);
    }

    #[test]
    fn every_overflowed_or_unallocated_slot_retains_full_burst_demand() {
        for capacity in [0, 2] {
            let log = Log::new(capacity);
            let counters = Counters::default();
            for _ in 0..10003 {
                let (_, slot) = log.reserve_optional();
                assert_eq!(slot.is_none(), capacity == 0);
            }
            let drained = log
                .drain_growing(
                    &mut Vec::<u64>::new(),
                    |_| panic!("lost payload must not be read"),
                    || None,
                    "restarted",
                    counters.borrowed(),
                )
                .unwrap();
            assert_eq!(drained.pending, 10003);
            assert_eq!(drained.copied, 0);
            assert_eq!(log.required.load(Ordering::Relaxed), 10003);
            assert_eq!(counters.overflow.load(Ordering::Relaxed), 1);
        }
    }
}
