//! Prepared scalar operands and expiry counts. Row reduction belongs to Slow;
//! grant snapshots settle their owner's earlier expiry counts before reanchoring.
use super::WatchRegistry;
use std::sync::{
    RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Slot {
    pointer: usize,
    value: AtomicU64,
}
struct Pending {
    watches: WatchRegistry,
    slots: RwLock<Vec<Slot>>,
    lost: AtomicBool,
    missed: AtomicU64,
}
impl Pending {
    const fn new() -> Self {
        Self {
            watches: WatchRegistry::marking(),
            slots: RwLock::new(Vec::new()),
            lost: AtomicBool::new(false),
            missed: AtomicU64::new(0),
        }
    }
    fn lose(&self) {
        self.missed.fetch_add(1, Ordering::Relaxed);
        self.lost.store(true, Ordering::Release);
    }
    fn prepare(
        &self,
        count: usize,
        pointers: impl Iterator<Item = usize>,
    ) -> Result<(), &'static str> {
        let mut prepared = engine_allocation::vec(count)
            .map_err(|_| "doctrine scalar receiver allocation failed")?;
        for pointer in pointers {
            engine_allocation::reserve(&mut prepared, 1)
                .map_err(|_| "doctrine scalar receiver allocation failed")?;
            prepared.push(Slot {
                pointer,
                value: AtomicU64::new(0),
            });
        }
        prepared.sort_unstable_by_key(|slot| slot.pointer);
        let mut slots = self
            .slots
            .write()
            .map_err(|_| "doctrine scalar receivers poisoned")?;
        self.watches.collect();
        self.watches.offer(prepared.len());
        self.watches.boundary(true);
        for slot in &prepared {
            self.watches
                .register(slot.pointer)
                .ok_or("doctrine scalar receiver registration unavailable")?;
        }
        self.watches
            .take(|_| {})
            .ok_or("doctrine scalar preparation overlapped a writer")?;
        *slots = prepared;
        Ok(())
    }
    fn publish(&self, pointer: usize, value: Option<i64>) {
        let notification = self.watches.notify(pointer);
        if !notification.has_mark() {
            self.lose();
            return;
        }
        // Only preparation takes the exclusive lock. A callback never waits
        // for it and never touches the doctrine Store or its row journals.
        let Ok(slots) = self.slots.try_read() else {
            self.lose();
            return;
        };
        let Ok(index) = slots.binary_search_by_key(&pointer, |slot| slot.pointer) else {
            self.lose();
            return;
        };
        if let Some(value) = value {
            slots[index].value.store(value as u64, Ordering::Relaxed);
        } else if slots[index]
            .value
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_add(1)
            })
            .is_err()
        {
            self.lose();
        }
    }
    fn take(&self, mut visit: impl FnMut(usize, u64)) -> Result<(), &'static str> {
        let slots = self
            .slots
            .read()
            .map_err(|_| "doctrine scalar receivers poisoned")?;
        self.watches
            .take(|pointer| {
                let index = slots
                    .binary_search_by_key(&pointer, |slot| slot.pointer)
                    .expect("prepared doctrine scalar receiver");
                visit(pointer, slots[index].value.swap(0, Ordering::Relaxed));
            })
            .ok_or("doctrine scalar marks overlapped a writer")?;
        Ok(())
    }
    fn take_one(&self, pointer: usize) -> Result<u64, &'static str> {
        let slots = self
            .slots
            .read()
            .map_err(|_| "doctrine scalar receivers poisoned")?;
        let index = slots
            .binary_search_by_key(&pointer, |slot| slot.pointer)
            .map_err(|_| "doctrine scalar receiver unavailable")?;
        Ok(slots[index].value.swap(0, Ordering::Relaxed))
    }
    fn take_lost(&self) -> bool {
        self.lost.swap(false, Ordering::AcqRel)
    }
    fn status(&self) -> impl serde::Serialize {
        crate::status::status!({
            "lost": self.lost.load(Ordering::Acquire),
            "missed_operands": self.missed.load(Ordering::Relaxed),
            "watches": self.watches.status()
        })
    }
}
static GAINS: Pending = Pending::new();
static EXPIRIES: Pending = Pending::new();

pub fn prepare_gains(
    count: usize,
    pointers: impl Iterator<Item = usize>,
) -> Result<(), &'static str> {
    GAINS.prepare(count, pointers)
}
pub fn prepare_expiries(
    count: usize,
    pointers: impl Iterator<Item = usize>,
) -> Result<(), &'static str> {
    EXPIRIES.prepare(count, pointers)
}
pub fn defer_gain(pointer: usize, value: i64) {
    GAINS.publish(pointer, Some(value));
}
pub fn defer_expiry(pointer: usize) {
    EXPIRIES.publish(pointer, None);
}
pub fn take_gains(mut visit: impl FnMut(usize, i64)) -> Result<(), &'static str> {
    GAINS.take(|pointer, value| visit(pointer, value as i64))
}
pub fn take_expiries(visit: impl FnMut(usize, u64)) -> Result<(), &'static str> {
    EXPIRIES.take(visit)
}
pub fn take_expiry(pointer: usize) -> Result<u64, &'static str> {
    EXPIRIES.take_one(pointer)
}
pub(super) fn take_lost() -> bool {
    GAINS.take_lost() | EXPIRIES.take_lost()
}
pub(super) fn active() -> usize {
    GAINS.watches.active() + EXPIRIES.watches.active()
}
pub(super) fn enabled(enabled: bool) {
    GAINS.watches.set_enabled(enabled);
    EXPIRIES.watches.set_enabled(enabled);
}
pub(super) fn invalidate() {
    GAINS.watches.boundary(true);
    EXPIRIES.watches.boundary(true);
}
/// # Safety
/// The doctrine callbacks and acquisition worker have quiesced.
pub(super) unsafe fn release() {
    for pending in [&GAINS, &EXPIRIES] {
        unsafe { pending.watches.release() };
        *pending
            .slots
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Vec::new();
    }
}
pub(super) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "gains": GAINS.status(),
        "expiries": EXPIRIES.status()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_slots_keep_last_gain_and_count_expiries_without_callback_allocation() {
        let gains = Pending::new();
        let expiries = Pending::new();
        for pending in [&gains, &expiries] {
            pending.watches.set_enabled(true);
            pending.prepare(2, [9, 7].into_iter()).unwrap();
        }
        collector_test_support::with_allocations_denied(|| {
            gains.publish(7, Some(-1));
            gains.publish(7, Some(i64::MIN));
            gains.publish(9, Some(0));
            for _ in 0..12 {
                expiries.publish(7, None);
            }
        });
        let mut values = Vec::new();
        gains
            .take(|pointer, value| values.push((pointer, value as i64)))
            .unwrap();
        values.sort_unstable();
        assert_eq!(values, [(7, i64::MIN), (9, 0)]);
        assert_eq!(expiries.take_one(7), Ok(12));
        expiries.publish(7, None);
        expiries
            .take(|pointer, count| assert_eq!((pointer, count), (7, 1)))
            .unwrap();
        gains
            .take(|_, _| panic!("quiet scalar published twice"))
            .unwrap();
        assert!(!gains.take_lost() && !expiries.take_lost());
        for pending in [&gains, &expiries] {
            unsafe { pending.watches.release() };
        }
    }
    #[test]
    fn unknown_receivers_and_preparation_overlap_lose_the_observation() {
        let pending = Pending::new();
        pending.watches.set_enabled(true);
        pending.prepare(1, [7].into_iter()).unwrap();
        pending.publish(9, Some(1));
        assert!(pending.take_lost());
        let preparing = pending.slots.write().unwrap();
        pending.publish(7, Some(2));
        assert!(pending.take_lost());
        drop(preparing);
        pending.prepare(1, [7].into_iter()).unwrap();
        pending
            .take(|_, _| panic!("lost gain survived a baseline"))
            .unwrap();
        unsafe { pending.watches.release() };
    }
}
