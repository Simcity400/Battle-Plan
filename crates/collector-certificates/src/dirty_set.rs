//! A per-domain atomic bitmap of the dense ids (province or state ids) a
//! hooked writer touched, for a change-driven table with one consumer
//! (the territory observer).
//!
//! A [`crate::watch_registry::WatchRegistry`] certificate costs a hashed
//! random access per receiver, which at tens of thousands of receivers costs
//! about as much as reading them. Here a writer sets its id's bit and the
//! consumer swaps every word to zero once an hour, visiting each set bit, so
//! a quiet hour costs one load per 64 ids.
//!
//! The safety model is the watch registry's:
//! - [`DirtySet::notify`] sets the id's bit on entry and again when its guard
//!   drops, between which the writer counter covers the native write. An id
//!   beyond the installed capacity, or no bitmap at all, moves the epoch
//!   instead, as [`DirtySet::notify_every`] does on entry and on return.
//! - [`DirtySet::take`] runs on a worker after the hour's boundary. It
//!   refuses while observation is off or a writer runs; otherwise it swaps
//!   every word to zero, visiting each set id, and returns the epoch only
//!   when no epoch change and no writer overlapped it. A bit set after its
//!   word was swapped survives to the next take.
//! - A consumer carries an entry only when the returned epoch equals the
//!   epoch of its held complete capture, and otherwise reads everything. A
//!   take consumes bits, so a capture that does not complete must not be
//!   held: the next hour reads everything.
//! - The epoch moves on enabling (writers ran unobserved before it), on
//!   every invalidating [`DirtySet::boundary`], on each install (the old
//!   bitmap's bits are dropped), on an id beyond capacity and on
//!   `notify_every`.
//!
//! The hook path never allocates or frees. A bitmap is allocated off-thread
//! ([`DirtySet::offer`]), installed at a boundary only while no writer runs
//! (the first at once, a larger one at the next invalidating hour), and a
//! replaced one is freed off-thread ([`DirtySet::collect`]) once no writer
//! can still hold it.

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
    /// Off the hook path; `None` when the allocation fails.
    fn new(words: usize) -> Option<Box<Self>> {
        let mut bits = engine::allocation::vec(words).ok()?;
        bits.resize_with(words, AtomicU64::default);
        Some(Box::new(Self { words: bits }))
    }

    fn ids(&self) -> usize {
        self.words.len() * BITS
    }
}

/// Frees a bitmap no writer or consumer can reach any more.
unsafe fn free(bits: *mut Bits) {
    if !bits.is_null() {
        drop(unsafe { Box::from_raw(bits) });
    }
}

/// One domain: the dirty ids, epoch and running writers.
pub struct DirtySet {
    /// The installed bitmap; null until a boundary installs one.
    bits: AtomicPtr<Bits>,
    /// A larger bitmap allocated off-thread, awaiting a boundary.
    offered: AtomicPtr<Bits>,
    /// A replaced bitmap that a writer begun before its replacement may
    /// still hold.
    draining: AtomicPtr<Bits>,
    /// A replaced bitmap nothing holds, awaiting [`Self::collect`].
    retired: AtomicPtr<Bits>,
    /// Ids of the installed bitmap.
    capacity: AtomicUsize,
    /// Ids of the largest bitmap allocated, installed or offered.
    requested: AtomicUsize,
    epoch: AtomicU64,
    active: AtomicUsize,
    enabled: AtomicBool,
    poisoned: AtomicBool,
    installs: AtomicU64,
    deferred: AtomicU64,
    /// Boundaries outside an invalidation that left an offered bitmap for
    /// the next invalidating hour.
    held: AtomicU64,
    failed_offers: AtomicU64,
    /// Notifications of an id beyond the installed capacity.
    overflows: AtomicU64,
    /// Notifications that named no id.
    everies: AtomicU64,
    /// Ids the takes visited.
    taken: AtomicU64,
    /// Takes that certified nothing: a writer ran or the epoch moved.
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
            active: AtomicUsize::new(0),
            enabled: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            installs: AtomicU64::new(0),
            deferred: AtomicU64::new(0),
            held: AtomicU64::new(0),
            failed_offers: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            everies: AtomicU64::new(0),
            taken: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        }
    }

    fn installed(&self) -> Option<&Bits> {
        // SAFETY: a bitmap is freed only after a boundary replaced it and saw
        // no writer running; consumers run only after the boundary.
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

    /// Sets `id`'s bit, or moves the epoch when the installed bitmap cannot
    /// hold it.
    fn mark(&self, id: usize) {
        match self.installed().and_then(|bits| bits.words.get(id / BITS)) {
            Some(word) => {
                word.fetch_or(1 << (id % BITS), SeqCst);
            }
            None => {
                self.overflows.fetch_add(1, Relaxed);
                self.new_epoch();
            }
        }
    }

    /// Hook path, allocation-free: a writer is about to change `id`.
    /// Dropping the guard marks it again for the writer's return.
    pub fn notify(&self, id: usize) -> Dirty<'_> {
        self.active.fetch_add(1, SeqCst);
        self.mark(id);
        Dirty {
            set: self,
            id: Some(id),
        }
    }

    /// Hook path, allocation-free: a writer whose id cannot be named is about
    /// to run. It moves the epoch on entry and on return, so no capture
    /// certified before it, or while it runs, survives it.
    pub fn notify_every(&self) -> Dirty<'_> {
        self.active.fetch_add(1, SeqCst);
        self.everies.fetch_add(1, Relaxed);
        self.new_epoch();
        Dirty {
            set: self,
            id: None,
        }
    }

    /// The single consumer, after the hour's boundary: clears every marked
    /// id, visiting each, and returns the epoch they were marked in. `None`
    /// when observation is off, no bitmap is installed, a writer runs (its
    /// bits stay set) or the epoch moved during the take; nothing taken then
    /// certifies anything.
    pub fn take(&self, mut visit: impl FnMut(usize)) -> Option<u64> {
        if !self.observing() {
            return None;
        }
        if self.active.load(SeqCst) != 0 {
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
        if self.epoch.load(SeqCst) == epoch && self.active.load(SeqCst) == 0 {
            Some(epoch)
        } else {
            self.refused.fetch_add(1, Relaxed);
            None
        }
    }

    /// Game thread, in the hourly callback before any capture: start an
    /// epoch when `invalidate`, then install an offered bitmap while no
    /// writer is running. Outside an invalidation an offered bitmap waits,
    /// counted as held, unless none is installed yet; so does one a running
    /// writer blocks. Returns whether it installed a bitmap.
    pub fn boundary(&self, invalidate: bool) -> bool {
        if invalidate {
            self.new_epoch();
        }
        let first = self.bits.load(SeqCst).is_null();
        if !invalidate && !first {
            if !self.offered.load(SeqCst).is_null() {
                self.held.fetch_add(1, Relaxed);
            }
            if self.active.load(SeqCst) == 0 {
                self.retire_drained();
            }
            return false;
        }
        if self.active.load(SeqCst) != 0 {
            if !self.offered.load(SeqCst).is_null() || !self.draining.load(SeqCst).is_null() {
                self.deferred.fetch_add(1, Relaxed);
            }
            return false;
        }
        // No writer is running, so none holds a replaced bitmap.
        self.retire_drained();
        if !self.draining.load(SeqCst).is_null() || !self.retired.load(SeqCst).is_null() {
            return false;
        }
        let next = self.offered.swap(ptr::null_mut(), SeqCst);
        // SAFETY: an offered bitmap is owned here once swapped out.
        let Some(ids) = unsafe { next.as_ref() }.map(Bits::ids) else {
            return false;
        };
        // The replaced bitmap's bits are dropped: no epoch before this one
        // certifies anything.
        self.new_epoch();
        let previous = self.bits.swap(next, SeqCst);
        self.new_epoch();
        self.capacity.store(ids, Relaxed);
        self.installs.fetch_add(1, Relaxed);
        // A writer begun since the check above may hold it.
        self.draining.store(previous, SeqCst);
        if self.active.load(SeqCst) == 0 {
            self.retire_drained();
        }
        true
    }

    /// Hand a drained bitmap to [`Self::collect`] once the previous one is
    /// gone. Only while no writer is running.
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

    /// Off the game thread: allocate a bitmap for ids `0..ids` when the
    /// installed and offered ones are smaller. A boundary installs it.
    pub fn offer(&self, ids: usize) {
        let Some(words) = ids.checked_add(BITS - 1).map(|bits| bits / BITS) else {
            let _ = engine::allocation::oom();
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

    /// Off the game thread: free a replaced bitmap nothing holds any more.
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
        self.new_epoch();
    }

    /// Enabling starts an epoch: writers ran unobserved while disabled.
    pub fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.new_epoch();
        }
        self.enabled.store(enabled, SeqCst);
    }

    /// Disable reuse for the attachment.
    pub fn poison(&self) {
        self.poisoned.store(true, SeqCst);
    }

    pub fn poisoned(&self) -> bool {
        self.poisoned.load(SeqCst)
    }

    /// Writers running now.
    pub fn active(&self) -> usize {
        self.active.load(SeqCst)
    }

    pub fn status(&self) -> impl serde::Serialize {
        crate::status::status!({
            "capacity": self.capacity.load(Relaxed),
            "requested": self.requested.load(Relaxed),
            "installs": self.installs.load(Relaxed),
            "deferred_boundaries": self.deferred.load(Relaxed),
            "held_for_full_hour": self.held.load(Relaxed),
            "failed_offers": self.failed_offers.load(Relaxed),
            "overflow_notifications": self.overflows.load(Relaxed),
            "every_notifications": self.everies.load(Relaxed),
            "taken_ids": self.taken.load(Relaxed),
            "refused_takes": self.refused.load(Relaxed),
            "scope": "cumulative since attach; an id beyond capacity or a writer naming none moves the epoch, so every entry is read afresh; an install starts an epoch and waits for an invalidating hour (attach, load, gap or recovery), except the first"
        })
    }
}

impl Drop for DirtySet {
    fn drop(&mut self) {
        // SAFETY: exclusive access; nothing else can reach the bitmaps.
        unsafe { self.release() };
    }
}

/// A running writer; its drop marks the writer's return.
#[must_use]
pub struct Dirty<'a> {
    set: &'a DirtySet,
    /// `None`: the writer named no id.
    id: Option<usize>,
}

impl Drop for Dirty<'_> {
    fn drop(&mut self) {
        match self.id {
            Some(id) => self.set.mark(id),
            None => self.set.new_epoch(),
        }
        self.set.active.fetch_sub(1, SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An enabled set with a bitmap for `ids` installed.
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
        // A quiet hour visits nothing and keeps the epoch.
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

    /// A writer that begins once the take saw none running: its entry bit
    /// is visited when its word is swapped later, and its return bit, set
    /// after its word was swapped, survives to the next take. Either way the
    /// overlap refuses the take or the id is visited by a later one.
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
            // A bit set after its word was swapped reaches the next take.
            assert_eq!(later.contains(&writer), swapped_before);
            assert!(visited.contains(&writer) || later.contains(&writer));
        }
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
        // Without any bitmap, every notification overflows.
        let empty = DirtySet::new();
        empty.set_enabled(true);
        drop(empty.notify(0));
        assert_eq!(empty.overflows.load(Relaxed), 2);
        assert_eq!(take(&empty).0, None);
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
        // Bits marked while disabled are still visited; the epoch moved, so
        // no capture held before enabling matches.
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
        // Already large enough: nothing is allocated.
        set.offer(10);
        assert!(set.offered.load(SeqCst).is_null());
        set.offer(1_000);
        // An ordinary hour holds it and keeps every certificate.
        assert!(!set.boundary(false));
        assert_eq!(set.capacity.load(Relaxed), 64);
        assert_eq!(set.held.load(Relaxed), 1);
        assert_eq!(take(&set).0, Some(before));
        let running = set.notify(3);
        assert!(!set.boundary(true));
        assert_eq!(set.deferred.load(Relaxed), 1);
        drop(running);
        // Blocked by the running writer, it waits for the next full hour.
        assert!(!set.boundary(false));
        assert_eq!(set.capacity.load(Relaxed), 64);
        drop(set.notify(999));
        assert!(set.boundary(true));
        assert_eq!(set.capacity.load(Relaxed), 1_024);
        assert_eq!(set.installs.load(Relaxed), 2);
        // The replaced bitmap's bits are gone and the epoch moved.
        let (after, ids) = take(&set);
        assert_ne!(after, Some(before));
        assert!(ids.is_empty());
        drop(set.notify(999));
        assert_eq!(take(&set).1, vec![999]);
        // Nothing ran across the swap, so the old bitmap is retired at once.
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
