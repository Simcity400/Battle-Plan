use engine_allocation as allocation;
use std::{
    cell::UnsafeCell,
    io, ptr,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

const RELEASED: u64 = 1 << 63;
const VOID: u64 = 1 << 62;
const FLAGS: u64 = RELEASED | VOID;
const SPILLED: u64 = 1 << 63;

struct Slot<R> {
    record: UnsafeCell<R>,
    mark: AtomicU64,
}

struct Spill<R> {
    next: AtomicPtr<Spill<R>>,
    ticket: u64,
    sequence: u64,
    released: AtomicBool,
    record: UnsafeCell<R>,
    bytes: Box<[UnsafeCell<u8>]>,
}

struct Taken<R>(*mut Spill<R>);
unsafe impl<R: Send> Send for Taken<R> {}

pub struct SharedLane<R> {
    slots: Box<[Slot<R>]>,
    bytes: Box<[UnsafeCell<u8>]>,
    stride: usize,
    reserved: AtomicU64,
    tail: AtomicU64,
    rejected: AtomicU64,
    consumed_rejected: AtomicU64,
    required_entries: AtomicUsize,
    pushed: AtomicPtr<Spill<R>>,
    taken: Mutex<Taken<R>>,
    spilled: AtomicUsize,
    spilled_bytes: AtomicUsize,
    spill_sequence: AtomicU64,
}
unsafe impl<R: Send> Sync for SharedLane<R> {}

impl<R: Copy + Default> SharedLane<R> {
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
            pushed: AtomicPtr::new(ptr::null_mut()),
            taken: Mutex::new(Taken(ptr::null_mut())),
            spilled: AtomicUsize::new(0),
            spilled_bytes: AtomicUsize::new(0),
            spill_sequence: AtomicU64::new(0),
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
    pub fn is_drained(&self) -> bool {
        self.reserved.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
            && self.spilled.load(Ordering::Acquire) == 0
    }
    pub fn allocated_bytes(&self) -> usize {
        self.slots.len() * size_of::<Slot<R>>()
            + self.bytes.len()
            + self.spilled.load(Ordering::Relaxed) * size_of::<Spill<R>>()
            + self.spilled_bytes.load(Ordering::Relaxed)
    }
    pub fn required_entries(&self) -> usize {
        self.required_entries.load(Ordering::Relaxed)
    }
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Acquire)
    }
    pub fn pending(&self) -> usize {
        (self
            .reserved
            .load(Ordering::Acquire)
            .saturating_sub(self.tail.load(Ordering::Acquire)) as usize)
            .saturating_add(self.spilled.load(Ordering::Acquire))
    }
    pub fn reserve(&self) -> Option<Reservation<'_, R>> {
        self.reserve_bytes(0)
    }
    pub fn reserve_bytes(&self, len: usize) -> Option<Reservation<'_, R>> {
        if len > self.stride {
            return self.spill(len, 0, self.reserved.load(Ordering::Acquire));
        }
        match self.ring() {
            Ok(reservation) => Some(reservation),
            Err((pending, ticket)) => {
                let outstanding =
                    pending.saturating_add(self.spilled.load(Ordering::Acquire) as u64);
                self.require(outstanding.saturating_add(1));
                self.spill(self.stride, outstanding, ticket)
            }
        }
    }
    pub fn try_reserve(&self) -> Option<Reservation<'_, R>> {
        self.ring()
            .map_err(|(pending, _)| self.refuse(pending))
            .ok()
    }
    fn ring(&self) -> Result<Reservation<'_, R>, (u64, u64)> {
        let len = self.slots.len() as u64;
        let mut index = self.reserved.load(Ordering::Acquire);
        loop {
            let pending = index.wrapping_sub(self.tail.load(Ordering::Acquire));
            if pending >= len {
                return Err((pending, index));
            }
            match self.reserved.compare_exchange_weak(
                index,
                index + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Reservation {
                        lane: self,
                        at: At::Slot(index),
                        published: false,
                    });
                }
                Err(current) => index = current,
            }
        }
    }
    fn require(&self, demand: u64) {
        self.required_entries.fetch_max(
            usize::try_from(demand)
                .unwrap_or(usize::MAX)
                .max(self.slots.len().saturating_mul(2)),
            Ordering::Relaxed,
        );
    }
    fn refuse(&self, outstanding: u64) {
        let refused = self
            .rejected
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .wrapping_sub(self.consumed_rejected.load(Ordering::Acquire));
        self.require(outstanding.saturating_add(refused));
    }
    #[cold]
    fn spill(&self, len: usize, outstanding: u64, ticket: u64) -> Option<Reservation<'_, R>> {
        let spill = allocation::advisory(|| {
            let bytes = allocation::boxed_slice(allocation::default_vec(len)?)?;
            allocation::boxed(Spill {
                next: AtomicPtr::new(ptr::null_mut()),
                ticket,
                sequence: self.spill_sequence.fetch_add(1, Ordering::Relaxed),
                released: AtomicBool::new(false),
                record: UnsafeCell::new(R::default()),
                bytes,
            })
        });
        let Ok(spill) = spill else {
            self.refuse(outstanding);
            return None;
        };
        self.spilled.fetch_add(1, Ordering::AcqRel);
        self.spilled_bytes.fetch_add(len, Ordering::Relaxed);
        Some(Reservation {
            lane: self,
            at: At::Spill(Box::into_raw(spill)),
            published: false,
        })
    }
    fn slot(&self, index: u64) -> &Slot<R> {
        &self.slots[index as usize & (self.slots.len() - 1)]
    }
    fn byte_pointer(&self, index: u64) -> *mut u8 {
        let start = (index as usize & (self.slots.len() - 1)) * self.stride;
        unsafe { self.bytes.as_ptr().add(start).cast::<u8>().cast_mut() }
    }
    fn spilled_record(index: u64) -> *mut Spill<R> {
        ptr::with_exposed_provenance_mut((index & !SPILLED) as usize)
    }
    fn position(&self, index: u64) -> (u64, bool, u64) {
        if index & SPILLED != 0 {
            let spill = unsafe { &*Self::spilled_record(index) };
            (spill.ticket, false, spill.sequence)
        } else {
            (index, true, 0)
        }
    }
    pub fn published(&self, order: impl Fn(&R) -> u64) -> Result<Vec<(u64, u64)>, &'static str> {
        let mut entries = Vec::new();
        self.published_into(&mut entries, order)?;
        Ok(entries)
    }
    pub fn published_into(
        &self,
        entries: &mut Vec<(u64, u64)>,
        order: impl Fn(&R) -> u64,
    ) -> Result<(), &'static str> {
        const FAILED: &str = "shared observer lane drain allocation failed";
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
            allocation::reserve(entries, 1).map_err(|_| FAILED)?;
            entries.push((order(unsafe { &*slot.record.get() }), index));
        }
        if self.spilled.load(Ordering::Acquire) != 0 {
            let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
            let mut next = self.pushed.swap(ptr::null_mut(), Ordering::Acquire);
            while let Some(spill) = unsafe { next.as_ref() } {
                let pushed = next;
                next = spill.next.swap(taken.0, Ordering::Relaxed);
                taken.0 = pushed;
            }
            let mut next = taken.0;
            while let Some(spill) = unsafe { next.as_ref() } {
                if !spill.released.load(Ordering::Acquire) {
                    allocation::reserve(entries, 1).map_err(|_| FAILED)?;
                    entries.push((
                        order(unsafe { &*spill.record.get() }),
                        SPILLED | next.expose_provenance() as u64,
                    ));
                }
                next = spill.next.load(Ordering::Relaxed);
            }
            entries.sort_unstable_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| self.position(left.1).cmp(&self.position(right.1)))
            });
            return Ok(());
        }
        entries.sort_unstable();
        Ok(())
    }
    pub fn record(&self, index: u64) -> (R, &[u8]) {
        if index & SPILLED != 0 {
            let spill = unsafe { &*Self::spilled_record(index) };
            let bytes = unsafe {
                std::slice::from_raw_parts(spill.bytes.as_ptr().cast::<u8>(), spill.bytes.len())
            };
            return (unsafe { *spill.record.get() }, bytes);
        }
        let record = unsafe { *self.slot(index).record.get() };
        let bytes = unsafe { std::slice::from_raw_parts(self.byte_pointer(index), self.stride) };
        (record, bytes)
    }
    pub fn release(&self, index: u64) {
        if index & SPILLED != 0 {
            unsafe { &*Self::spilled_record(index) }
                .released
                .store(true, Ordering::Release);
            return;
        }
        self.slot(index).mark.fetch_or(RELEASED, Ordering::Release);
    }
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
        if self.spilled.load(Ordering::Acquire) != 0 {
            let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
            let mut previous: Option<&Spill<R>> = None;
            let mut next = taken.0;
            while let Some(spill) = unsafe { next.as_ref() } {
                let current = next;
                next = spill.next.load(Ordering::Relaxed);
                if !spill.released.load(Ordering::Acquire) {
                    previous = Some(spill);
                    continue;
                }
                match previous {
                    Some(previous) => previous.next.store(next, Ordering::Relaxed),
                    None => taken.0 = next,
                }
                self.free(current);
            }
        }
        if tail == end && self.spilled.load(Ordering::Acquire) == 0 {
            self.consumed_rejected.store(rejected, Ordering::Release);
        }
    }
    fn free(&self, spill: *mut Spill<R>) {
        let spill = unsafe { Box::from_raw(spill) };
        self.spilled_bytes
            .fetch_sub(spill.bytes.len(), Ordering::Relaxed);
        drop(spill);
        self.spilled.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<R> Drop for SharedLane<R> {
    fn drop(&mut self) {
        for mut next in [
            *self.pushed.get_mut(),
            self.taken
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .0,
        ] {
            while !next.is_null() {
                next = unsafe { Box::from_raw(next) }.next.into_inner();
            }
        }
    }
}

enum At<R> {
    Slot(u64),
    Spill(*mut Spill<R>),
}

pub struct Reservation<'a, R: Copy + Default> {
    lane: &'a SharedLane<R>,
    at: At<R>,
    published: bool,
}
impl<R: Copy + Default> Reservation<'_, R> {
    pub fn bytes(&mut self) -> *mut u8 {
        match self.at {
            At::Slot(index) => self.lane.byte_pointer(index),
            At::Spill(spill) => unsafe { (&*spill).bytes.as_ptr().cast::<u8>().cast_mut() },
        }
    }
    pub fn stride(&self) -> usize {
        match self.at {
            At::Slot(_) => self.lane.stride,
            At::Spill(spill) => unsafe { (&*spill).bytes.len() },
        }
    }
    pub fn publish(mut self, record: R) {
        match self.at {
            At::Slot(index) => {
                let slot = self.lane.slot(index);
                unsafe { *slot.record.get() = record };
                slot.mark.store(index + 1, Ordering::Release);
            }
            At::Spill(spill) => {
                unsafe { *(&*spill).record.get() = record };
                let mut head = self.lane.pushed.load(Ordering::Relaxed);
                loop {
                    unsafe { (&*spill).next.store(head, Ordering::Relaxed) };
                    match self.lane.pushed.compare_exchange_weak(
                        head,
                        spill,
                        Ordering::Release,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(current) => head = current,
                    }
                }
            }
        }
        self.published = true;
    }
}
impl<R: Copy + Default> Drop for Reservation<'_, R> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        match self.at {
            At::Slot(index) => self
                .lane
                .slot(index)
                .mark
                .store((index + 1) | VOID, Ordering::Release),
            At::Spill(spill) => self.lane.free(spill),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_publication_frees_only_a_released_prefix() {
        let lane = SharedLane::<u64>::new(2, 1).unwrap();
        let first = lane.try_reserve().unwrap();
        let second = lane.try_reserve().unwrap();
        assert!(lane.try_reserve().is_none());
        assert_eq!(lane.rejected(), 1);
        assert!(lane.required_entries() >= 3);
        second.publish(7);
        assert_eq!(lane.published(|&r| r).unwrap(), vec![(7, 1)]);
        lane.release(1);
        lane.advance(lane.rejected());
        assert!(lane.try_reserve().is_none());
        first.publish(9);
        assert_eq!(lane.published(|&r| r).unwrap(), vec![(9, 0)]);
        lane.release(0);
        lane.advance(lane.rejected());
        let third = lane.try_reserve().unwrap();
        drop(third);
        assert!(lane.published(|&r| r).unwrap().is_empty());
        lane.advance(lane.rejected());
        assert!(lane.try_reserve().is_some());
    }

    fn write(reservation: &mut Reservation<'_, u64>, value: u64) {
        let len = reservation.stride();
        let bytes = unsafe { std::slice::from_raw_parts_mut(reservation.bytes(), len) };
        for (at, byte) in bytes.iter_mut().enumerate() {
            *byte = (value as u8).wrapping_add(at as u8);
        }
    }

    #[test]
    fn a_burst_past_the_ring_keeps_every_record_in_order_without_moving_the_ring() {
        let lane = SharedLane::<u64>::new(4, 3).unwrap();
        let ring = lane.byte_pointer(0);
        let burst = 4 * lane.len() as u64 + 3;
        for value in 0..burst {
            let mut reservation = lane.reserve().unwrap();
            write(&mut reservation, value);
            reservation.publish(value);
        }
        let mut long = lane.reserve_bytes(9).unwrap();
        assert_eq!(long.stride(), 9);
        write(&mut long, burst);
        long.publish(burst);
        assert_eq!(lane.byte_pointer(0), ring);
        assert_eq!(lane.rejected(), 0);
        assert_eq!(lane.pending(), burst as usize + 1);
        assert!(lane.required_entries() >= burst as usize);
        assert!(!lane.is_drained());
        let published = lane.published(|&r| r).unwrap();
        assert_eq!(
            published
                .iter()
                .map(|&(order, _)| order)
                .collect::<Vec<_>>(),
            (0..=burst).collect::<Vec<_>>()
        );
        for &(order, index) in &published {
            let (record, bytes) = lane.record(index);
            assert_eq!(record, order);
            assert_eq!(bytes.len(), if order == burst { 9 } else { 3 });
            assert!(
                bytes
                    .iter()
                    .enumerate()
                    .all(|(at, &byte)| byte == (order as u8).wrapping_add(at as u8))
            );
        }
        let (_, kept) = published[published.len() - 2];
        for &(_, index) in &published {
            if index != kept {
                lane.release(index);
            }
        }
        lane.advance(lane.rejected());
        assert_eq!(lane.published(|&r| r).unwrap(), vec![(burst - 1, kept)]);
        assert_eq!(lane.pending(), 1);
        lane.release(kept);
        lane.advance(lane.rejected());
        assert!(lane.is_drained());
        assert_eq!(lane.allocated_bytes(), 4 * size_of::<Slot<u64>>() + 12);
        assert!(lane.published(|&r| r).unwrap().is_empty());
    }

    #[test]
    fn records_of_one_order_keep_their_publication_order_across_ring_and_spill() {
        let lane = SharedLane::<(u64, u64)>::new(2, 0).unwrap();
        let mut next = 0;
        let mut publish = |lane: &SharedLane<(u64, u64)>| {
            lane.reserve().unwrap().publish((7, next));
            next += 1;
        };
        for _ in 0..5 {
            publish(&lane);
        }
        let published = lane.published(|&(order, _)| order).unwrap();
        for &(_, index) in &published[..2] {
            lane.release(index);
        }
        lane.advance(lane.rejected());
        for _ in 0..3 {
            publish(&lane);
        }
        let order: Vec<_> = lane
            .published(|&(order, _)| order)
            .unwrap()
            .into_iter()
            .map(|(_, index)| lane.record(index).0.1)
            .collect();
        assert_eq!(order, (2..8).collect::<Vec<_>>());
    }

    #[test]
    fn producers_on_many_threads_past_the_ring_lose_nothing_and_never_wait_for_the_consumer() {
        let lane = SharedLane::<u64>::new(8, 0).unwrap();
        let threads = 4_u64;
        let each = 2_000_u64;
        std::thread::scope(|scope| {
            let taken = lane.taken.lock().unwrap();
            let producers: Vec<_> = (0..threads)
                .map(|thread| {
                    let lane = &lane;
                    scope.spawn(move || {
                        for at in 0..each {
                            lane.reserve().unwrap().publish(thread * each + at);
                        }
                    })
                })
                .collect();
            for producer in producers {
                producer.join().unwrap();
            }
            drop(taken);
        });
        let published = lane.published(|&r| r).unwrap();
        assert_eq!(
            published
                .iter()
                .map(|&(order, _)| order)
                .collect::<Vec<_>>(),
            (0..threads * each).collect::<Vec<_>>()
        );
        assert_eq!(lane.rejected(), 0);
    }

    #[test]
    fn only_a_refused_allocation_refuses_and_an_abandoned_spill_is_freed() {
        let lane = SharedLane::<u64>::new(2, 4).unwrap();
        for value in 0..2 {
            lane.reserve().unwrap().publish(value);
        }
        let abandoned = lane.reserve().unwrap();
        assert_eq!(lane.pending(), 3);
        drop(abandoned);
        assert_eq!(lane.pending(), 2);
        assert!(collector_test_support::with_allocations_denied(|| lane.reserve()).is_none());
        assert!(
            collector_test_support::with_allocations_denied(|| lane.reserve_bytes(5)).is_none()
        );
        assert_eq!(lane.rejected(), 2);
        assert!(lane.required_entries() >= 4);
        let published = lane.published(|&r| r).unwrap();
        assert_eq!(published, vec![(0, 0), (1, 1)]);
        for (_, index) in published {
            lane.release(index);
        }
        lane.advance(lane.rejected());
        assert!(lane.is_drained());
        assert!(lane.reserve().is_some());
    }
}
