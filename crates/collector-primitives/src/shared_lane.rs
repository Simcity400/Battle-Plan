//! Multi-producer overflow lane for per-thread observer logs.
//!
//! A log gives each registered native thread its own single-producer lane.
//! A thread registering after every lane is taken writes here instead of
//! invalidating the interval: it reserves the next index by compare-exchange
//! (a full ring refuses before taking one, so no hole is left), fills that
//! slot and its byte span, and publishes the slot with a release mark. The
//! hourly consumer merges published slots with the dedicated lanes by the
//! log-wide order each record carries, releases each slot after copying it,
//! and frees ring space only over a released prefix, so out-of-order
//! publication never lets a producer overwrite an unread slot.
use engine_allocation as allocation;
use std::{
    cell::UnsafeCell,
    io,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

// A slot mark holds its index + 1 once published, with these flags.
const RELEASED: u64 = 1 << 63;
const VOID: u64 = 1 << 62;
const FLAGS: u64 = RELEASED | VOID;

struct Slot<R> {
    record: UnsafeCell<R>,
    mark: AtomicU64,
}

pub struct SharedLane<R> {
    slots: Box<[Slot<R>]>,
    bytes: Box<[UnsafeCell<u8>]>,
    stride: usize,
    reserved: AtomicU64,
    tail: AtomicU64,
    rejected: AtomicU64,
    consumed_rejected: AtomicU64,
    required_entries: AtomicUsize,
}
// Each reserved index has one producer until it publishes; the consumer reads
// a slot only after its release mark and frees it only after copying it.
unsafe impl<R: Send> Sync for SharedLane<R> {}

impl<R: Copy + Default> SharedLane<R> {
    /// `entries` rounds up to a power of two; `stride` bytes follow each slot.
    pub fn new(entries: usize, stride: usize) -> io::Result<Self> {
        let entries = entries
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(allocation::oom)?;
        let mut slots = allocation::vec(entries)?;
        slots.resize_with(entries, || Slot {
            record: UnsafeCell::new(R::default()),
            mark: AtomicU64::new(0),
        });
        let bytes = allocation::mul(entries, stride)?;
        Ok(Self {
            slots: allocation::boxed_slice(slots)?,
            bytes: allocation::boxed_slice(allocation::default_vec(bytes)?)?,
            stride,
            reserved: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            consumed_rejected: AtomicU64::new(0),
            required_entries: AtomicUsize::new(0),
        })
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn stride(&self) -> usize {
        self.stride
    }
    /// No reservation can still own an element. The owner must separately
    /// exclude new producers before replacing the lane.
    pub fn is_drained(&self) -> bool {
        self.reserved.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.slots.len() * size_of::<Slot<R>>() + self.bytes.len()
    }
    /// Unreleased demand including refused records, for off-thread growth.
    pub fn required_entries(&self) -> usize {
        self.required_entries.load(Ordering::Relaxed)
    }
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Acquire)
    }
    /// Callback side: never waits. `None` when every slot holds an unreleased
    /// record; the refusal counts toward the demand the next lane must hold.
    pub fn reserve(&self) -> Option<Reservation<'_, R>> {
        let len = self.slots.len() as u64;
        let mut index = self.reserved.load(Ordering::Acquire);
        loop {
            let pending = index.wrapping_sub(self.tail.load(Ordering::Acquire));
            if pending >= len {
                let refused = self
                    .rejected
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1)
                    .wrapping_sub(self.consumed_rejected.load(Ordering::Acquire));
                let demand = pending.saturating_add(refused);
                self.required_entries.fetch_max(
                    usize::try_from(demand)
                        .unwrap_or(usize::MAX)
                        .max(self.slots.len().saturating_mul(2)),
                    Ordering::Relaxed,
                );
                return None;
            }
            match self.reserved.compare_exchange_weak(
                index,
                index + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(Reservation {
                        lane: self,
                        index,
                        published: false,
                    });
                }
                Err(current) => index = current,
            }
        }
    }
    fn slot(&self, index: u64) -> &Slot<R> {
        &self.slots[index as usize & (self.slots.len() - 1)]
    }
    fn byte_pointer(&self, index: u64) -> *mut u8 {
        let start = (index as usize & (self.slots.len() - 1)) * self.stride;
        unsafe { self.bytes.as_ptr().add(start).cast::<u8>().cast_mut() }
    }
    /// Consumer side: every published, unreleased record as (order, index),
    /// sorted by the log-wide order. Void reservations are released here.
    pub fn published(&self, order: impl Fn(&R) -> u64) -> Result<Vec<(u64, u64)>, &'static str> {
        let mut entries = Vec::new();
        self.published_into(&mut entries, order)?;
        Ok(entries)
    }
    /// Reuse the sole consumer's ordering arena across drains.
    pub fn published_into(
        &self,
        entries: &mut Vec<(u64, u64)>,
        order: impl Fn(&R) -> u64,
    ) -> Result<(), &'static str> {
        entries.clear();
        let tail = self.tail.load(Ordering::Acquire);
        let end = self.reserved.load(Ordering::Acquire);
        for index in tail..end {
            let slot = self.slot(index);
            let mark = slot.mark.load(Ordering::Acquire);
            if mark & !FLAGS != index + 1 || mark & RELEASED != 0 {
                continue;
            }
            if mark & VOID != 0 {
                slot.mark.fetch_or(RELEASED, Ordering::Release);
                continue;
            }
            allocation::reserve(entries, 1)
                .map_err(|_| "shared observer lane drain allocation failed")?;
            entries.push((order(unsafe { &*slot.record.get() }), index));
        }
        entries.sort_unstable();
        Ok(())
    }
    /// A published, unreleased record and its byte span.
    pub fn record(&self, index: u64) -> (R, &[u8]) {
        let record = unsafe { *self.slot(index).record.get() };
        let bytes = unsafe { std::slice::from_raw_parts(self.byte_pointer(index), self.stride) };
        (record, bytes)
    }
    /// After the record and its bytes are copied or withdrawn.
    pub fn release(&self, index: u64) {
        self.slot(index).mark.fetch_or(RELEASED, Ordering::Release);
    }
    /// Frees ring space over the released prefix. When nothing is left
    /// unreleased, refusals up to `rejected` belong to the drained interval.
    pub fn advance(&self, rejected: u64) {
        let end = self.reserved.load(Ordering::Acquire);
        let mut tail = self.tail.load(Ordering::Acquire);
        while tail < end {
            let mark = self.slot(tail).mark.load(Ordering::Acquire);
            if mark & !FLAGS != tail + 1 || mark & RELEASED == 0 {
                break;
            }
            tail += 1;
        }
        self.tail.store(tail, Ordering::Release);
        if tail == end {
            self.consumed_rejected.store(rejected, Ordering::Release);
        }
    }
}

/// One reserved slot. Dropped unpublished (an early refusal after the
/// reservation), it is published void so the ring never keeps a hole.
pub struct Reservation<'a, R> {
    lane: &'a SharedLane<R>,
    index: u64,
    published: bool,
}
impl<R> Reservation<'_, R> {
    /// This slot's byte span, written only by its producer.
    pub fn bytes(&mut self) -> *mut u8 {
        let start = (self.index as usize & (self.lane.slots.len() - 1)) * self.lane.stride;
        unsafe { self.lane.bytes.as_ptr().add(start).cast::<u8>().cast_mut() }
    }
    pub fn stride(&self) -> usize {
        self.lane.stride
    }
    pub fn publish(mut self, record: R) {
        let slot = &self.lane.slots[self.index as usize & (self.lane.slots.len() - 1)];
        unsafe { *slot.record.get() = record };
        slot.mark.store(self.index + 1, Ordering::Release);
        self.published = true;
    }
}
impl<R> Drop for Reservation<'_, R> {
    fn drop(&mut self) {
        if !self.published {
            let slot = &self.lane.slots[self.index as usize & (self.lane.slots.len() - 1)];
            slot.mark.store((self.index + 1) | VOID, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_publication_frees_only_a_released_prefix() {
        let lane = SharedLane::<u64>::new(2, 1).unwrap();
        let first = lane.reserve().unwrap();
        let second = lane.reserve().unwrap();
        // A full ring refuses before taking an index and records demand.
        assert!(lane.reserve().is_none());
        assert_eq!(lane.rejected(), 1);
        assert!(lane.required_entries() >= 3);
        second.publish(7);
        assert_eq!(lane.published(|&r| r).unwrap(), vec![(7, 1)]);
        lane.release(1);
        lane.advance(lane.rejected());
        // The earlier slot is still being written: no space is freed yet.
        assert!(lane.reserve().is_none());
        first.publish(9);
        assert_eq!(lane.published(|&r| r).unwrap(), vec![(9, 0)]);
        lane.release(0);
        lane.advance(lane.rejected());
        let third = lane.reserve().unwrap();
        drop(third);
        // An abandoned reservation is void, never a hole.
        assert!(lane.published(|&r| r).unwrap().is_empty());
        lane.advance(lane.rejected());
        assert!(lane.reserve().is_some());
    }
}
