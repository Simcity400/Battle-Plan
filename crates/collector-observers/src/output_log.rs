use crate::opinion_changes::Padded;
use collector_primitives::spill_stack::SpillStack;
use std::sync::{
    Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

#[cfg(any(test, feature = "test-support"))]
pub const CAPACITY: usize = 1 << 16;

pub const STORAGE_FAILED: &str = "output log entry storage allocation failed";

pub struct Log {
    pub capacity: AtomicUsize,
    pub cursor: Padded<AtomicU64>,
    pub epoch: AtomicU64,
    pub enabled: AtomicBool,
    pub active: Padded<AtomicUsize>,
    pub consumed: AtomicU64,
    pub required: AtomicUsize,
    lost: AtomicU64,
}

pub struct Spill<T> {
    pushed: SpillStack<(u64, T)>,
    held: Mutex<Vec<(u64, T)>>,
}

impl<T> Default for Spill<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Spill<T> {
    pub const fn new() -> Self {
        Self {
            pushed: SpillStack::new(),
            held: Mutex::new(Vec::new()),
        }
    }
    fn held(&self, from: u64) -> (MutexGuard<'_, Vec<(u64, T)>>, Option<u64>) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let refused = self.pushed.take_into(&mut held).err().and_then(|taken| {
            taken
                .map(|(index, _)| index)
                .filter(|&index| index >= from)
                .min()
        });
        (held, refused)
    }
    fn push(&self, index: u64, entry: T) -> bool {
        self.pushed.push((index, entry)).is_ok()
    }
    fn discard_before(&self, index: u64) -> Option<u64> {
        let (mut held, refused) = self.held(index);
        held.retain(|&(at, _)| at >= index);
        refused
    }
    #[doc = " After every writer and drain has left: frees the storage."]
    pub fn release(&self) {
        drop(self.pushed.take());
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Vec::new();
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.held(0).0.len()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn clear(&self) {
        self.held(0).0.clear();
    }
}

#[inline(always)]
pub fn raise(value: &AtomicUsize, minimum: usize, order: Ordering) {
    if value.load(Ordering::Relaxed) < minimum {
        value.fetch_max(minimum, order);
    }
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
    pub first: u64,
    pub cursor: u64,
    pub copied: usize,
    pub pending: usize,
}
#[cfg(any(test, feature = "test-support"))]
impl Drained {
    pub fn complete(&self) -> bool {
        self.first + self.copied as u64 >= self.cursor
    }
}

pub struct DrainCounters<'a> {
    pub drains: &'a AtomicU64,
    pub copied: &'a AtomicU64,
    pub overflow: &'a AtomicU64,
    pub unobserved: &'a AtomicU64,
}

pub enum Reserved {
    Slot(usize),
    Spill,
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
            lost: AtomicU64::new(u64::MAX),
        }
    }
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn enter(&self) -> Call<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        Call(self)
    }
    pub fn reserve(&self) -> (u64, Reserved) {
        let consumed = self.consumed.load(Ordering::Acquire);
        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        let waiting = index.wrapping_add(1).wrapping_sub(consumed);
        self.required.fetch_max(waiting as usize, Ordering::Relaxed);
        let capacity = self.capacity.load(Ordering::Acquire) as u64;
        if waiting <= capacity {
            (index, Reserved::Slot((index % capacity) as usize))
        } else {
            (index, Reserved::Spill)
        }
    }
    pub fn finish(&self, index: u64) {
        if self.consumed.load(Ordering::Acquire) > index
            && self.cursor.load(Ordering::SeqCst).wrapping_sub(index)
                > self.capacity.load(Ordering::Acquire) as u64
        {
            self.invalidate();
        }
    }
    pub fn spill<T>(&self, spill: &Spill<T>, index: u64, entry: T) {
        if !spill.push(index, entry) {
            self.lost.fetch_min(index, Ordering::AcqRel);
            self.invalidate();
        }
    }
    fn skip<T>(&self, spill: &Spill<T>, cursor: u64) {
        self.consumed.store(cursor, Ordering::Release);
        if let Some(index) = spill.discard_before(cursor) {
            self.lost.fetch_min(index, Ordering::AcqRel);
        }
    }
    pub fn drain<T: Copy + Default>(
        &self,
        output: &mut Vec<T>,
        spill: &Spill<T>,
        entry: impl Fn(usize) -> Option<T>,
        unavailable: impl FnOnce() -> Option<&'static str>,
        restarted: &'static str,
        counters: DrainCounters<'_>,
    ) -> Result<Drained, &'static str> {
        self.drain_buffer(
            engine_native_memory::walk::Buffer::Growing(output),
            spill,
            entry,
            unavailable,
            restarted,
            counters,
        )
    }
    pub fn drain_buffer<T: Copy + Default>(
        &self,
        mut output: engine_native_memory::walk::Buffer<'_, T>,
        spill: &Spill<T>,
        entry: impl Fn(usize) -> Option<T>,
        unavailable: impl FnOnce() -> Option<&'static str>,
        restarted: &'static str,
        counters: DrainCounters<'_>,
    ) -> Result<Drained, &'static str> {
        counters.drains.fetch_add(1, Ordering::Relaxed);
        let first = self.consumed.load(Ordering::Relaxed);
        let cursor = self.cursor.load(Ordering::SeqCst);
        let epoch = self.epoch.load(Ordering::SeqCst);
        if let Some(reason) = unavailable() {
            counters.unobserved.fetch_add(1, Ordering::Relaxed);
            self.skip(spill, cursor);
            return Err(reason);
        }
        let pending = cursor.wrapping_sub(first) as usize;
        self.required.fetch_max(pending, Ordering::Relaxed);
        let lost = || {
            let _ = self
                .lost
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |lost| {
                    (lost < cursor).then_some(u64::MAX)
                });
            counters.overflow.fetch_add(1, Ordering::Relaxed);
            self.skip(spill, cursor);
            Err(STORAGE_FAILED)
        };
        if self.lost.load(Ordering::Acquire) < cursor {
            return lost();
        }
        if self.active.load(Ordering::SeqCst) != 0 {
            return Ok(Drained {
                epoch,
                first,
                cursor,
                copied: 0,
                pending,
            });
        }
        output
            .ensure(pending)
            .map_err(|_| "output log drain allocation failed")?;
        let copied = pending.min(output.len());
        let capacity = self.capacity.load(Ordering::Acquire) as u64;
        let mut missing = false;
        {
            let (mut held, refused) = spill.held(first);
            if let Some(index) = refused {
                self.lost.fetch_min(index, Ordering::AcqRel);
                if index < cursor {
                    drop(held);
                    return lost();
                }
            }
            held.sort_unstable_by_key(|&(index, _)| index);
            let stale = held.partition_point(|&(index, _)| index < first);
            held.drain(..stale);
            let mut taken = 0;
            for (offset, out) in output[..copied].iter_mut().enumerate() {
                let index = first + offset as u64;
                *out = match held.get(taken) {
                    Some(&(at, value)) if at == index => {
                        taken += 1;
                        value
                    }
                    _ => index
                        .checked_rem(capacity)
                        .and_then(|slot| entry(slot as usize))
                        .unwrap_or_else(|| {
                            missing = true;
                            T::default()
                        }),
                };
            }
            held.drain(..taken);
        }
        if missing {
            return lost();
        }
        if self.epoch.load(Ordering::SeqCst) != epoch {
            counters.unobserved.fetch_add(1, Ordering::Relaxed);
            self.skip(spill, cursor);
            return Err(restarted);
        }
        self.consumed
            .store(first + copied as u64, Ordering::Release);
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

    struct Ring {
        log: Log,
        slots: Vec<AtomicU64>,
        spill: Spill<u64>,
    }
    impl Ring {
        fn new(capacity: usize) -> Self {
            Self {
                log: Log::new(capacity),
                slots: (0..capacity).map(|_| AtomicU64::new(u64::MAX)).collect(),
                spill: Spill::new(),
            }
        }
        fn append(&self) -> u64 {
            let _writer = self.log.enter();
            let (index, reserved) = self.log.reserve();
            match reserved {
                Reserved::Slot(slot) => {
                    self.slots[slot].store(index, Ordering::SeqCst);
                    self.log.finish(index);
                }
                Reserved::Spill => self.log.spill(&self.spill, index, index),
            }
            index
        }
        fn drain(&self, output: &mut Vec<u64>, counters: &Counters) -> Result<Drained, &'static str> {
            self.log.drain(
                output,
                &self.spill,
                |slot| Some(self.slots[slot].load(Ordering::SeqCst)),
                || None,
                "restarted",
                counters.borrowed(),
            )
        }
    }

    #[test]
    fn worker_drain_sizes_empty_and_recycled_output_from_the_original_cursor() {
        let ring = Ring::new(10003);
        let counters = Counters::default();
        let mut frame = Vec::new();
        for count in [5000, 10003] {
            let first = ring.log.cursor.load(Ordering::SeqCst);
            for _ in 0..count {
                ring.append();
            }
            let drained = ring.drain(&mut frame, &counters).unwrap();
            assert_eq!(drained.first, first);
            assert_eq!(drained.pending, count);
            assert_eq!(drained.copied, count);
            assert_eq!(frame[..count], (first..first + count as u64).collect::<Vec<_>>());
        }
        assert_eq!(counters.overflow.load(Ordering::Relaxed), 0);
        assert!(ring.spill.is_empty());
    }

    #[test]
    fn a_burst_over_the_ring_or_with_no_ring_hands_over_every_entry_in_order() {
        for capacity in [0, 2, 8] {
            let ring = Ring::new(capacity);
            let counters = Counters::default();
            let mut frame = Vec::new();
            for burst in [10003_u64, 5, 70_001] {
                let first = ring.log.cursor.load(Ordering::SeqCst);
                for _ in 0..burst {
                    ring.append();
                }
                let drained = ring.drain(&mut frame, &counters).unwrap();
                assert_eq!((drained.first, drained.cursor), (first, first + burst));
                assert_eq!(drained.copied as u64, burst);
                assert!(drained.complete());
                assert_eq!(frame[..burst as usize], (first..first + burst).collect::<Vec<_>>());
                assert!(ring.spill.is_empty());
            }
            assert_eq!(ring.log.required.load(Ordering::Relaxed), 70_001);
            assert_eq!(counters.overflow.load(Ordering::Relaxed), 0);
            assert_eq!(ring.log.epoch.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn a_spill_never_waits_for_the_drain_and_a_burst_keeps_every_entry_in_order() {
        let ring = Ring::new(4);
        let counters = Counters::default();
        let held = ring.spill.held.lock().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let ring = &ring;
            scope.spawn(move || {
                for _ in 0..1_000 {
                    ring.append();
                }
                done.send(()).unwrap();
            });
            finished
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("an output log spill waited for the drain's lock");
            drop(held);
        });
        let mut frame = Vec::new();
        let drained = ring.drain(&mut frame, &counters).unwrap();
        assert_eq!((drained.first, drained.cursor, drained.copied), (0, 1_000, 1_000));
        assert_eq!(frame[..1_000], (0..1_000).collect::<Vec<_>>());
        assert_eq!(counters.overflow.load(Ordering::Relaxed), 0);
        assert!(ring.spill.is_empty());
    }

    #[test]
    fn an_entry_no_storage_holds_gives_up_its_interval_by_name() {
        let ring = Ring::new(1);
        let counters = Counters::default();
        ring.append();
        ring.append();
        ring.log.lost.store(1, Ordering::SeqCst);
        let mut frame = Vec::new();
        assert_eq!(ring.drain(&mut frame, &counters), Err(STORAGE_FAILED));
        assert_eq!(counters.overflow.load(Ordering::Relaxed), 1);
        assert!(ring.spill.is_empty());
        ring.append();
        let next = ring.drain(&mut frame, &counters).unwrap();
        assert_eq!((next.first, next.copied), (2, 1));
        assert_eq!(frame, [2]);
    }
}
