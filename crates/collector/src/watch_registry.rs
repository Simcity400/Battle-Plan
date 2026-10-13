//! A clearable registry of the receiver addresses a mutation observer
//! watches, for the observers' hooks ([`crate::relation_changes`],
//! [`crate::history_changes`], [`crate::character_changes`]).
//!
//! A capture registers each receiver it certifies and takes its
//! `(epoch, sequence)`. A hooked writer bumps the sequence of a registered
//! receiver once, on entry, and the domain-wide revision with it. Equal
//! values across hours prove that no observed writer touched it.
//!
//! The revision and the count of running writers share one word
//! ([`WatchRegistry::state`]), on its own cache line: a writer's entry is one
//! checked compare-and-swap (running +1, revision +1; one locked operation
//! when uncontended, a retry per lost race) and its return one subtract
//! (running −1), and [`WatchRegistry::quiet`] reads both at once. The same
//! swap that would fill the running field to half its width, or take the
//! revision past its last value, instead sets the word's sticky untrusted
//! bit and leaves the writer uncounted. A word carrying that bit is never
//! quiet, so the registry certifies nothing more and reports writers running
//! to detach; a boundary that reads it refuses every clear, install and
//! retirement, and one already past its quiet check cannot retire a table a
//! notification may still hold (its later check sees the bit or the count).
//! Neither field can wrap. One bump per writer
//! suffices because no certificate or quiet token is taken while a writer
//! runs. Take a writer W (entry E, then its write, then its return X, each
//! sequentially consistent) and a certificate C, which loads the sequence
//! and then requires the running count to be zero:
//! - if E's bump precedes C's sequence load, either X precedes C's count load
//!   (C includes the bump, and W's write happened before anything C's
//!   capture reads after it) or W still runs there and C is refused;
//! - if E's bump follows C's sequence load, every later certificate of that
//!   receiver loads the bumped sequence (or is refused while W runs), so it
//!   differs from C and the receiver is read again.
//!
//! Either way an equal later certificate excludes any write after the read C
//! covers. The same holds for the revision in a quiet token, which reads the
//! revision and the count in one load. A writer's return bumps nothing. A
//! marking registry still marks on return (below).
//!
//! Registrations are dropped only together with an epoch change, so no
//! certificate outlives a notification it could have missed:
//! - [`WatchRegistry::boundary`] runs on the game thread in the hourly
//!   callback, before any capture. It clears the table (on an invalidation,
//!   or, unless [`WatchRegistry::retaining`], once dead receivers fill half
//!   of it) or installs a larger table offered off-thread, each between two
//!   epoch changes, and only while no notification is running (see below
//!   for when each waits).
//! - An epoch change makes every receiver be read afresh, so a clear or an
//!   install waits for an invalidating hour (attach, load, gap or recovery:
//!   the recorder's full hours), where that happens anyway. One a running
//!   notification blocked there waits for the next invalidating hour too:
//!   that hour's epoch already revoked every certificate, and dead
//!   registrations only take room. Only the first table installs at once:
//!   nothing is certified without one.
//! - Enabling observation starts an epoch: writers ran unobserved before it.
//!
//! The hook path never allocates or frees. Tables are allocated off-thread
//! ([`WatchRegistry::offer`]); a replaced table is freed off-thread
//! ([`WatchRegistry::collect`]) once no notification can still hold it.
//! Bounded probing refuses a registration when the table is full; that
//! receiver is then read afresh every hour, and the refusal is counted.
//!
//! A [`WatchRegistry::marking`] registry also keeps one bit per slot, which a
//! notification of a registered receiver sets on entry and again on return,
//! beside its sequence. Its single consumer takes the bits once an hour
//! ([`WatchRegistry::take`]), clearing every word and visiting each marked
//! receiver, so it finds what writers touched without looking up every
//! receiver, as a [`crate::dirty_set::DirtySet`] does for dense ids: the take
//! certifies only when observation is on, no writer ran and the epoch held
//! still across it, and a bit set after its word was swapped survives to the
//! next take. A clear or an install moves the epoch, so no take spanning one
//! certifies a receiver it could have missed.

use std::{
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr, AtomicU64, AtomicUsize,
        Ordering::{Relaxed, SeqCst},
    },
};

/// Low bits of [`WatchRegistry::state`]: the writers running now.
const RUNNING_BITS: u32 = 20;
/// Running writers a word may count; the admission that would reach this
/// sets [`UNTRUSTED`] instead, so the field never carries.
const RUNNING_LIMIT: u64 = 1 << (RUNNING_BITS - 1);
/// [`WatchRegistry::active`] once the count is untrusted: never zero, and
/// small enough that the families' sums of several counts cannot overflow.
const UNTRUSTED_ACTIVE: usize = usize::MAX >> 8;
/// One running writer, and one revision, in [`WatchRegistry::state`]; the
/// revision takes the bits between the running field and [`UNTRUSTED`].
const RUNNING: u64 = 1;
const REVISION: u64 = 1 << RUNNING_BITS;
const UNTRUSTED: u64 = 1 << 63;
const LAST_REVISION: u64 = (UNTRUSTED - 1) >> RUNNING_BITS;

/// The writers running in a state word.
const fn running(state: u64) -> u64 {
    state & (REVISION - 1)
}

/// The revision in a state word.
const fn revision(state: u64) -> u64 {
    (state & !UNTRUSTED) >> RUNNING_BITS
}

/// The word every writer updates, alone on its cache line so that the
/// read-mostly fields beside it (the table, the epoch and the flags each
/// notification and certificate loads) are not invalidated by every entry
/// and return.
#[repr(align(64))]
struct Hot(AtomicU64);

/// Slots probed from a receiver's hash before its registration is refused.
const PROBES: usize = 16;
/// Table slots per expected watch: probing stays short, and receivers that
/// died since the last clear have room to accumulate.
const SLOTS_PER_WATCH: usize = 4;
const MIN_SLOTS: usize = 1024;

#[derive(Default)]
struct Slot {
    address: AtomicUsize,
    sequence: AtomicU64,
}

struct Table {
    slots: Vec<Slot>,
    /// One bit per slot in a marking registry; empty otherwise.
    marks: Vec<AtomicU64>,
}

enum Lookup<'a> {
    /// The receiver's slot index and sequence.
    Watched(usize, &'a AtomicU64),
    Registered(&'a AtomicU64),
    Unwatched,
    Full,
}

const MARK_BITS: usize = u64::BITS as usize;

impl Table {
    /// Off the hook path; `None` when the allocation fails.
    fn new(len: usize, marking: bool) -> Option<Box<Self>> {
        debug_assert!(len.is_power_of_two());
        let mut slots = engine::allocation::vec(len).ok()?;
        slots.resize_with(len, Slot::default);
        let words = if marking { len.div_ceil(MARK_BITS) } else { 0 };
        let mut marks = engine::allocation::vec(words).ok()?;
        marks.resize_with(words, AtomicU64::default);
        engine::allocation::boxed(Self { slots, marks }).ok()
    }

    fn find(&self, address: usize, register: bool) -> Lookup<'_> {
        if address == 0 {
            return Lookup::Unwatched;
        }
        let mask = self.slots.len() - 1;
        // Strided receivers spread like random ones (`table_hash`).
        let first = crate::table_hash::slot((address >> 3) as u64, self.slots.len());
        for probe in 0..PROBES.min(self.slots.len()) {
            let index = (first + probe) & mask;
            let slot = &self.slots[index];
            let mut current = slot.address.load(SeqCst);
            if current == 0 && register {
                match slot.address.compare_exchange(0, address, SeqCst, SeqCst) {
                    Ok(_) => return Lookup::Registered(&slot.sequence),
                    Err(winner) => current = winner,
                }
            }
            if current == address {
                return Lookup::Watched(index, &slot.sequence);
            }
            if current == 0 {
                return Lookup::Unwatched;
            }
        }
        if register {
            Lookup::Full
        } else {
            Lookup::Unwatched
        }
    }

    /// The word and bit of slot `index`, in a marking table.
    fn mark_of(&self, index: usize) -> Option<(&AtomicU64, u64)> {
        self.marks
            .get(index / MARK_BITS)
            .map(|word| (word, 1 << (index % MARK_BITS)))
    }

    /// Between two epoch changes, which order these stores.
    fn clear(&self) {
        for slot in &self.slots {
            slot.sequence.store(0, Relaxed);
            slot.address.store(0, Relaxed);
        }
        for word in &self.marks {
            word.store(0, Relaxed);
        }
    }
}

/// Frees a table no notification or capture can reach any more.
unsafe fn free(table: *mut Table) {
    if !table.is_null() {
        drop(unsafe { Box::from_raw(table) });
    }
}

/// One observation domain: its watches, epoch, revision and running
/// notifications.
pub struct WatchRegistry {
    /// The installed table; null until a boundary installs one.
    table: AtomicPtr<Table>,
    /// A larger table allocated off-thread, awaiting a boundary.
    offered: AtomicPtr<Table>,
    /// A replaced table that a notification begun before its replacement
    /// may still hold.
    draining: AtomicPtr<Table>,
    /// A replaced table nothing holds, awaiting [`Self::collect`].
    retired: AtomicPtr<Table>,
    /// Slots of the installed table.
    capacity: AtomicUsize,
    /// Slots of the largest table allocated, installed or offered.
    requested: AtomicUsize,
    /// Registrations since the table was last cleared or installed.
    registered: AtomicUsize,
    /// Clear once dead receivers fill half the table.
    purge: bool,
    /// Notifications mark their receiver's slot for [`Self::take`].
    marking: bool,
    epoch: AtomicU64,
    /// The revision (above [`RUNNING_BITS`]) and the writers running now.
    state: Hot,
    enabled: AtomicBool,
    poisoned: AtomicBool,
    refused: AtomicU64,
    clears: AtomicU64,
    installs: AtomicU64,
    deferred: AtomicU64,
    /// Boundaries outside an invalidation that left a wanted clear or an
    /// offered table for the next invalidating hour.
    held: AtomicU64,
    failed_offers: AtomicU64,
    /// Marked receivers the takes visited.
    taken: AtomicU64,
    /// Takes that certified nothing: a writer ran or the epoch moved.
    refused_takes: AtomicU64,
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRegistry {
    pub const fn new() -> Self {
        Self::with(true, false)
    }

    /// Cleared only on an invalidation: every clear starts an epoch, so each
    /// certified receiver is read afresh once. Offer a table large enough
    /// for the receivers instead; a full one refuses registrations.
    pub const fn retaining() -> Self {
        Self::with(false, false)
    }

    /// [`Self::new`] whose notifications also mark their receiver for
    /// [`Self::take`].
    pub const fn marking() -> Self {
        Self::with(true, true)
    }

    const fn with(purge: bool, marking: bool) -> Self {
        Self {
            table: AtomicPtr::new(ptr::null_mut()),
            offered: AtomicPtr::new(ptr::null_mut()),
            draining: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
            capacity: AtomicUsize::new(0),
            requested: AtomicUsize::new(0),
            registered: AtomicUsize::new(0),
            purge,
            marking,
            epoch: AtomicU64::new(1),
            state: Hot(AtomicU64::new(0)),
            enabled: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            refused: AtomicU64::new(0),
            clears: AtomicU64::new(0),
            installs: AtomicU64::new(0),
            deferred: AtomicU64::new(0),
            held: AtomicU64::new(0),
            failed_offers: AtomicU64::new(0),
            taken: AtomicU64::new(0),
            refused_takes: AtomicU64::new(0),
        }
    }

    fn table(&self) -> Option<&Table> {
        // SAFETY: a table is freed only after a boundary replaced it and saw
        // no notification running; captures run only after the boundary.
        unsafe { self.table.load(SeqCst).as_ref() }
    }

    fn increment(&self, value: &AtomicU64) {
        if value.fetch_add(1, SeqCst) == u64::MAX {
            self.poison();
        }
    }

    fn new_epoch(&self) {
        self.increment(&self.epoch);
    }

    /// A writer begins (one running more and, for a writer, one revision);
    /// whether it was counted. In the same swap, an admission that would
    /// fill the running field to [`RUNNING_LIMIT`] or pass [`LAST_REVISION`]
    /// sets [`UNTRUSTED`] instead and counts nothing; an untrusted word
    /// counts nothing more.
    fn admit(&self, writer: bool) -> bool {
        let mut current = self.state.0.load(Relaxed);
        loop {
            if current & UNTRUSTED != 0 {
                return false;
            }
            let full = running(current) + 1 >= RUNNING_LIMIT
                || writer && revision(current) == LAST_REVISION;
            let next = if full {
                current | UNTRUSTED
            } else {
                current + RUNNING + if writer { REVISION } else { 0 }
            };
            match self
                .state
                .0
                .compare_exchange_weak(current, next, SeqCst, Relaxed)
            {
                Ok(_) if full => {
                    self.poison();
                    return false;
                }
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn enter(&self) -> bool {
        self.admit(true)
    }

    /// A writer, or a [`Running`] guard, ends; only a counted one is
    /// subtracted.
    fn leave(&self, counted: bool) {
        if counted {
            self.state.0.fetch_sub(RUNNING, SeqCst);
        }
    }

    fn quiet_now(&self) -> bool {
        self.quiet_state().is_some()
    }

    /// The state word when no writer runs and the count is trusted.
    fn quiet_state(&self) -> Option<u64> {
        let state = self.state.0.load(SeqCst);
        (running(state) == 0 && state & UNTRUSTED == 0).then_some(state)
    }

    fn observing(&self) -> bool {
        self.enabled.load(SeqCst) && !self.poisoned.load(SeqCst)
    }

    /// Hook path, allocation-free: a writer is about to change `receiver`.
    /// Dropping the guard notifies its return.
    pub fn notify(&self, receiver: usize) -> Notification<'_> {
        let counted = self.enter();
        let (sequence, mark) = match self.table() {
            Some(table) => match table.find(receiver, false) {
                Lookup::Watched(index, sequence) => (Some(sequence), table.mark_of(index)),
                _ => (None, None),
            },
            None => (None, None),
        };
        if let Some(sequence) = sequence {
            self.increment(sequence);
        }
        if let Some((word, bit)) = mark {
            word.fetch_or(bit, SeqCst);
        }
        Notification {
            registry: self,
            mark,
            every: false,
            counted,
        }
    }

    /// Hook path, allocation-free: a writer whose receiver cannot be named
    /// is about to run. It starts an epoch on entry and on return, so no
    /// certificate taken before it, or while it runs, survives it.
    pub fn notify_every(&self) -> Notification<'_> {
        let counted = self.enter();
        self.new_epoch();
        Notification {
            registry: self,
            mark: None,
            every: true,
            counted,
        }
    }

    /// Hook path, allocation-free: a writer that often leaves every receiver
    /// unchanged is running. While the guard lives no certificate is taken
    /// and no hour is quiet, as for a notification, but dropping it revokes
    /// nothing; the writer notifies a receiver it did change before the
    /// guard drops.
    pub fn running(&self) -> Running<'_> {
        Running {
            registry: self,
            counted: self.admit(false),
        }
    }

    /// The single consumer of a marking registry, after the hour's boundary:
    /// clears every marked slot, visiting its receiver, and returns the epoch
    /// they were marked in. A receiver no take visited since its
    /// registration in that epoch was touched by no observed writer. `None`
    /// when the registry does not mark, observation is off, no table is
    /// installed, a writer runs (its bits stay set) or the epoch moved during
    /// the take; nothing taken then certifies anything.
    pub fn take(&self, mut visit: impl FnMut(usize)) -> Option<u64> {
        if !self.marking || !self.observing() {
            return None;
        }
        if !self.quiet_now() {
            self.refused_takes.fetch_add(1, Relaxed);
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let table = self.table()?;
        let mut taken = 0;
        for (index, word) in table.marks.iter().enumerate() {
            if word.load(Relaxed) == 0 {
                continue;
            }
            let mut value = word.swap(0, SeqCst);
            while value != 0 {
                let slot = index * MARK_BITS + value.trailing_zeros() as usize;
                value &= value - 1;
                let address = table.slots[slot].address.load(SeqCst);
                if address != 0 {
                    visit(address);
                    taken += 1;
                }
            }
        }
        self.taken.fetch_add(taken, Relaxed);
        if self.epoch.load(SeqCst) == epoch && self.quiet_now() {
            Some(epoch)
        } else {
            self.refused_takes.fetch_add(1, Relaxed);
            None
        }
    }

    /// Marked receivers the takes visited, and takes that certified nothing.
    pub fn takes(&self) -> (u64, u64) {
        (self.taken.load(Relaxed), self.refused_takes.load(Relaxed))
    }

    /// `(epoch, revision)` while observing with no writer running; equal
    /// values across hours prove that no observed writer ran.
    pub fn quiet(&self) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let state = self.quiet_state()?;
        Some((epoch, revision(state)))
    }

    /// A cache hint for `receiver`'s first slot, ahead of its
    /// [`Self::sequence`]: reads and changes nothing.
    pub fn prefetch(&self, receiver: usize) {
        if receiver == 0 {
            return;
        }
        if let Some(table) = self.table() {
            let index = crate::table_hash::slot((receiver >> 3) as u64, table.slots.len());
            if let Some(slot) = table.slots.get(index) {
                engine::prefetch::span(
                    std::ptr::from_ref(slot) as usize,
                    std::mem::size_of::<Slot>(),
                );
            }
        }
    }

    /// `(epoch, sequence)` of `receiver`, registering it; `None` when it
    /// cannot be observed. Captures only, after the hour's boundary.
    pub fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.register(receiver).map(|(certificate, _)| certificate)
    }

    /// [`Self::sequence`], and whether this call registered `receiver`: a
    /// writer that ran before it was not observed, even one that ran after
    /// the capture's earlier reads.
    pub fn register(&self, receiver: usize) -> Option<((u64, u64), bool)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let (sequence, fresh) = match self.table()?.find(receiver, true) {
            Lookup::Watched(_, sequence) => (sequence, false),
            Lookup::Registered(sequence) => {
                self.registered.fetch_add(1, Relaxed);
                (sequence, true)
            }
            Lookup::Unwatched => return None,
            Lookup::Full => {
                self.refused.fetch_add(1, Relaxed);
                return None;
            }
        };
        let sequence = sequence.load(SeqCst);
        (self.epoch.load(SeqCst) == epoch && self.quiet_now()).then_some(((epoch, sequence), fresh))
    }

    /// Game thread, in the hourly callback before any capture: start an
    /// epoch when `invalidate`, then install an offered table or clear the
    /// installed one while no notification is running. Outside an
    /// invalidation both wait, counted as held, unless no table is installed
    /// yet; so does one a running notification blocks. Returns whether it
    /// installed a table.
    pub fn boundary(&self, invalidate: bool) -> bool {
        if invalidate {
            self.new_epoch();
        }
        let full = self.purge && self.registered.load(Relaxed) * 2 > self.capacity.load(Relaxed);
        let first = self.table.load(SeqCst).is_null();
        let restart = invalidate || first;
        if !restart {
            if full || !self.offered.load(SeqCst).is_null() {
                self.held.fetch_add(1, Relaxed);
            }
            if self.quiet_now() {
                self.retire_drained();
            }
            return false;
        }
        if !self.quiet_now() {
            if full
                || invalidate
                || !self.offered.load(SeqCst).is_null()
                || !self.draining.load(SeqCst).is_null()
            {
                self.deferred.fetch_add(1, Relaxed);
            }
            // Only a missing first table retries at the next hour; the rest
            // waits for the next invalidating hour.
            return false;
        }
        // No notification is running, so none holds a replaced table.
        self.retire_drained();
        if self.draining.load(SeqCst).is_null() && self.retired.load(SeqCst).is_null() {
            let next = self.offered.swap(ptr::null_mut(), SeqCst);
            // SAFETY: an offered table is owned here once swapped out.
            if let Some(len) = unsafe { next.as_ref() }.map(|table| table.slots.len()) {
                self.new_epoch();
                let previous = self.table.swap(next, SeqCst);
                self.new_epoch();
                self.capacity.store(len, Relaxed);
                self.registered.store(0, Relaxed);
                self.installs.fetch_add(1, Relaxed);
                // A notification begun since the check above may hold it.
                self.draining.store(previous, SeqCst);
                if self.quiet_now() {
                    self.retire_drained();
                }
                return true;
            }
        }
        if full || invalidate {
            // A notification beginning during the clear either bumped its
            // receiver before it (a certificate taken while it runs is
            // refused) or finds it unregistered; the epoch changes both ways.
            if let Some(table) = self.table() {
                self.new_epoch();
                table.clear();
                self.new_epoch();
                self.registered.store(0, Relaxed);
                self.clears.fetch_add(1, Relaxed);
            }
        }
        false
    }

    /// Hand a drained table to [`Self::collect`] once the previous one is
    /// gone. Only while no notification is running.
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

    /// Off the game thread: allocate a table for `watches` receivers when
    /// the installed and offered ones are smaller. A boundary installs it.
    pub fn offer(&self, watches: usize) {
        let Some(len) = watches
            .checked_mul(SLOTS_PER_WATCH)
            .and_then(|slots| slots.max(MIN_SLOTS).checked_next_power_of_two())
        else {
            let _ = engine::allocation::oom();
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        self.offer_slots(len);
    }

    /// An enabled registry with a `len`-slot table installed (`len` a power
    /// of two), for tests.
    #[cfg(test)]
    pub(crate) fn enabled(len: usize) -> Self {
        let registry = Self::new();
        registry.offer_slots(len);
        registry.boundary(false);
        registry.set_enabled(true);
        registry
    }

    fn offer_slots(&self, len: usize) {
        if len <= self.requested.load(SeqCst) {
            return;
        }
        let Some(table) = Table::new(len, self.marking) else {
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        self.requested.store(len, SeqCst);
        let previous = self.offered.swap(Box::into_raw(table), SeqCst);
        // SAFETY: an offered table was never installed; the swap owns it.
        unsafe { free(previous) };
    }

    /// Off the game thread: free a replaced table nothing holds any more.
    pub fn collect(&self) {
        let retired = self.retired.swap(ptr::null_mut(), SeqCst);
        // SAFETY: only a drained table is retired, and the swap owns it.
        unsafe { free(retired) };
    }

    /// Free every table.
    ///
    /// # Safety
    /// No notification or capture may run: the hooks are removed and
    /// drained, and the acquisition pool has stopped.
    pub unsafe fn release(&self) {
        for table in [&self.table, &self.offered, &self.draining, &self.retired] {
            unsafe { free(table.swap(ptr::null_mut(), SeqCst)) };
        }
        self.capacity.store(0, Relaxed);
        self.requested.store(0, SeqCst);
        self.registered.store(0, Relaxed);
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

    /// Registrations since the table was last cleared or installed.
    pub fn registered(&self) -> usize {
        self.registered.load(Relaxed)
    }

    /// Notifications running now.
    pub fn active(&self) -> usize {
        let state = self.state.0.load(SeqCst);
        if state & UNTRUSTED != 0 {
            UNTRUSTED_ACTIVE
        } else {
            running(state) as usize
        }
    }

    pub fn status(&self) -> impl serde::Serialize {
        crate::status::status!({
            "capacity": self.capacity.load(Relaxed),
            "registered": self.registered.load(Relaxed),
            "refused_registrations": self.refused.load(Relaxed),
            "clears": self.clears.load(Relaxed),
            "installs": self.installs.load(Relaxed),
            "deferred_boundaries": self.deferred.load(Relaxed),
            "held_for_full_hour": self.held.load(Relaxed),
            "failed_offers": self.failed_offers.load(Relaxed),
            "scope": "cumulative since attach; a refused receiver is read afresh every hour; a clear or install starts an epoch, so every receiver is read afresh once, and waits for an invalidating hour (attach, load, gap or recovery)"
        })
    }
}

impl Drop for WatchRegistry {
    fn drop(&mut self) {
        // SAFETY: exclusive access; nothing else can reach the tables.
        unsafe { self.release() };
    }
}

/// A running writer; its drop notifies the writer's return.
pub struct Notification<'a> {
    registry: &'a WatchRegistry,
    /// The receiver's mark in a marking registry, set again on return.
    mark: Option<(&'a AtomicU64, u64)>,
    /// The receiver is unknown: every certificate is revoked.
    every: bool,
    /// The entry was counted as running (the word was trusted).
    counted: bool,
}

/// A writer running without a notification ([`WatchRegistry::running`]).
pub struct Running<'a> {
    registry: &'a WatchRegistry,
    counted: bool,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.registry.leave(self.counted);
    }
}

impl Drop for Notification<'_> {
    fn drop(&mut self) {
        // The entry bumped the sequence and the revision (module docs).
        if let Some((word, bit)) = self.mark {
            word.fetch_or(bit, SeqCst);
        }
        if self.every {
            self.registry.new_epoch();
        }
        self.registry.leave(self.counted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An enabled registry with a `len`-slot table installed.
    fn registry(len: usize) -> WatchRegistry {
        let registry = WatchRegistry::enabled(len);
        assert_eq!(registry.capacity.load(Relaxed), len);
        registry
    }

    /// A running writer certifies nothing while it runs and, unless it
    /// notifies, revokes nothing when it ends.
    #[test]
    fn a_running_writer_blocks_certificates_without_revoking_them() {
        let registry = registry(64);
        let certificate = registry.sequence(0x1000).unwrap();
        let quiet = registry.quiet().unwrap();
        let running = registry.running();
        assert!(registry.sequence(0x1000).is_none() && registry.quiet().is_none());
        drop(running);
        assert_eq!(registry.sequence(0x1000), Some(certificate));
        assert_eq!(registry.quiet(), Some(quiet));
        let running = registry.running();
        drop(registry.notify(0x1000));
        drop(running);
        assert_ne!(registry.sequence(0x1000), Some(certificate));
    }

    #[test]
    fn watches_are_exact_and_a_full_table_refuses_and_counts() {
        let registry = registry(4);
        assert!(registry.sequence(0).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(registry.sequence(address).map(|(_, s)| s), Some(0));
        }
        assert!(registry.sequence(40).is_none());
        assert_eq!(registry.refused.load(Relaxed), 1);
        drop(registry.notify(8));
        assert_eq!(registry.sequence(8).map(|(_, s)| s), Some(1));
        assert_eq!(registry.sequence(16).map(|(_, s)| s), Some(0));
        // An unregistered receiver moves only the revision.
        let quiet = registry.quiet().unwrap();
        drop(registry.notify(40));
        assert_ne!(registry.quiet(), Some(quiet));
        assert_eq!(registry.sequence(24).map(|(_, s)| s), Some(0));
    }

    /// Receivers allocated one after another, at any object size, all
    /// register in the table `offer` sizes for them: 155,000 (the
    /// characters of 5,000 countries) at each stride. The bits 32 up of a
    /// multiplicative hash refused 73,654 of them at a 0xF8 stride, 102,428
    /// at 0x1F0, 127,608 at 0xF80 and 46 at 0x100; its top bits 14,825 at
    /// 0x370, 41,234 at 0x6E0, 54,441 at 0xDC0 and 58,114 at 0x1310.
    #[test]
    fn strided_receivers_at_5000_countries_all_register() {
        const RECEIVERS: usize = 5_000 * 31;
        for stride in [
            0x48, 0xf8, 0x100, 0x1f0, 0x200, 0x370, 0x6e0, 0xdc0, 0xf80, 0x1310, 0x2000,
        ] {
            let registry = WatchRegistry::new();
            registry.offer(RECEIVERS);
            registry.boundary(false);
            registry.set_enabled(true);
            for index in 0..RECEIVERS {
                assert!(
                    registry.sequence(0x4_0000_0000 + index * stride).is_some(),
                    "stride {stride:#x}, receiver {index}"
                );
            }
            assert_eq!(registry.refused.load(Relaxed), 0, "stride {stride:#x}");
            assert_eq!(registry.registered(), RECEIVERS);
        }
    }

    #[test]
    fn a_clear_starts_an_epoch_so_no_certificate_survives_it() {
        let registry = registry(64);
        let certified = registry.sequence(8).unwrap();
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_eq!(registry.registered.load(Relaxed), 0);
        // Unregistered now: this writer's notification is not recorded,
        // but the epoch moved, so the old certificate cannot match.
        drop(registry.notify(8));
        let fresh = registry.sequence(8).unwrap();
        assert_eq!(fresh.1, 0);
        assert_ne!(fresh, certified);
    }

    #[test]
    fn a_running_notification_defers_the_clear_but_not_the_epoch() {
        let registry = registry(64);
        let certified = registry.sequence(8).unwrap();
        let running = registry.notify(8);
        registry.boundary(true);
        assert_eq!(registry.deferred.load(Relaxed), 1);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert!(registry.sequence(8).is_none());
        assert!(registry.quiet().is_none());
        drop(running);
        assert_ne!(registry.sequence(8), Some(certified));
        // The next ordinary hour keeps the new certificates: the clear waits
        // for the next invalidating hour, whose epoch revokes them anyway.
        let fresh = registry.sequence(8).unwrap();
        registry.boundary(false);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert_eq!(registry.sequence(8), Some(fresh));
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_eq!(registry.registered.load(Relaxed), 0);
    }

    #[test]
    fn dead_receivers_filling_half_the_table_clear_it_at_the_next_full_hour() {
        let registry = registry(64);
        for address in (1..=33).map(|n| n * 8) {
            registry.sequence(address).unwrap();
        }
        // An ordinary hour keeps every certificate: the clear is held.
        let certified = registry.sequence(8).unwrap();
        registry.boundary(false);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert_eq!(registry.held.load(Relaxed), 1);
        assert_eq!(registry.sequence(8), Some(certified));
        let before = registry.epoch.load(SeqCst);
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert!(registry.epoch.load(SeqCst) > before);
    }

    #[test]
    fn a_retaining_registry_clears_only_on_an_invalidation() {
        let registry = WatchRegistry::retaining();
        registry.offer_slots(64);
        registry.boundary(false);
        registry.set_enabled(true);
        for address in (1..=40).map(|n| n * 8) {
            registry.sequence(address).unwrap();
        }
        let certified = registry.sequence(8).unwrap();
        registry.boundary(false);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert_eq!(registry.sequence(8), Some(certified));
        registry.boundary(true);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_ne!(registry.sequence(8), Some(certified));
    }

    /// A writer that began after the boundary saw no notification running
    /// can overlap the clear. Whether its bump lands before or after the
    /// clear zeroes its slot, a receiver registered there is refused while
    /// it runs and certified after it ends with that bump included; its
    /// return bumps nothing that could read as a later advance.
    #[test]
    fn a_notification_spanning_a_clear_never_advances_a_new_certificate() {
        for bumped_after_clear in [false, true] {
            let registry = registry(64);
            registry.sequence(8).unwrap();
            let table = registry.table().unwrap();
            let Lookup::Watched(_, slot) = table.find(8, false) else {
                unreachable!("registered above")
            };
            // Found its slot before the clear reached it.
            let running = (!bumped_after_clear).then(|| registry.notify(8));
            registry.new_epoch();
            table.clear();
            registry.new_epoch();
            let running = running.unwrap_or_else(|| {
                let counted = registry.enter();
                slot.fetch_add(1, SeqCst);
                Notification {
                    registry: &registry,
                    mark: None,
                    every: false,
                    counted,
                }
            });
            assert!(registry.sequence(8).is_none());
            drop(running);
            let (_, baseline) = registry.sequence(8).unwrap();
            assert_eq!(registry.sequence(8).map(|(_, s)| s), Some(baseline));
            drop(registry.notify(8));
            assert_eq!(registry.sequence(8).map(|(_, s)| s), Some(baseline + 1));
        }
    }

    #[test]
    fn a_larger_table_is_installed_at_a_quiet_full_hour_and_freed_off_thread() {
        let registry = registry(4);
        let certified = registry.sequence(8).unwrap();
        // Already large enough: nothing is allocated.
        registry.offer_slots(4);
        assert!(registry.offered.load(SeqCst).is_null());
        registry.offer(1_000);
        // An ordinary hour holds it and keeps every certificate.
        assert!(!registry.boundary(false));
        assert_eq!(registry.capacity.load(Relaxed), 4);
        assert_eq!(registry.held.load(Relaxed), 1);
        assert_eq!(registry.sequence(8), Some(certified));
        let running = registry.notify(8);
        registry.boundary(true);
        assert_eq!(registry.capacity.load(Relaxed), 4);
        drop(running);
        // Blocked by the running writer, it waits for the next full hour.
        assert!(!registry.boundary(false));
        assert_eq!(registry.capacity.load(Relaxed), 4);
        assert!(registry.boundary(true));
        assert_eq!(registry.capacity.load(Relaxed), 4096);
        assert_eq!(registry.installs.load(Relaxed), 2);
        assert_ne!(registry.sequence(8), Some(certified));
        // Nothing ran across the swap, so the old table is retired at once.
        assert!(registry.draining.load(SeqCst).is_null());
        assert!(!registry.retired.load(SeqCst).is_null());
        registry.collect();
        assert!(registry.retired.load(SeqCst).is_null());
    }

    #[test]
    fn an_unrepresentable_offer_reports_failure_and_keeps_the_installed_table() {
        let registry = registry(4);
        let installed = registry.table.load(SeqCst);
        let certificate = registry.sequence(8).unwrap();
        registry.offer(usize::MAX);
        assert_eq!(registry.failed_offers.load(Relaxed), 1);
        assert_eq!(registry.table.load(SeqCst), installed);
        assert!(registry.offered.load(SeqCst).is_null());
        assert_eq!(registry.sequence(8), Some(certificate));
    }

    #[test]
    fn an_unnamed_receiver_revokes_every_certificate() {
        let registry = registry(64);
        let certified = [8, 16].map(|address| registry.sequence(address).unwrap());
        let running = registry.notify_every();
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
        drop(running);
        for (address, certificate) in [8, 16].into_iter().zip(certified) {
            assert_ne!(registry.sequence(address), Some(certificate));
        }
    }

    /// A writer is one bump of its receiver and of the revision, at its
    /// entry; certificates and quiet tokens are refused until it returns,
    /// and its return bumps nothing. The running count and the revision
    /// share a word on its own cache line.
    #[test]
    fn a_writer_bumps_once_at_entry_and_is_refused_until_it_returns() {
        let registry = registry(64);
        let (epoch, sequence) = registry.sequence(8).unwrap();
        let (_, initial_revision) = registry.quiet().unwrap();
        let writer = registry.notify(8);
        assert_eq!(registry.active(), 1);
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
        let nested = registry.notify(16);
        assert_eq!(registry.active(), 2);
        drop(nested);
        drop(writer);
        assert_eq!(registry.active(), 0);
        assert_eq!(registry.sequence(8), Some((epoch, sequence + 1)));
        assert_eq!(registry.quiet(), Some((epoch, initial_revision + 2)));
        // A running guard moves neither.
        drop(registry.running());
        assert_eq!(registry.quiet(), Some((epoch, initial_revision + 2)));
        assert_eq!(std::mem::align_of::<Hot>(), 64);
        assert_eq!(std::mem::size_of::<Hot>(), 64);
        // The last representable revision makes the word untrusted rather
        // than wrapping into a value an older token holds.
        registry
            .state
            .0
            .store(LAST_REVISION << RUNNING_BITS, SeqCst);
        drop(registry.notify(8));
        assert!(registry.poisoned());
        assert_eq!(registry.state.0.load(SeqCst) & UNTRUSTED, UNTRUSTED);
        assert_eq!(revision(registry.state.0.load(SeqCst)), LAST_REVISION);
        assert!(registry.quiet().is_none());
    }

    /// The admission that would fill the running field to its limit sets
    /// the word's untrusted bit in the same swap and counts nothing, so the
    /// field never carries: no interleaving of admissions can bring the
    /// count back to zero while writers it counted still run. From then on
    /// the word is never quiet, even once every counted writer returned:
    /// nothing is certified, no table (installed, offered or draining) is
    /// cleared, installed or retired, and detach sees writers running.
    #[test]
    fn the_admission_at_the_running_limit_makes_the_word_untrusted() {
        for guard in [false, true] {
            let registry = registry(64);
            registry.sequence(8).unwrap();
            // A replaced table a stale notification may still hold.
            let draining = Box::into_raw(Table::new(64, false).unwrap());
            registry.draining.store(draining, SeqCst);
            registry
                .state
                .0
                .store((7 << RUNNING_BITS) | (RUNNING_LIMIT - 2), SeqCst);
            let hold = || {
                (
                    guard.then(|| registry.running()),
                    (!guard).then(|| registry.notify(8)),
                )
            };
            let counted = hold();
            let state = registry.state.0.load(SeqCst);
            assert_eq!((state & UNTRUSTED, running(state)), (0, RUNNING_LIMIT - 1));
            let refused = hold();
            let state = registry.state.0.load(SeqCst);
            assert_eq!(state & UNTRUSTED, UNTRUSTED);
            assert_eq!(running(state), RUNNING_LIMIT - 1, "not counted");
            assert!(registry.poisoned());
            // Uncounted admissions and returns leave the count alone.
            drop(hold());
            drop(refused);
            assert_eq!(running(registry.state.0.load(SeqCst)), RUNNING_LIMIT - 1);
            drop(counted);
            assert_eq!(running(registry.state.0.load(SeqCst)), RUNNING_LIMIT - 2);
            // Even a count that reads zero is never quiet again.
            registry
                .state
                .0
                .store(UNTRUSTED | (9 << RUNNING_BITS), SeqCst);
            assert_eq!(registry.active(), UNTRUSTED_ACTIVE);
            assert!(registry.quiet().is_none() && registry.sequence(8).is_none());
            let installed = registry.table.load(SeqCst);
            registry.offer(1_000);
            for invalidate in [false, true] {
                assert!(!registry.boundary(invalidate));
            }
            assert_eq!(registry.table.load(SeqCst), installed, "no install");
            assert_eq!(registry.draining.load(SeqCst), draining, "not retired");
            assert!(registry.retired.load(SeqCst).is_null());
        }
    }

    /// Release bench: one notification of a registered receiver (entry and
    /// return), on one thread and on four at once.
    #[test]
    #[ignore = "release per-call notification cost; run with --release --ignored --nocapture --test-threads=1"]
    fn hook_cost_watch_registry() {
        const RECEIVERS: usize = 1500;
        let registry = registry(8192);
        let receiver = |index: usize| 0x10_0000 + index * 0x200;
        for index in 0..RECEIVERS {
            registry.sequence(receiver(index)).unwrap();
        }
        let one = crate::hook_timing::per_call_ns(RECEIVERS, || {
            for index in 0..RECEIVERS {
                drop(std::hint::black_box(registry.notify(receiver(index))));
            }
        });
        let four = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        crate::hook_timing::per_call_ns(RECEIVERS, || {
                            for index in 0..RECEIVERS {
                                drop(std::hint::black_box(registry.notify(receiver(index))));
                            }
                        })
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .sum::<f64>()
                / 4.0
        });
        println!(
            "hook_cost watch_registry notify+return: {one:.1} ns/call on one thread, {four:.1} ns/call on each of four"
        );
    }

    /// An enabled marking registry with a `len`-slot table installed.
    fn marking(len: usize) -> WatchRegistry {
        let registry = WatchRegistry::marking();
        registry.offer_slots(len);
        registry.boundary(false);
        registry.set_enabled(true);
        registry
    }

    fn take(registry: &WatchRegistry) -> (Option<u64>, Vec<usize>) {
        let mut seen = Vec::new();
        let epoch = registry.take(|address| seen.push(address));
        seen.sort_unstable();
        (epoch, seen)
    }

    #[test]
    fn a_marking_registry_takes_each_touched_receiver_once_within_one_epoch() {
        let registry = marking(64);
        for address in [8, 16, 24] {
            registry.sequence(address).unwrap();
        }
        let (Some(epoch), seen) = take(&registry) else {
            unreachable!("observing")
        };
        assert!(seen.is_empty());
        // A registration says whether it was this call's.
        assert_eq!(registry.register(48).map(|(_, fresh)| fresh), Some(true));
        assert_eq!(registry.register(48).map(|(_, fresh)| fresh), Some(false));
        drop(registry.notify(16));
        drop(registry.notify(16));
        // An unregistered receiver has no slot to mark.
        drop(registry.notify(40));
        assert_eq!(take(&registry), (Some(epoch), vec![16]));
        // A quiet hour visits nothing and keeps the epoch.
        assert_eq!(take(&registry), (Some(epoch), vec![]));
        assert_eq!(registry.takes(), (1, 0));
        // A running writer refuses the take and keeps its mark.
        let running = registry.notify(24);
        assert_eq!(take(&registry), (None, vec![]));
        drop(running);
        assert_eq!(take(&registry), (Some(epoch), vec![24]));
        // A writer that begins during the take, after its word was swapped,
        // refuses it and marks again on return for the next take.
        drop(registry.notify(8));
        let mut running = None;
        let certified = registry.take(|address| {
            if address == 8 {
                running = Some(registry.notify(8));
            }
        });
        assert_eq!(certified, None);
        drop(running);
        assert_eq!(take(&registry), (Some(epoch), vec![8]));
        // An invalidation moves the epoch.
        registry.boundary(true);
        assert_ne!(take(&registry).0, Some(epoch));
    }

    #[test]
    fn only_a_marking_registry_takes_and_a_poisoned_one_refuses() {
        let plain = registry(64);
        plain.sequence(8).unwrap();
        drop(plain.notify(8));
        assert_eq!(take(&plain), (None, vec![]));
        let marking = marking(64);
        marking.sequence(8).unwrap();
        marking.poison();
        drop(marking.notify(8));
        assert_eq!(take(&marking), (None, vec![]));
    }

    #[test]
    fn observation_is_off_until_enabled_and_enabling_starts_an_epoch() {
        let registry = WatchRegistry::new();
        registry.offer(1);
        registry.boundary(false);
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
        let before = registry.epoch.load(SeqCst);
        registry.set_enabled(true);
        assert!(registry.epoch.load(SeqCst) > before);
        assert!(registry.sequence(8).is_some());
        registry.poison();
        assert!(registry.sequence(8).is_none() && registry.quiet().is_none());
    }
}
