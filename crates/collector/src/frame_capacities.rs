//! The capacities a new frame is allocated with. The spool, the sizing thread and the frame
//! builder change them in turn under a lock; every change is published into one of two slots,
//! which an hour building its own frame reads without ever waiting on a holder of that lock.

use crate::sampling::FrameCapacities;
use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) struct Capacities {
    held: Mutex<FrameCapacities>,
    slots: [UnsafeCell<FrameCapacities>; 2],
    /// The slot last published.
    current: AtomicUsize,
    /// Readers copying out of each slot.
    readers: [AtomicUsize; 2],
}

// SAFETY: a slot is written only by the holder of `held`, only while it is not `current` and no
// reader is in it; readers copy only the current slot.
unsafe impl Sync for Capacities {}

impl Capacities {
    pub(crate) fn new(capacities: FrameCapacities) -> Self {
        Self {
            held: Mutex::new(capacities),
            slots: [UnsafeCell::new(capacities), UnsafeCell::new(capacities)],
            current: AtomicUsize::new(0),
            readers: [AtomicUsize::new(0), AtomicUsize::new(0)],
        }
    }

    /// The capacities to change, published when the guard drops. Collector threads only.
    pub(crate) fn lock(&self) -> Guard<'_> {
        Guard {
            cell: self,
            held: self.held.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    #[cfg(test)]
    pub(crate) fn try_lock(&self) -> Option<Guard<'_>> {
        use std::sync::TryLockError;
        let held = match self.held.try_lock() {
            Ok(held) => held,
            Err(TryLockError::Poisoned(held)) => held.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        Some(Guard { cell: self, held })
    }

    /// The capacities last published, never waiting on a holder: a holder publishes into the
    /// slot no reader is in, so a retry only follows a publication made meanwhile.
    pub(crate) fn published(&self) -> FrameCapacities {
        loop {
            let slot = self.current.load(Ordering::SeqCst);
            self.readers[slot].fetch_add(1, Ordering::SeqCst);
            if self.current.load(Ordering::SeqCst) == slot {
                // SAFETY: the current slot, pinned: no holder writes it until this reader leaves.
                let capacities = unsafe { *self.slots[slot].get() };
                self.readers[slot].fetch_sub(1, Ordering::Release);
                return capacities;
            }
            self.readers[slot].fetch_sub(1, Ordering::Release);
        }
    }

    fn publish(&self, capacities: &FrameCapacities) {
        let current = self.current.load(Ordering::SeqCst);
        // SAFETY: only the holder writes a slot, and never the current one.
        if unsafe { *self.slots[current].get() } == *capacities {
            return;
        }
        let next = current ^ 1;
        // A reader still copying what was current before the last publication.
        while self.readers[next].load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
        // SAFETY: not current and no reader in it; readers find it only once it is published.
        unsafe { *self.slots[next].get() = *capacities };
        self.current.store(next, Ordering::SeqCst);
    }
}

pub(crate) struct Guard<'a> {
    cell: &'a Capacities,
    held: MutexGuard<'a, FrameCapacities>,
}

impl Deref for Guard<'_> {
    type Target = FrameCapacities;

    fn deref(&self) -> &FrameCapacities {
        &self.held
    }
}

impl DerefMut for Guard<'_> {
    fn deref_mut(&mut self) -> &mut FrameCapacities {
        &mut self.held
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.cell.publish(&self.held);
    }
}
