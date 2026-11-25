use super::WatchRegistry;
use std::sync::{
    RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Slot {
    pointer: usize,
    value: AtomicU64,
}
#[derive(Default)]
struct Table {
    slots: Vec<Slot>,
}
impl Table {
    fn from_pointers(
        count: usize,
        pointers: impl Iterator<Item = usize>,
    ) -> Result<Self, &'static str> {
        let mut keys = engine_allocation::vec(count)
            .map_err(|_| "doctrine scalar receiver allocation failed")?;
        for pointer in pointers {
            if pointer == 0 {
                return Err("doctrine scalar receiver pointer is null");
            }
            engine_allocation::reserve(&mut keys, 1)
                .map_err(|_| "doctrine scalar receiver allocation failed")?;
            keys.push(pointer);
        }
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Ok(Self { slots: Vec::new() });
        }
        let len = keys
            .len()
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or("doctrine scalar receiver allocation failed")?;
        let mut slots = engine_allocation::vec(len)
            .map_err(|_| "doctrine scalar receiver allocation failed")?;
        slots.resize_with(len, || Slot {
            pointer: 0,
            value: AtomicU64::new(0),
        });
        let mask = len - 1;
        for pointer in keys {
            let first = collector_primitives::table_hash::slot(pointer as u64, len);
            let mut probe = 0;
            while slots[(first + probe) & mask].pointer != 0 {
                probe += 1;
            }
            slots[(first + probe) & mask].pointer = pointer;
        }
        Ok(Self { slots })
    }
    #[inline]
    fn get(&self, pointer: usize) -> Option<&Slot> {
        if pointer == 0 || self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let first = collector_primitives::table_hash::slot(pointer as u64, self.slots.len());
        for probe in 0..self.slots.len() {
            let slot = &self.slots[(first + probe) & mask];
            if slot.pointer == pointer {
                return Some(slot);
            }
            if slot.pointer == 0 {
                return None;
            }
        }
        None
    }
    fn pointers(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .filter_map(|slot| (slot.pointer != 0).then_some(slot.pointer))
    }
    fn count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.pointer != 0).count()
    }
}
struct Pending {
    watches: WatchRegistry,
    slots: RwLock<Table>,
    lost: AtomicBool,
    missed: AtomicU64,
}
impl Pending {
    const fn new() -> Self {
        Self {
            watches: WatchRegistry::marks_only(),
            slots: RwLock::new(Table { slots: Vec::new() }),
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
        let prepared = Table::from_pointers(count, pointers)?;
        let mut slots = self
            .slots
            .write()
            .map_err(|_| "doctrine scalar receivers poisoned")?;
        self.watches.collect();
        self.watches.offer(prepared.count());
        self.watches.boundary(true);
        for pointer in prepared.pointers() {
            self.watches
                .register(pointer)
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
        let Ok(slots) = self.slots.try_read() else {
            self.lose();
            return;
        };
        let Some(slot) = slots.get(pointer) else {
            self.lose();
            return;
        };
        if let Some(value) = value {
            slot.value.store(value as u64, Ordering::Relaxed);
        } else if slot
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
                let slot = slots
                    .get(pointer)
                    .expect("prepared doctrine scalar receiver");
                visit(pointer, slot.value.swap(0, Ordering::Relaxed));
            })
            .ok_or("doctrine scalar marks overlapped a writer")?;
        Ok(())
    }
    fn take_one(&self, pointer: usize) -> Result<u64, &'static str> {
        let slots = self
            .slots
            .read()
            .map_err(|_| "doctrine scalar receivers poisoned")?;
        let slot = slots
            .get(pointer)
            .ok_or("doctrine scalar receiver unavailable")?;
        Ok(slot.value.swap(0, Ordering::Relaxed))
    }
    fn take_lost(&self) -> bool {
        self.lost.swap(false, Ordering::AcqRel)
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
            .unwrap_or_else(|error| error.into_inner()) = Table::default();
    }
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
