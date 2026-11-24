
use collector_primitives::asymmetric::{FlightGate, FlightGuard};
use std::{
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr, AtomicU64, AtomicUsize,
        Ordering::{Relaxed, SeqCst},
    },
};

const BITS: usize = u64::BITS as usize;

struct Bits {
    words: Vec<AtomicU64>,
}

impl Bits {
    fn new(words: usize) -> Option<Box<Self>> {
        let mut bits = engine_allocation::vec(words).ok()?;
        bits.resize_with(words, AtomicU64::default);
        engine_allocation::boxed(Self { words: bits }).ok()
    }

    fn ids(&self) -> usize {
        self.words.len() * BITS
    }
}

/// Frees a bitmap.
///
/// # Safety
/// `bits` is null or came from `Box::into_raw`, and no writer or consumer can reach it any more.
unsafe fn free(bits: *mut Bits) {
    if !bits.is_null() {
        drop(unsafe { Box::from_raw(bits) });
    }
}

pub struct DirtySet {
    bits: AtomicPtr<Bits>,
    offered: AtomicPtr<Bits>,
    draining: AtomicPtr<Bits>,
    retired: AtomicPtr<Bits>,
    capacity: AtomicUsize,
    requested: AtomicUsize,
    epoch: AtomicU64,
    active: FlightGate,
    enabled: AtomicBool,
    poisoned: AtomicBool,
    installs: AtomicU64,
    deferred: AtomicU64,
    held: AtomicU64,
    failed_offers: AtomicU64,
    overflows: AtomicU64,
    demand: AtomicUsize,
    outgrown: AtomicBool,
    everies: AtomicU64,
    taken: AtomicU64,
    refused: AtomicU64,
}

impl Default for DirtySet {
    fn default() -> Self {
        Self::new()
    }
}

impl DirtySet {
    pub const fn new() -> Self {
        Self {
            bits: AtomicPtr::new(ptr::null_mut()),
            offered: AtomicPtr::new(ptr::null_mut()),
            draining: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
            capacity: AtomicUsize::new(0),
            requested: AtomicUsize::new(0),
            epoch: AtomicU64::new(1),
            active: FlightGate::new(0),
            enabled: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            installs: AtomicU64::new(0),
            deferred: AtomicU64::new(0),
            held: AtomicU64::new(0),
            failed_offers: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            demand: AtomicUsize::new(0),
            outgrown: AtomicBool::new(false),
            everies: AtomicU64::new(0),
            taken: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        }
    }

    #[inline]
    fn installed(&self) -> Option<&Bits> {
        // SAFETY: during capture a bitmap is freed only after a boundary replaced it and saw no
        // writer running, and consumers run only after the boundary; teardown (`release`) frees
        // the installed bitmap once callbacks and acquisition readers have quiesced.
        unsafe { self.bits.load(SeqCst).as_ref() }
    }

    fn new_epoch(&self) {
        if self.epoch.fetch_add(1, SeqCst) == u64::MAX {
            self.poison();
        }
    }

    fn observing(&self) -> bool {
        self.enabled.load(SeqCst) && !self.poisoned.load(SeqCst)
    }

    #[inline]
    fn mark(&self, id: usize) -> Mark<'_> {
        match self.installed().and_then(|bits| bits.words.get(id / BITS)) {
            Some(word) => {
                let bit = 1 << (id % BITS);
                if word.load(SeqCst) & bit == 0 {
                    word.fetch_or(bit, SeqCst);
                }
                Mark::Word(word, bit)
            }
            None => {
                self.overflows.fetch_add(1, Relaxed);
                self.demand.fetch_max(id.saturating_add(1), SeqCst);
                self.outgrown.store(true, SeqCst);
                self.new_epoch();
                Mark::Overflow(id)
            }
        }
    }

    #[inline]
    pub fn notify(&self, id: usize) -> Dirty<'_> {
        let flight = self.active.enter(false);
        let mark = self.mark(id);
        Dirty {
            set: self,
            mark,
            _flight: flight,
        }
    }

    pub fn notify_every(&self) -> Dirty<'_> {
        let flight = self.active.enter(false);
        self.everies.fetch_add(1, Relaxed);
        self.new_epoch();
        Dirty {
            set: self,
            mark: Mark::Every,
            _flight: flight,
        }
    }

    pub fn take(&self, mut visit: impl FnMut(usize)) -> Option<u64> {
        if !self.observing() {
            return None;
        }
        if self.active.any_active() {
            self.refused.fetch_add(1, Relaxed);
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let bits = self.installed()?;
        let mut taken = 0;
        for (index, word) in bits.words.iter().enumerate() {
            if word.load(Relaxed) == 0 {
                continue;
            }
            let mut value = word.swap(0, SeqCst);
            while value != 0 {
                visit(index * BITS + value.trailing_zeros() as usize);
                value &= value - 1;
                taken += 1;
            }
        }
        self.taken.fetch_add(taken, Relaxed);
        if self.epoch.load(SeqCst) == epoch && !self.active.any_active() {
            Some(epoch)
        } else {
            self.refused.fetch_add(1, Relaxed);
            None
        }
    }

    pub fn boundary(&self, invalidate: bool) -> bool {
        if invalidate {
            self.new_epoch();
        }
        let first = self.bits.load(SeqCst).is_null();
        if !invalidate && !first && !self.outgrown.load(SeqCst) {
            if !self.offered.load(SeqCst).is_null() {
                self.held.fetch_add(1, Relaxed);
            }
            if !self.draining.load(SeqCst).is_null() && !self.active.any_active() {
                self.retire_drained();
            }
            return false;
        }
        if self.active.any_active() {
            if !self.offered.load(SeqCst).is_null() || !self.draining.load(SeqCst).is_null() {
                self.deferred.fetch_add(1, Relaxed);
            }
            return false;
        }
        self.retire_drained();
        if !self.draining.load(SeqCst).is_null() || !self.retired.load(SeqCst).is_null() {
            return false;
        }
        let next = self.offered.swap(ptr::null_mut(), SeqCst);
        // SAFETY: an offered bitmap is owned here once swapped out.
        let Some(ids) = unsafe { next.as_ref() }.map(Bits::ids) else {
            return false;
        };
        self.new_epoch();
        let previous = self.bits.swap(next, SeqCst);
        self.new_epoch();
        self.capacity.store(ids, Relaxed);
        if ids >= self.demand.load(SeqCst) {
            self.outgrown.store(false, SeqCst);
        }
        self.installs.fetch_add(1, Relaxed);
        self.draining.store(previous, SeqCst);
        if !self.active.any_active() {
            self.retire_drained();
        }
        true
    }

    fn retire_drained(&self) {
        let drained = self.draining.load(SeqCst);
        if !drained.is_null()
            && self
                .retired
                .compare_exchange(ptr::null_mut(), drained, SeqCst, SeqCst)
                .is_ok()
        {
            self.draining.store(ptr::null_mut(), SeqCst);
        }
    }

    pub fn offer(&self, ids: usize) {
        let ids = ids.max(self.demand.load(SeqCst));
        let Some(words) = ids.checked_add(BITS - 1).map(|bits| bits / BITS) else {
            let _ = engine_allocation::oom();
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        let words = words.max(1);
        if words * BITS <= self.requested.load(SeqCst) {
            return;
        }
        let Some(bits) = Bits::new(words) else {
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        self.requested.store(words * BITS, SeqCst);
        let previous = self.offered.swap(Box::into_raw(bits), SeqCst);
        // SAFETY: an offered bitmap was never installed; the swap owns it.
        unsafe { free(previous) };
    }

    pub fn collect(&self) {
        let retired = self.retired.swap(ptr::null_mut(), SeqCst);
        // SAFETY: only a drained bitmap is retired, and the swap owns it.
        unsafe { free(retired) };
    }

    /// Free every bitmap.
    ///
    /// # Safety
    /// No writer or consumer may run: the hooks are removed and drained, and
    /// the acquisition pool has stopped.
    pub unsafe fn release(&self) {
        for bits in [&self.bits, &self.offered, &self.draining, &self.retired] {
            unsafe { free(bits.swap(ptr::null_mut(), SeqCst)) };
        }
        self.capacity.store(0, Relaxed);
        self.requested.store(0, SeqCst);
        self.demand.store(0, SeqCst);
        self.outgrown.store(false, SeqCst);
        self.new_epoch();
    }

    pub fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.new_epoch();
        }
        self.enabled.store(enabled, SeqCst);
    }

    pub fn poison(&self) {
        self.poisoned.store(true, SeqCst);
    }

    pub fn poisoned(&self) -> bool {
        self.poisoned.load(SeqCst)
    }

    pub fn active(&self) -> usize {
        self.active.active()
    }

}

impl Drop for DirtySet {
    fn drop(&mut self) {
        // SAFETY: exclusive access; nothing else can reach the bitmaps.
        unsafe { self.release() };
    }
}

enum Mark<'a> {
    Word(&'a AtomicU64, u64),
    Overflow(usize),
    Every,
}

#[must_use]
pub struct Dirty<'a> {
    set: &'a DirtySet,
    mark: Mark<'a>,
    _flight: FlightGuard<'a>,
}

impl Drop for Dirty<'_> {
    #[inline]
    fn drop(&mut self) {
        match self.mark {
            Mark::Word(word, bit) => {
                if word.load(SeqCst) & bit == 0 {
                    word.fetch_or(bit, SeqCst);
                }
            }
            Mark::Overflow(id) => {
                let _ = self.set.mark(id);
            }
            Mark::Every => self.set.new_epoch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: usize) -> DirtySet {
        let set = DirtySet::new();
        set.offer(ids);
        assert!(set.boundary(false), "the first bitmap installs at once");
        set.set_enabled(true);
        set
    }

    fn take(set: &DirtySet) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = set.take(|id| ids.push(id));
        (epoch, ids)
    }

    #[test]
    fn marked_ids_are_taken_once_in_order_within_one_epoch() {
        let set = set(200);
        let (epoch, ids) = take(&set);
        assert!(ids.is_empty());
        let epoch = epoch.unwrap();
        for id in [130, 3, 64, 3] {
            drop(set.notify(id));
        }
        assert_eq!(take(&set), (Some(epoch), vec![3, 64, 130]));
        assert_eq!(take(&set), (Some(epoch), vec![]));
        assert_eq!(set.taken.load(Relaxed), 3);
    }

    #[test]
    fn a_writer_spanning_the_take_refuses_it_and_keeps_its_bit() {
        let set = set(64);
        let (Some(epoch), _) = take(&set) else {
            unreachable!("observing")
        };
        let running = set.notify(9);
        assert_eq!(take(&set), (None, vec![]));
        assert_eq!(set.refused.load(Relaxed), 1);
        drop(running);
        assert_eq!(take(&set), (Some(epoch), vec![9]));
    }

    #[test]
    fn bits_set_during_or_after_a_take_survive_to_the_next_take() {
        for (writer, trigger, spans) in [(5, 70, false), (5, 70, true), (100, 2, false)] {
            let set = set(128);
            let (Some(epoch), _) = take(&set) else {
                unreachable!("observing")
            };
            drop(set.notify(trigger));
            let mut running = None;
            let mut visited = Vec::new();
            let certified = set.take(|id| {
                visited.push(id);
                if id == trigger {
                    let guard = set.notify(writer);
                    if spans {
                        running = Some(guard);
                    }
                }
            });
            let swapped_before = writer / BITS < trigger / BITS;
            if spans {
                assert_eq!(certified, None);
            } else {
                assert_eq!(certified, Some(epoch));
            }
            assert_eq!(visited.contains(&writer), !swapped_before);
            drop(running);
            let (next, later) = take(&set);
            assert_eq!(next, Some(epoch));
            assert_eq!(later.contains(&writer), swapped_before);
            assert!(visited.contains(&writer) || later.contains(&writer));
        }
    }

    #[test]
    fn repeated_marks_do_not_clear_siblings_and_return_marks_survive_a_take() {
        let set = set(128);
        let epoch = take(&set).0.unwrap();
        drop(set.notify(5));
        drop(set.notify(6));
        let first = set.notify(5);
        let second = set.notify(5);
        assert_eq!(set.active(), 2);
        assert_eq!(take(&set), (None, vec![]));
        drop(first);
        assert_eq!(take(&set), (None, vec![]));
        drop(second);
        assert_eq!(take(&set), (Some(epoch), vec![5, 6]));
        drop(set.notify(70));
        let mut running = None;
        assert_eq!(
            set.take(|id| {
                assert_eq!(id, 70);
                drop(set.notify(5));
                running = Some(set.notify(5));
            }),
            None
        );
        drop(running);
        assert_eq!(take(&set), (Some(epoch), vec![5]));
        assert_eq!(take(&set), (Some(epoch), vec![]));
    }

    #[test]
    fn an_unnamed_writer_moves_the_epoch_on_entry_and_return() {
        let set = set(64);
        let (Some(before), _) = take(&set) else {
            unreachable!("observing")
        };
        let running = set.notify_every();
        assert_eq!(take(&set).0, None);
        drop(running);
        let (after, _) = take(&set);
        let after = after.unwrap();
        assert!(after >= before + 2);
        assert_eq!(set.everies.load(Relaxed), 1);
    }

    #[test]
    fn an_id_beyond_capacity_moves_the_epoch() {
        let set = set(64);
        let (Some(before), _) = take(&set) else {
            unreachable!("observing")
        };
        drop(set.notify(64));
        assert_eq!(set.overflows.load(Relaxed), 2);
        let (after, ids) = take(&set);
        assert!(ids.is_empty());
        assert_ne!(after, Some(before));
        let empty = DirtySet::new();
        empty.set_enabled(true);
        drop(empty.notify(0));
        assert_eq!(empty.overflows.load(Relaxed), 2);
        assert_eq!(take(&empty).0, None);
    }

    #[test]
    fn an_id_beyond_capacity_grows_the_bitmap_at_the_next_hour() {
        let set = set(64);
        drop(set.notify(200));
        assert_eq!(set.demand.load(SeqCst), 201);
        set.offer(10);
        assert!(set.boundary(false));
        assert!(set.capacity.load(Relaxed) > 200);
        let (Some(epoch), _) = take(&set) else {
            unreachable!("observing")
        };
        drop(set.notify(200));
        assert_eq!(take(&set), (Some(epoch), vec![200]));
        set.offer(1_000);
        assert!(!set.boundary(false));
        assert_eq!(set.held.load(Relaxed), 1);
    }

    #[test]
    fn observation_is_off_until_enabled_and_after_poison() {
        let set = DirtySet::new();
        set.offer(64);
        set.boundary(false);
        drop(set.notify(1));
        assert_eq!(take(&set), (None, vec![]));
        let before = set.epoch.load(SeqCst);
        set.set_enabled(true);
        assert!(set.epoch.load(SeqCst) > before);
        assert_eq!(take(&set).1, vec![1]);
        set.poison();
        drop(set.notify(2));
        assert_eq!(take(&set), (None, vec![]));
        assert!(set.poisoned());
    }

    #[test]
    fn an_invalidating_boundary_moves_the_epoch_and_keeps_bits() {
        let set = set(64);
        let (Some(before), _) = take(&set) else {
            unreachable!("observing")
        };
        drop(set.notify(4));
        assert!(!set.boundary(false));
        assert_eq!(take(&set), (Some(before), vec![4]));
        drop(set.notify(4));
        set.boundary(true);
        let (after, ids) = take(&set);
        assert_ne!(after, Some(before));
        assert_eq!(ids, vec![4]);
    }

    #[test]
    fn a_larger_bitmap_installs_at_a_quiet_invalidating_hour_and_is_freed_off_thread() {
        let set = set(64);
        let (Some(before), _) = take(&set) else {
            unreachable!("observing")
        };
        set.offer(10);
        assert!(set.offered.load(SeqCst).is_null());
        set.offer(1_000);
        assert!(!set.boundary(false));
        assert_eq!(set.capacity.load(Relaxed), 64);
        assert_eq!(set.held.load(Relaxed), 1);
        assert_eq!(take(&set).0, Some(before));
        let running = set.notify(3);
        assert!(!set.boundary(true));
        assert_eq!(set.deferred.load(Relaxed), 1);
        drop(running);
        assert!(!set.boundary(false));
        assert_eq!(set.capacity.load(Relaxed), 64);
        drop(set.notify(999));
        assert!(set.boundary(true));
        assert_eq!(set.capacity.load(Relaxed), 1_024);
        assert_eq!(set.installs.load(Relaxed), 2);
        let (after, ids) = take(&set);
        assert_ne!(after, Some(before));
        assert!(ids.is_empty());
        drop(set.notify(999));
        assert_eq!(take(&set).1, vec![999]);
        assert!(set.draining.load(SeqCst).is_null());
        assert!(!set.retired.load(SeqCst).is_null());
        set.collect();
        assert!(set.retired.load(SeqCst).is_null());
    }

    #[test]
    fn an_unrepresentable_offer_reports_failure_and_keeps_the_installed_bitmap() {
        let set = set(64);
        let installed = set.bits.load(SeqCst);
        set.offer(usize::MAX);
        assert_eq!(set.failed_offers.load(Relaxed), 1);
        assert_eq!(set.bits.load(SeqCst), installed);
        assert!(set.offered.load(SeqCst).is_null());
    }
}
