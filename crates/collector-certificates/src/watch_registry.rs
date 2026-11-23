
use collector_primitives::asymmetric;
use std::{
    cell::Cell,
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr, AtomicU64, AtomicUsize,
        Ordering::{Acquire, Relaxed, Release, SeqCst},
    },
};

thread_local! {
    static QUIESCENT_GROWTH: Cell<bool> = const { Cell::new(false) };
}

struct QuiescentGrowth(bool);
impl Drop for QuiescentGrowth {
    fn drop(&mut self) {
        QUIESCENT_GROWTH.set(self.0);
    }
}

/// # Safety
/// The caller holds the native world's quiescent acquisition lease and
/// exclusively owns capture and registration for every registry bounded inside
/// `f`. No certificate query, concurrent boundary, or release may run during
/// `f`.
pub unsafe fn with_quiescent_growth<R>(f: impl FnOnce() -> R) -> R {
    let _guard = QuiescentGrowth(QUIESCENT_GROWTH.replace(true));
    f()
}

const RUNNING_BITS: u32 = 20;
const RUNNING_LIMIT: u64 = 1 << (RUNNING_BITS - 1);
const UNTRUSTED_ACTIVE: usize = usize::MAX >> 8;
const RUNNING: u64 = 1;
const REVISION: u64 = 1 << RUNNING_BITS;
const UNTRUSTED: u64 = 1 << 63;
const LAST_REVISION: u64 = (UNTRUSTED - 1) >> RUNNING_BITS;

const fn running(state: u64) -> u64 {
    state & (REVISION - 1)
}

const fn revision(state: u64) -> u64 {
    (state & !UNTRUSTED) >> RUNNING_BITS
}

#[repr(align(64))]
struct Hot(AtomicU64);

const PROBES: usize = 32;
static REFUSED_TOTAL: AtomicU64 = AtomicU64::new(0);
static PEAK_LOAD: AtomicU64 = AtomicU64::new(0);
const SLOTS_PER_WATCH: usize = 2;
const MIN_SLOTS: usize = 1024;

#[derive(Default)]
struct Slot {
    address: AtomicUsize,
    sequence: AtomicU64,
}

struct Table {
    slots: Vec<Slot>,
    marks: Vec<AtomicU64>,
}

enum Lookup<'a> {
    Watched(usize, &'a AtomicU64),
    Registered(usize, &'a AtomicU64),
    Unwatched,
    Full,
}

const MARK_BITS: usize = u64::BITS as usize;

#[inline]
fn mark_slot(word: &AtomicU64, bit: u64) {
    if word.load(SeqCst) & bit == 0 {
        word.fetch_or(bit, SeqCst);
    }
}

impl Table {
    fn new(len: usize, marking: bool) -> Option<Box<Self>> {
        debug_assert!(len.is_power_of_two());
        let mut slots = engine_allocation::vec(len).ok()?;
        slots.resize_with(len, Slot::default);
        let words = if marking { len.div_ceil(MARK_BITS) } else { 0 };
        let mut marks = engine_allocation::vec(words).ok()?;
        marks.resize_with(words, AtomicU64::default);
        engine_allocation::boxed(Self { slots, marks }).ok()
    }

    #[inline]
    fn find(&self, address: usize, register: bool) -> Lookup<'_> {
        if address == 0 {
            return Lookup::Unwatched;
        }
        let mask = self.slots.len() - 1;
        let first = crate::table_hash::slot(address as u64, self.slots.len());
        for probe in 0..PROBES.min(self.slots.len()) {
            let index = (first + probe) & mask;
            // SAFETY: `self.slots.len()` is a non-zero power of two and `mask == len - 1`, so `index < len`.
            let slot = unsafe { self.slots.get_unchecked(index) };
            let mut current = slot.address.load(SeqCst);
            if current == 0 && register {
                match slot.address.compare_exchange(0, address, SeqCst, SeqCst) {
                    Ok(_) => return Lookup::Registered(index, &slot.sequence),
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

    #[inline]
    fn mark_of(&self, index: usize) -> Option<(&AtomicU64, u64)> {
        self.marks
            .get(index / MARK_BITS)
            .map(|word| (word, 1 << (index % MARK_BITS)))
    }

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

/// Frees a table.
///
/// # Safety
/// `table` is null or came from `Box::into_raw`, and no notification or capture can reach it any
/// more.
unsafe fn free(table: *mut Table) {
    if !table.is_null() {
        drop(unsafe { Box::from_raw(table) });
    }
}

#[derive(Clone, Copy)]
enum Admission {
    None,
    Slot(u8, u8),
    Slow,
}

impl Admission {
    #[inline(always)]
    fn active(self) -> bool {
        !matches!(self, Self::None)
    }
}

pub struct WatchRegistry {
    table: AtomicPtr<Table>,
    offered: AtomicPtr<Table>,
    offer_lock: std::sync::Mutex<()>,
    draining: AtomicPtr<Table>,
    retired: AtomicPtr<Table>,
    capacity: AtomicUsize,
    requested: AtomicUsize,
    registered: AtomicUsize,
    purge: bool,
    marking: bool,
    sequences: bool,
    synchronous: bool,
    epoch: AtomicU64,
    state: Hot,
    dirty: AtomicBool,
    revision: AtomicU64,
    flushing: AtomicBool,
    enabled: AtomicBool,
    poisoned: AtomicBool,
    refused: AtomicU64,
    refused_at_offer: AtomicU64,
    clears: AtomicU64,
    installs: AtomicU64,
    deferred: AtomicU64,
    held: AtomicU64,
    failed_offers: AtomicU64,
    taken: AtomicU64,
    refused_takes: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary {
    Unchanged,
    Deferred,
    Cleared,
    Installed,
}
impl Boundary {
    pub fn installed(self) -> bool {
        self == Self::Installed
    }
    pub fn renewed(self) -> bool {
        matches!(self, Self::Cleared | Self::Installed)
    }
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRegistry {
    pub const fn new() -> Self {
        Self::with(true, false, true)
    }

    pub const fn retaining() -> Self {
        Self::with(false, false, true)
    }

    pub const fn marking() -> Self {
        Self::with(true, true, true)
    }

    pub const fn marks_only() -> Self {
        Self::with(true, true, false)
    }

    pub const fn with_synchronous(mut self) -> Self {
        self.synchronous = true;
        self
    }

    const fn with(purge: bool, marking: bool, sequences: bool) -> Self {
        Self {
            table: AtomicPtr::new(ptr::null_mut()),
            offered: AtomicPtr::new(ptr::null_mut()),
            offer_lock: std::sync::Mutex::new(()),
            draining: AtomicPtr::new(ptr::null_mut()),
            retired: AtomicPtr::new(ptr::null_mut()),
            capacity: AtomicUsize::new(0),
            requested: AtomicUsize::new(0),
            registered: AtomicUsize::new(0),
            purge,
            marking,
            sequences,
            synchronous: false,
            epoch: AtomicU64::new(1),
            state: Hot(AtomicU64::new(0)),
            dirty: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            flushing: AtomicBool::new(false),
            enabled: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            refused: AtomicU64::new(0),
            refused_at_offer: AtomicU64::new(0),
            clears: AtomicU64::new(0),
            installs: AtomicU64::new(0),
            deferred: AtomicU64::new(0),
            held: AtomicU64::new(0),
            failed_offers: AtomicU64::new(0),
            taken: AtomicU64::new(0),
            refused_takes: AtomicU64::new(0),
        }
    }

    #[inline]
    fn table(&self) -> Option<&Table> {
        // SAFETY: during capture only a retired table is freed (`collect`); teardown (`release`)
        // frees the installed table once callbacks and acquisition readers have quiesced. A
        // boundary swap parks the old table in `draining`, since a notification that entered
        // after the quiet check may still hold its mark; `retire_drained` retires it only once a
        // later quiet check sees none running.
        // Worker growth retires the old table at once, which its exclusive lease allows because
        // no notifier runs during the call. A `Notification` keeps its mark until it drops and
        // counts as running until then, so neither path reclaims a table under a live guard.
        unsafe { self.table.load(SeqCst).as_ref() }
    }

    #[inline]
    fn increment(&self, value: &AtomicU64) {
        if value.fetch_add(1, SeqCst) == u64::MAX {
            self.poison();
        }
    }

    fn new_epoch(&self) {
        self.increment(&self.epoch);
    }

    #[inline(always)]
    fn tag(&self) -> usize {
        self as *const Self as usize
    }

    #[inline(always)]
    fn admit(&self, writer: bool) -> Admission {
        if !self.synchronous
            && self.state.0.load(Relaxed) == 0
            && let Some((slot_idx, entry_idx)) = asymmetric::enter_slot(self.tag())
        {
            if writer && !self.dirty.load(Relaxed) {
                self.dirty.store(true, Relaxed);
            }
            return Admission::Slot(slot_idx, entry_idx);
        }
        if self.admit_word(writer) {
            Admission::Slow
        } else {
            Admission::None
        }
    }

    #[cold]
    #[inline(never)]
    fn admit_word(&self, writer: bool) -> bool {
        let delta = RUNNING + if writer { REVISION } else { 0 };
        let current = self.state.0.load(Relaxed);
        if current & UNTRUSTED == 0
            && running(current) + 1 < RUNNING_LIMIT / 2
            && (!writer || revision(current) < LAST_REVISION - (RUNNING_LIMIT / 2))
        {
            let before = self.state.0.fetch_add(delta, SeqCst);
            if before & UNTRUSTED == 0
                && running(before) + 1 < RUNNING_LIMIT
                && (!writer || revision(before) < LAST_REVISION)
            {
                return true;
            }
            return self.admit_overflow(delta);
        }
        self.admit_slow(writer)
    }

    #[cold]
    #[inline(never)]
    fn admit_overflow(&self, delta: u64) -> bool {
        self.state.0.fetch_or(UNTRUSTED, SeqCst);
        self.state.0.fetch_sub(delta, SeqCst);
        self.poison();
        false
    }

    #[cold]
    #[inline(never)]
    fn admit_slow(&self, writer: bool) -> bool {
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

    #[inline(always)]
    fn enter(&self) -> Admission {
        self.admit(true)
    }

    #[inline(always)]
    fn leave(&self, admit: Admission) {
        match admit {
            Admission::Slot(slot_idx, entry_idx) => asymmetric::leave_slot(slot_idx, entry_idx),
            Admission::Slow => {
                self.state.0.fetch_sub(RUNNING, SeqCst);
            }
            Admission::None => {}
        }
    }

    fn lock_flush(&self) {
        while self
            .flushing
            .compare_exchange_weak(false, true, Acquire, Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
    }

    fn quiet_now_fast(&self) -> bool {
        let state = self.state.0.load(SeqCst);
        running(state) == 0
            && state & UNTRUSTED == 0
            && (self.synchronous || !asymmetric::any_active(self.tag()))
    }

    fn quiet_now(&self) -> bool {
        if !self.quiet_now_fast() {
            return false;
        }
        if !self.synchronous && asymmetric::flush_writers() {
            self.quiet_now_fast()
        } else {
            true
        }
    }

    fn quiet_state(&self) -> Option<u64> {
        let mut state = self.state.0.load(SeqCst);
        if running(state) != 0 || state & UNTRUSTED != 0 {
            return None;
        }
        if self.synchronous {
            return Some(revision(state));
        }
        let tag = self.tag();
        if asymmetric::current_thread_has(tag) || asymmetric::any_active(tag) {
            return None;
        }
        if asymmetric::flush_writers() {
            state = self.state.0.load(SeqCst);
            if running(state) != 0 || state & UNTRUSTED != 0 || asymmetric::any_active(tag) {
                return None;
            }
        }
        if !self.dirty.load(SeqCst) {
            return Some(revision(state).wrapping_add(self.revision.load(SeqCst)));
        }
        self.lock_flush();
        let result = (|| {
            if self.dirty.swap(false, SeqCst) {
                self.revision.fetch_add(1, SeqCst);
                asymmetric::flush_writers();
            }
            let state = self.state.0.load(SeqCst);
            if running(state) != 0 || state & UNTRUSTED != 0 || asymmetric::any_active(tag) {
                self.dirty.store(true, SeqCst);
                return None;
            }
            Some(revision(state).wrapping_add(self.revision.load(SeqCst)))
        })();
        self.flushing.store(false, Release);
        result
    }

    fn observing(&self) -> bool {
        self.enabled.load(SeqCst) && !self.poisoned.load(SeqCst)
    }

    #[inline(always)]
    fn bump_revision(&self) {
        if !self.synchronous
            && self.state.0.load(Relaxed) == 0
            && let Some((slot_idx, entry_idx)) = asymmetric::enter_slot(self.tag())
        {
            if !self.dirty.load(Relaxed) {
                self.dirty.store(true, Relaxed);
            }
            asymmetric::leave_slot(slot_idx, entry_idx);
            return;
        }
        self.bump_revision_slow();
    }

    #[cold]
    #[inline(never)]
    fn bump_revision_slow(&self) {
        let current = self.state.0.load(Relaxed);
        if current & UNTRUSTED == 0 && revision(current) < LAST_REVISION - (RUNNING_LIMIT / 2) {
            let before = self.state.0.fetch_add(REVISION, SeqCst);
            if before & UNTRUSTED == 0 && revision(before) < LAST_REVISION {
                return;
            }
            self.admit_overflow(REVISION);
            return;
        }
        let admit = self.enter();
        self.leave(admit);
    }

    #[inline]
    pub fn touch(&self, receiver: usize) {
        self.bump_revision();
        let (sequence, mark) = match (receiver != 0).then(|| self.table()).flatten() {
            Some(table) => match table.find(receiver, false) {
                Lookup::Watched(index, sequence) => (Some(sequence), table.mark_of(index)),
                _ => (None, None),
            },
            None => (None, None),
        };
        if self.sequences
            && let Some(sequence) = sequence
        {
            self.increment(sequence);
        }
        if let Some((word, bit)) = mark {
            mark_slot(word, bit);
        }
    }

    #[inline]
    pub fn touch_every(&self) {
        self.bump_revision();
        self.new_epoch();
    }

    #[inline]
    pub fn notify(&self, receiver: usize) -> Notification<'_> {
        let admit = self.enter();
        let (sequence, mark) = match (receiver != 0).then(|| self.table()).flatten() {
            Some(table) => match table.find(receiver, false) {
                Lookup::Watched(index, sequence) => (Some(sequence), table.mark_of(index)),
                _ => (None, None),
            },
            None => (None, None),
        };
        if self.sequences
            && let Some(sequence) = sequence
        {
            self.increment(sequence);
        }
        if let Some((word, bit)) = mark {
            mark_slot(word, bit);
        }
        Notification {
            registry: self,
            mark,
            every: false,
            admit,
        }
    }

    #[inline]
    pub fn notify_pair(&self, first: usize, second: usize) -> NotificationPair<'_> {
        let admit = self.enter();
        let mut marks = [None, None];
        if let Some(table) = self.table() {
            for (slot, receiver) in [first, second].into_iter().enumerate() {
                if receiver == 0 || (slot == 1 && receiver == first) {
                    continue;
                }
                if let Lookup::Watched(index, sequence) = table.find(receiver, false) {
                    if self.sequences {
                        self.increment(sequence);
                    }
                    let mark = table.mark_of(index);
                    if let Some((word, bit)) = mark {
                        mark_slot(word, bit);
                    }
                    marks[slot] = mark;
                }
            }
        }
        NotificationPair {
            registry: self,
            marks,
            admit,
        }
    }

    #[inline]
    pub fn notify_every(&self) -> Notification<'_> {
        let admit = self.enter();
        self.new_epoch();
        Notification {
            registry: self,
            mark: None,
            every: true,
            admit,
        }
    }

    pub fn running(&self) -> Running<'_> {
        Running {
            registry: self,
            admit: self.admit(false),
        }
    }

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

    pub fn takes(&self) -> (u64, u64) {
        (self.taken.load(Relaxed), self.refused_takes.load(Relaxed))
    }

    pub fn quiet(&self) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let rev = self.quiet_state()?;
        Some((epoch, rev))
    }

    pub fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.register(receiver).map(|(certificate, _)| certificate)
    }

    pub fn observed_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let sequence = match self.table()?.find(receiver, false) {
            Lookup::Watched(_, sequence) => sequence.load(SeqCst),
            _ => return None,
        };
        (self.epoch.load(SeqCst) == epoch && self.quiet_now_fast()).then_some((epoch, sequence))
    }

    pub fn range_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let (sequence, fresh) = match self.table()?.find(receiver, true) {
            Lookup::Watched(_, sequence) => (sequence, false),
            Lookup::Registered(_, sequence) => {
                self.registered.fetch_add(1, Relaxed);
                (sequence, true)
            }
            Lookup::Unwatched | Lookup::Full => return None,
        };
        let sequence = sequence.load(SeqCst);
        let quiet = if fresh {
            self.quiet_now()
        } else {
            self.quiet_now_fast()
        };
        (self.epoch.load(SeqCst) == epoch && quiet).then_some((epoch, sequence))
    }

    pub fn observed_sequence_at(
        &self,
        receiver: usize,
        held_slot: usize,
    ) -> Option<((u64, u64), usize)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let table = self.table()?;
        let lookup = table
            .slots
            .get(held_slot)
            .filter(|slot| receiver != 0 && slot.address.load(SeqCst) == receiver)
            .map(|slot| Lookup::Watched(held_slot, &slot.sequence))
            .unwrap_or_else(|| table.find(receiver, false));
        let Lookup::Watched(index, sequence) = lookup else {
            return None;
        };
        let sequence = sequence.load(SeqCst);
        (self.epoch.load(SeqCst) == epoch && self.quiet_now_fast())
            .then_some(((epoch, sequence), index))
    }

    pub fn sequence_at(&self, receiver: usize, held_slot: usize) -> Option<((u64, u64), usize)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let table = self.table()?;
        let lookup = table
            .slots
            .get(held_slot)
            .filter(|slot| receiver != 0 && slot.address.load(SeqCst) == receiver)
            .map(|slot| Lookup::Watched(held_slot, &slot.sequence))
            .unwrap_or_else(|| table.find(receiver, true));
        let (index, sequence, fresh) = match lookup {
            Lookup::Watched(index, sequence) => (index, sequence, false),
            Lookup::Registered(index, sequence) => {
                self.registered.fetch_add(1, Relaxed);
                (index, sequence, true)
            }
            Lookup::Unwatched => return None,
            Lookup::Full => {
                self.refuse();
                return None;
            }
        };
        let sequence = sequence.load(SeqCst);
        let quiet = if fresh {
            self.quiet_now()
        } else {
            self.quiet_now_fast()
        };
        (self.epoch.load(SeqCst) == epoch && quiet).then_some(((epoch, sequence), index))
    }

    pub fn register(&self, receiver: usize) -> Option<((u64, u64), bool)> {
        if !self.observing() {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        let (sequence, fresh) = match self.table()?.find(receiver, true) {
            Lookup::Watched(_, sequence) => (sequence, false),
            Lookup::Registered(_, sequence) => {
                self.registered.fetch_add(1, Relaxed);
                (sequence, true)
            }
            Lookup::Unwatched => return None,
            Lookup::Full => {
                self.refuse();
                return None;
            }
        };
        let sequence = sequence.load(SeqCst);
        let quiet = if fresh {
            self.quiet_now()
        } else {
            self.quiet_now_fast()
        };
        (self.epoch.load(SeqCst) == epoch && quiet).then_some(((epoch, sequence), fresh))
    }

    pub fn boundary(&self, invalidate: bool) -> Boundary {
        if invalidate {
            self.new_epoch();
        }
        let full = self.purge && self.registered.load(Relaxed) * 2 > self.capacity.load(Relaxed);
        let first = self.table.load(SeqCst).is_null();
        let restart = invalidate || first;
        if !restart {
            if self.quiet_now() {
                self.retire_drained();
            }
            if QUIESCENT_GROWTH.get()
                && !self.offered.load(SeqCst).is_null()
                // SAFETY: `with_quiescent_growth` guarantees exclusive capture/registration ownership.
                && unsafe { self.install_growth_quiescent_preserving() }
            {
                return Boundary::Installed;
            }
            if full || !self.offered.load(SeqCst).is_null() {
                self.held.fetch_add(1, Relaxed);
            }
            return Boundary::Unchanged;
        }
        if !self.quiet_now() {
            if full
                || invalidate
                || !self.offered.load(SeqCst).is_null()
                || !self.draining.load(SeqCst).is_null()
            {
                self.deferred.fetch_add(1, Relaxed);
            }
            return Boundary::Deferred;
        }
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
                self.draining.store(previous, SeqCst);
                if self.quiet_now() {
                    self.retire_drained();
                }
                return Boundary::Installed;
            }
        }
        if full || invalidate {
            if let Some(table) = self.table() {
                self.new_epoch();
                table.clear();
                self.new_epoch();
                self.registered.store(0, Relaxed);
                self.clears.fetch_add(1, Relaxed);
                return Boundary::Cleared;
            }
        }
        Boundary::Unchanged
    }

    /// Install prepared growth on a worker while preserving every registered
    /// receiver's sequence and the quiet token. Retire the old allocation for
    /// off-lease collection. This performs no allocation and touches no native
    /// object. Marking registries keep their existing boundary protocol.
    ///
    /// # Safety
    /// The caller holds the native world's quiescent acquisition lease and
    /// exclusively owns this registry's capture/registration side. No notifier,
    /// certificate query, boundary or release may run during the call. Status
    /// readers, offers and collection of an earlier retired table may run.
    pub unsafe fn install_growth_quiescent(&self) -> bool {
        if self.marking {
            return false;
        }
        // SAFETY: caller upholds the quiescent acquisition lease and exclusive capture/registration ownership.
        unsafe { self.install_growth_quiescent_preserving() }
    }

    /// # Safety
    /// The caller holds the native world's quiescent acquisition lease and
    /// exclusively owns this registry's capture/registration side. No
    /// certificate query, boundary or release may run during the call.
    unsafe fn install_growth_quiescent_preserving(&self) -> bool {
        if !self.observing()
            || !self.draining.load(SeqCst).is_null()
            || !self.retired.load(SeqCst).is_null()
        {
            return false;
        }
        let Some(state) = self.quiet_state() else {
            return false;
        };
        let epoch = self.epoch.load(SeqCst);
        let previous = self.table.load(SeqCst);
        let Some(old) = (unsafe { previous.as_ref() }) else {
            return false;
        };
        let offered = self.offered.swap(ptr::null_mut(), SeqCst);
        let Some(next) = (unsafe { offered.as_ref() }) else {
            return false;
        };
        let mut copied_count = 0usize;
        let copied = next.slots.len() > old.slots.len()
            && old.slots.iter().enumerate().all(|(old_index, slot)| {
                let receiver = slot.address.load(SeqCst);
                if receiver == 0 {
                    return true;
                }
                match next.find(receiver, true) {
                    Lookup::Watched(new_index, sequence)
                    | Lookup::Registered(new_index, sequence) => {
                        sequence.store(slot.sequence.load(SeqCst), SeqCst);
                        if self.marking
                            && let Some((old_word, old_bit)) = old.mark_of(old_index)
                            && old_word.load(SeqCst) & old_bit != 0
                            && let Some((new_word, new_bit)) = next.mark_of(new_index)
                        {
                            mark_slot(new_word, new_bit);
                        }
                        copied_count += 1;
                        true
                    }
                    _ => false,
                }
            });
        if !copied
            || self.quiet_state() != Some(state)
            || self.epoch.load(SeqCst) != epoch
            || self.table.load(SeqCst) != previous
        {
            self.requested
                .compare_exchange(next.slots.len(), old.slots.len(), SeqCst, SeqCst)
                .ok();
            self.failed_offers.fetch_add(1, Relaxed);
            self.retired.store(offered, SeqCst);
            return false;
        }
        self.table.store(offered, SeqCst);
        self.capacity.store(next.slots.len(), Relaxed);
        self.registered.store(copied_count, Relaxed);
        self.installs.fetch_add(1, Relaxed);
        if self.quiet_state() == Some(state) {
            self.retired.store(previous, SeqCst);
        } else {
            self.new_epoch();
            self.draining.store(previous, SeqCst);
            if self.quiet_now() {
                self.retire_drained();
            }
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

    pub fn offer(&self, watches: usize) {
        let Some(mut len) = watches
            .checked_mul(SLOTS_PER_WATCH)
            .and_then(|slots| slots.max(MIN_SLOTS).checked_next_power_of_two())
        else {
            let _ = engine_allocation::oom();
            self.failed_offers.fetch_add(1, Relaxed);
            return;
        };
        let refused = self.refused.load(Relaxed);
        let cap = self.capacity.load(Relaxed);
        if refused > self.refused_at_offer.load(Relaxed)
            && cap != 0
            && let Some(grown) = cap.checked_mul(2)
        {
            len = len.max(grown);
        }
        if self.offer_slots(len) {
            self.refused_at_offer.fetch_max(refused, Relaxed);
        }
    }

    pub fn growth_pending(&self) -> bool {
        self.requested.load(SeqCst) > self.capacity.load(Relaxed)
    }

    pub fn capacity(&self) -> usize {
        self.capacity.load(Relaxed)
    }

    pub fn refused(&self) -> u64 {
        self.refused.load(Relaxed)
    }

    fn refuse(&self) {
        self.refused.fetch_add(1, Relaxed);
        REFUSED_TOTAL.fetch_add(1, Relaxed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn defer_retirement(&self) {
        assert!(self.draining.load(SeqCst).is_null());
        let previous = self.retired.swap(ptr::null_mut(), SeqCst);
        assert!(!previous.is_null());
        self.draining.store(previous, SeqCst);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(len: usize) -> Self {
        let registry = Self::new();
        registry.offer_slots(len);
        registry.boundary(false);
        registry.set_enabled(true);
        registry
    }

    fn offer_slots(&self, len: usize) -> bool {
        self.offer_slots_with(len, || {})
    }

    fn offer_slots_with(&self, len: usize, before_publish: impl FnOnce()) -> bool {
        if len <= self.requested.load(SeqCst) {
            return false;
        }
        let Some(table) = Table::new(len, self.marking) else {
            self.failed_offers.fetch_add(1, Relaxed);
            return false;
        };
        before_publish();
        let previous = {
            let _guard = self
                .offer_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if len <= self.requested.load(SeqCst) {
                return false;
            }
            self.requested.store(len, SeqCst);
            self.offered.swap(Box::into_raw(table), SeqCst)
        };
        // SAFETY: an offered table was never installed; the swap owns it.
        unsafe { free(previous) };
        true
    }

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

    pub fn registered(&self) -> usize {
        self.registered.load(Relaxed)
    }

    pub fn active(&self) -> usize {
        if self.synchronous {
            let state = self.state.0.load(SeqCst);
            return if state & UNTRUSTED != 0 {
                UNTRUSTED_ACTIVE
            } else {
                running(state) as usize
            };
        }
        asymmetric::flush_writers();
        let state = self.state.0.load(SeqCst);
        if state & UNTRUSTED != 0 {
            UNTRUSTED_ACTIVE
        } else {
            (running(state) as usize).saturating_add(asymmetric::count_active(self.tag()))
        }
    }

    fn sole_runner_revision(&self) -> Option<u64> {
        if self.synchronous {
            let state = self.state.0.load(SeqCst);
            return (state & UNTRUSTED == 0 && running(state) == 1).then_some(revision(state));
        }
        let state = self.state.0.load(SeqCst);
        if state & UNTRUSTED != 0 {
            return None;
        }
        let tag = self.tag();
        if (running(state) as usize).saturating_add(asymmetric::count_active(tag)) > 1 {
            return None;
        }
        asymmetric::flush_writers();
        let state = self.state.0.load(SeqCst);
        if state & UNTRUSTED != 0 {
            return None;
        }
        let total = (running(state) as usize).saturating_add(asymmetric::count_active(tag));
        if total != 1 {
            return None;
        }
        if !self.dirty.load(SeqCst) {
            return Some(revision(state).wrapping_add(self.revision.load(SeqCst)));
        }
        self.lock_flush();
        let result = (|| {
            if self.dirty.swap(false, SeqCst) {
                self.revision.fetch_add(1, SeqCst);
                asymmetric::flush_writers();
            }
            let state = self.state.0.load(SeqCst);
            let total = (running(state) as usize).saturating_add(asymmetric::count_active(tag));
            if state & UNTRUSTED != 0 || total != 1 {
                self.dirty.store(true, SeqCst);
                return None;
            }
            Some(revision(state).wrapping_add(self.revision.load(SeqCst)))
        })();
        self.flushing.store(false, Release);
        result
    }

}

impl Drop for WatchRegistry {
    fn drop(&mut self) {
        // SAFETY: exclusive access; nothing else can reach the tables.
        unsafe { self.release() };
    }
}

pub struct Notification<'a> {
    registry: &'a WatchRegistry,
    mark: Option<(&'a AtomicU64, u64)>,
    every: bool,
    admit: Admission,
}

impl Notification<'_> {
    pub fn has_mark(&self) -> bool {
        self.mark.is_some()
    }
}

pub struct NotificationPair<'a> {
    registry: &'a WatchRegistry,
    marks: [Option<(&'a AtomicU64, u64)>; 2],
    admit: Admission,
}

impl Drop for NotificationPair<'_> {
    #[inline]
    fn drop(&mut self) {
        for mark in self.marks.into_iter().flatten() {
            mark_slot(mark.0, mark.1);
        }
        self.registry.leave(self.admit);
    }
}

pub struct Running<'a> {
    registry: &'a WatchRegistry,
    admit: Admission,
}

impl Running<'_> {
    pub fn dependency_stamp(&self, receiver: usize) -> Option<(u64, u64)> {
        if !self.admit.active() || !self.registry.observing() {
            return None;
        }
        let epoch = self.registry.epoch.load(SeqCst);
        let before = self.registry.sole_runner_revision()?;
        let sequence = match self.registry.table()?.find(receiver, false) {
            Lookup::Watched(_, sequence) => sequence.load(SeqCst),
            _ => return None,
        };
        (self.registry.sole_runner_revision() == Some(before)
            && self.registry.epoch.load(SeqCst) == epoch
            && self.registry.observing())
        .then_some((epoch, sequence))
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.registry.leave(self.admit);
    }
}

impl Drop for Notification<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some((word, bit)) = self.mark {
            mark_slot(word, bit);
        }
        if self.every {
            self.registry.new_epoch();
        }
        self.registry.leave(self.admit);
    }
}

pub fn refused_total() -> u64 {
    REFUSED_TOTAL.load(Relaxed)
}

pub fn status<const N: usize>(named: [(&'static str, &WatchRegistry); N]) -> impl serde::Serialize {
    use collector_status::status::{map, status};
    let registries = named.map(|(name, registry)| {
        let (registered, capacity) = (registry.registered(), registry.capacity());
        if capacity != 0 {
            PEAK_LOAD.fetch_max((registered as f64 / capacity as f64).to_bits() + 1, Relaxed);
        }
        let counts = status!({
            "registered": registered,
            "capacity": capacity,
            "refused": registry.refused(),
        });
        (name, counts)
    });
    status!({
        "refused_total": refused_total(),
        "peak_load": PEAK_LOAD.load(Relaxed).checked_sub(1).map(f64::from_bits),
        "longest_probe": None::<u64>,
        "note": "refusals since the collector loaded; peak_load is the highest registered/capacity of these registries at any status read; the longest probe is not tracked, as that would add work to registration",
        "registries": map(registries.into_iter()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_warm_lookup_matches_gate_then_registration() {
        const COUNT: usize = 16;
        let watches = WatchRegistry::new();
        watches.offer(COUNT);
        watches.boundary(false);
        watches.set_enabled(true);
        let held: Vec<_> = (1..=COUNT)
            .map(|index| watches.sequence_at(index * 8, usize::MAX).unwrap())
            .collect();
        let old = |receiver, hint| {
            let (_, slot) = watches.observed_sequence_at(receiver, hint)?;
            watches.sequence_at(receiver, slot)
        };
        let counts = || (watches.registered(), watches.refused.load(Relaxed));
        let before = counts();
        collector_test_support::with_allocations_denied(|| {
            for (index, &(_, slot)) in held.iter().enumerate() {
                let receiver = (index + 1) * 8;
                for hint in [slot, held[(index + 1) % COUNT].1, usize::MAX] {
                    assert_eq!(
                        watches.observed_sequence_at(receiver, hint),
                        old(receiver, hint)
                    );
                }
                assert_eq!(watches.observed_sequence_at(receiver + COUNT * 8, slot), None);
            }
        });
        assert_eq!(counts(), before);
        let running = watches.notify(8);
        assert_eq!(
            watches.observed_sequence_at(8, held[0].1),
            old(8, held[0].1)
        );
        assert_eq!(old(8, held[0].1), None);
        drop(running);
        assert_ne!(old(8, held[0].1), Some(held[0]));
        assert_eq!(
            watches.observed_sequence_at(8, held[0].1),
            old(8, held[0].1)
        );
        watches.boundary(true);
        assert_eq!(watches.observed_sequence_at(8, held[0].1), None);
        assert_eq!(old(8, held[0].1), None);
        assert_eq!(watches.registered(), 0, "cleared slots remain cold");

        let full = registry(4);
        for receiver in 1..=4 {
            full.sequence(receiver).unwrap();
        }
        assert_eq!(full.observed_sequence_at(5, usize::MAX), None);
        assert_eq!(full.refused.load(Relaxed), 0, "the warm gate is passive");
        assert_eq!(full.sequence(5), None, "ordered admission still refuses");
        assert_eq!(full.refused.load(Relaxed), 1);
        full.poison();
        assert_eq!(full.observed_sequence_at(1, usize::MAX), None);
    }

    #[test]
    fn hinted_passive_lookup_never_admits_cold_or_cleared_receivers() {
        let registry = WatchRegistry::new();
        registry.offer(2);
        registry.boundary(false);
        registry.set_enabled(true);
        let held = registry.sequence_at(8, usize::MAX).unwrap();
        assert_eq!(registry.observed_sequence_at(8, held.1), Some(held));
        let admitted = registry.registered.load(Relaxed);
        let refused = registry.refused.load(Relaxed);
        for receiver in (16..=5000 * 8).step_by(8) {
            assert_eq!(registry.observed_sequence_at(receiver, held.1), None);
        }
        assert_eq!(registry.registered.load(Relaxed), admitted);
        assert_eq!(registry.refused.load(Relaxed), refused);
        registry.boundary(true);
        assert_eq!(registry.observed_sequence_at(8, held.1), None);
        assert_eq!(registry.refused.load(Relaxed), refused);
    }

    #[test]
    fn passive_sequences_preserve_registration_admission_and_counters() {
        let registry = registry(64);
        let first = registry.sequence(0x1000).unwrap();
        let registered = registry.registered.load(Relaxed);
        let refused = registry.refused.load(Relaxed);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(registry.observed_sequence(0x1000), Some(first));
            for receiver in 0..5_000 {
                assert_eq!(registry.observed_sequence(0x2000 + receiver * 8), None);
            }
        });
        assert_eq!(registry.registered.load(Relaxed), registered);
        assert_eq!(registry.refused.load(Relaxed), refused);
        assert!(registry.sequence(0x2000).is_some());
        let running = registry.notify(0x1000);
        assert!(registry.observed_sequence(0x1000).is_none());
        drop(running);
        assert_eq!(
            registry.observed_sequence(0x1000),
            registry.sequence(0x1000)
        );
        registry.set_enabled(false);
        assert!(registry.observed_sequence(0x1000).is_none());
    }

    #[test]
    fn notification_attributes_only_prepared_marking_slots_without_allocating() {
        let watches = WatchRegistry::marking();
        watches.offer(2);
        watches.set_enabled(true);
        watches.boundary(true);
        watches.register(7).unwrap();
        collector_test_support::with_allocations_denied(|| {
            assert!(watches.notify(7).has_mark());
            assert!(!watches.notify(8).has_mark());
            assert!(!watches.notify(0).has_mark());
        });
        let mut marked = Vec::new();
        assert!(watches.take(|key| marked.push(key)).is_some());
        assert_eq!(marked, [7]);
        unsafe { watches.release() };
    }
    #[test]
    fn hook_dependency_stamp_excludes_other_writers_and_is_not_an_acquisition_certificate() {
        let watches = registry(64);
        let before = watches.sequence(0x1000).unwrap();
        let running = watches.running();
        assert!(watches.sequence(0x1000).is_none());
        assert_eq!(running.dependency_stamp(0x1000), Some(before));
        let writer = watches.notify(0x1000);
        assert!(running.dependency_stamp(0x1000).is_none());
        drop(writer);
        assert_ne!(running.dependency_stamp(0x1000), Some(before));
        let other = watches.running();
        assert!(running.dependency_stamp(0x1000).is_none());
        drop(other);
        assert!(running.dependency_stamp(0x1000).is_some());
        drop(running);
        assert!(watches.sequence(0x1000).is_some());
    }

    fn registry(len: usize) -> WatchRegistry {
        let registry = WatchRegistry::enabled(len);
        assert_eq!(registry.capacity.load(Relaxed), len);
        registry
    }

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
    fn held_slot_certificates_keep_writer_and_running_semantics() {
        let registry = registry(64);
        let (held, slot) = registry.sequence_at(8, usize::MAX).unwrap();
        let other = registry.sequence_at(16, usize::MAX).unwrap();
        assert_eq!(
            registry.table().unwrap().slots[slot].address.load(SeqCst),
            8
        );
        assert_eq!(registry.sequence(8), Some(held));
        for _ in 0..3 {
            assert_eq!(registry.sequence_at(8, slot), Some((held, slot)));
        }
        assert_eq!(registry.registered(), 2);
        let quiet = registry.quiet();
        let running = registry.running();
        assert!(registry.sequence_at(8, slot).is_none());
        assert!(registry.quiet().is_none());
        drop(running);
        assert_eq!(registry.sequence_at(8, slot), Some((held, slot)));
        assert_eq!(registry.quiet(), quiet);
        let writer = registry.notify(8);
        assert!(registry.sequence_at(8, slot).is_none());
        assert!(registry.sequence_at(16, other.1).is_none());
        drop(writer);
        assert_eq!(
            registry.sequence_at(8, slot),
            Some(((held.0, held.1 + 1), slot))
        );
        assert_eq!(registry.sequence_at(16, other.1), Some(other));
        assert_eq!(registry.registered(), 2);
    }

    #[test]
    fn held_slot_hints_validate_the_receiver_and_bounds() {
        let registry = registry(64);
        let first = registry.sequence_at(8, usize::MAX).unwrap();
        let second = registry.sequence_at(16, first.1).unwrap();
        assert_ne!(first.1, second.1);
        assert_eq!(registry.sequence_at(8, second.1), Some(first));
        assert_eq!(registry.sequence_at(16, first.1), Some(second));
        assert_eq!(registry.sequence_at(8, 64), Some(first));
        assert_eq!(registry.sequence_at(8, usize::MAX), Some(first));
        assert!(registry.sequence_at(0, first.1).is_none());
        assert!(registry.sequence_at(0, usize::MAX).is_none());
        assert_eq!(registry.registered(), 2);
        assert_eq!(registry.refused.load(Relaxed), 0);
    }

    #[test]
    fn held_slot_hints_do_not_preserve_a_cleared_or_replaced_epoch() {
        let registry = registry(64);
        let (_, slot) = registry.sequence_at(8, usize::MAX).unwrap();
        drop(registry.notify(8));
        let held = registry.sequence_at(8, slot).unwrap();
        registry.boundary(true);
        let cleared = registry.sequence_at(8, slot).unwrap();
        assert_ne!(cleared.0.0, held.0.0);
        assert_eq!(cleared.0.1, 0);
        assert_eq!(registry.sequence(8), Some(cleared.0));
        registry.offer(1_000);
        assert!(!registry.boundary(false).installed());
        assert_eq!(registry.sequence_at(8, cleared.1), Some(cleared));
        assert!(registry.boundary(true).installed());
        registry.collect();
        let installed = registry.sequence_at(8, cleared.1).unwrap();
        assert_ne!(installed.0.0, cleared.0.0);
        assert_eq!(registry.sequence(8), Some(installed.0));
        drop(registry.notify(8));
        assert_eq!(
            registry.sequence_at(8, installed.1),
            Some(((installed.0.0, installed.0.1 + 1), installed.1))
        );
        assert_eq!(registry.registered(), 1);
    }

    #[test]
    fn held_slot_certificates_refuse_disabled_poisoned_or_untrusted_observation() {
        let empty = WatchRegistry::new();
        empty.set_enabled(true);
        assert!(empty.sequence_at(8, usize::MAX).is_none());
        let registry = registry(64);
        let held = registry.sequence_at(8, usize::MAX).unwrap();
        registry.set_enabled(false);
        assert!(registry.sequence_at(8, held.1).is_none());
        assert!(registry.quiet().is_none());
        registry.set_enabled(true);
        let enabled = registry.sequence_at(8, held.1).unwrap();
        assert_ne!(enabled.0.0, held.0.0);
        registry.state.0.store(UNTRUSTED, SeqCst);
        assert!(registry.sequence_at(8, held.1).is_none());
        registry.state.0.store(0, SeqCst);
        registry.poison();
        assert!(registry.sequence_at(8, held.1).is_none());
        assert!(registry.quiet().is_none());
        assert_eq!(registry.registered(), 1);
    }

    #[test]
    fn held_slot_registration_keeps_full_table_refusal_counts() {
        let registry = registry(4);
        let mut slots = Vec::new();
        for receiver in [8, 16, 24, 32] {
            slots.push(registry.sequence_at(receiver, usize::MAX).unwrap());
        }
        assert_eq!(registry.registered(), 4);
        assert!(registry.sequence_at(40, slots[0].1).is_none());
        assert_eq!(registry.refused.load(Relaxed), 1);
        assert!(registry.sequence(40).is_none());
        assert_eq!(registry.refused.load(Relaxed), 2);
        for (receiver, held) in [8, 16, 24, 32].into_iter().zip(slots) {
            assert_eq!(registry.sequence_at(receiver, held.1), Some(held));
        }
        assert_eq!(registry.registered(), 4);
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
        let quiet = registry.quiet().unwrap();
        drop(registry.notify(40));
        assert_ne!(registry.quiet(), Some(quiet));
        assert_eq!(registry.sequence(24).map(|(_, s)| s), Some(0));
    }

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
    fn dense_country_ids_and_aligned_holders_at_5000_countries_register_without_allocating() {
        const COUNTRIES: usize = 5_000;
        let registry = WatchRegistry::new();
        registry.offer(COUNTRIES * 2);
        registry.boundary(false);
        registry.set_enabled(true);
        let refused = crate::test_support::with_allocations_denied(|| {
            for country in 1..=COUNTRIES {
                let holder = 0x21800 + (country - 1) * 0x2000;
                for receiver in [country, holder] {
                    if registry.sequence(receiver).is_none() {
                        return Some(receiver);
                    }
                }
            }
            None
        });
        assert_eq!(refused, None);
        assert_eq!(registry.refused.load(Relaxed), 0);
        assert_eq!(registry.registered(), COUNTRIES * 2);
    }

    #[test]
    fn a_clear_starts_an_epoch_so_no_certificate_survives_it() {
        let registry = registry(64);
        let certified = registry.sequence(8).unwrap();
        assert_eq!(registry.boundary(true), Boundary::Cleared);
        assert_eq!(registry.clears.load(Relaxed), 1);
        assert_eq!(registry.registered.load(Relaxed), 0);
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
        assert_eq!(registry.boundary(true), Boundary::Deferred);
        assert_eq!(registry.deferred.load(Relaxed), 1);
        assert_eq!(registry.clears.load(Relaxed), 0);
        assert!(registry.sequence(8).is_none());
        assert!(registry.quiet().is_none());
        drop(running);
        assert_ne!(registry.sequence(8), Some(certified));
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

    #[test]
    fn a_notification_spanning_a_clear_never_advances_a_new_certificate() {
        for bumped_after_clear in [false, true] {
            let registry = registry(64);
            registry.sequence(8).unwrap();
            let table = registry.table().unwrap();
            let Lookup::Watched(_, slot) = table.find(8, false) else {
                unreachable!("registered above")
            };
            let running = (!bumped_after_clear).then(|| registry.notify(8));
            registry.new_epoch();
            table.clear();
            registry.new_epoch();
            let running = running.unwrap_or_else(|| {
                let admit = registry.enter();
                slot.fetch_add(1, SeqCst);
                Notification {
                    registry: &registry,
                    mark: None,
                    every: false,
                    admit,
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
        registry.offer_slots(4);
        assert!(registry.offered.load(SeqCst).is_null());
        registry.offer(1_000);
        assert!(!registry.boundary(false).installed());
        assert_eq!(registry.capacity.load(Relaxed), 4);
        assert_eq!(registry.held.load(Relaxed), 1);
        assert_eq!(registry.sequence(8), Some(certified));
        let running = registry.notify(8);
        registry.boundary(true);
        assert_eq!(registry.capacity.load(Relaxed), 4);
        drop(running);
        assert!(!registry.boundary(false).installed());
        assert_eq!(registry.capacity.load(Relaxed), 4);
        assert!(registry.boundary(true).installed());
        assert_eq!(registry.capacity.load(Relaxed), 4096);
        assert_eq!(registry.installs.load(Relaxed), 2);
        assert_ne!(registry.sequence(8), Some(certified));
        assert!(registry.draining.load(SeqCst).is_null());
        assert!(!registry.retired.load(SeqCst).is_null());
        registry.collect();
        assert!(registry.retired.load(SeqCst).is_null());
    }

    #[test]
    fn pending_growth_survives_deferred_boundaries_and_blocked_fallback_clears() {
        for marking in [false, true] {
            let registry = if marking {
                WatchRegistry::marking()
            } else {
                WatchRegistry::new()
            };
            assert!(!registry.growth_pending());
            assert_eq!(registry.boundary(true), Boundary::Unchanged);
            registry.offer(1);
            assert!(registry.growth_pending());
            let running = registry.running();
            assert_eq!(registry.boundary(false), Boundary::Deferred);
            assert!(registry.growth_pending());
            drop(running);
            assert_eq!(registry.boundary(false), Boundary::Installed);
            assert!(!registry.growth_pending());
            registry.set_enabled(true);

            registry.offer(MIN_SLOTS);
            assert_eq!(registry.boundary(false), Boundary::Unchanged);
            assert!(registry.growth_pending());
            assert_eq!(registry.boundary(true), Boundary::Installed);
            assert!(!registry.growth_pending());
            registry.defer_retirement();
            assert!(!registry.draining.load(SeqCst).is_null());
            assert!(registry.retired.load(SeqCst).is_null());
            assert!(!registry.growth_pending());

            registry.offer(MIN_SLOTS * 2);
            assert!(registry.growth_pending());
            let running = registry.running();
            assert_eq!(registry.boundary(true), Boundary::Deferred);
            assert!(registry.growth_pending());
            assert!(!registry.draining.load(SeqCst).is_null());
            drop(running);
            for _ in 0..2 {
                assert_eq!(registry.boundary(true), Boundary::Cleared);
                assert!(registry.draining.load(SeqCst).is_null());
                assert!(!registry.retired.load(SeqCst).is_null());
                assert!(registry.growth_pending());
            }
            registry.collect();
            assert!(registry.retired.load(SeqCst).is_null());
            assert!(registry.growth_pending());
            assert_eq!(registry.boundary(false), Boundary::Unchanged);
            assert!(registry.growth_pending());
            assert_eq!(registry.boundary(true), Boundary::Installed);
            assert!(!registry.growth_pending());
            assert!(!registry.retired.load(SeqCst).is_null());
            registry.collect();
            assert!(!registry.growth_pending());
        }
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
        let (after_epoch, after_revision) = registry.quiet().unwrap();
        assert_eq!(after_epoch, epoch);
        assert!(after_revision > initial_revision);
        drop(registry.running());
        assert_eq!(registry.quiet(), Some((epoch, after_revision)));
        assert_eq!(std::mem::align_of::<Hot>(), 64);
        assert_eq!(std::mem::size_of::<Hot>(), 64);
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

    #[test]
    fn the_admission_at_the_running_limit_makes_the_word_untrusted() {
        for guard in [false, true] {
            let registry = registry(64);
            registry.sequence(8).unwrap();
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
            drop(hold());
            drop(refused);
            assert_eq!(running(registry.state.0.load(SeqCst)), RUNNING_LIMIT - 1);
            drop(counted);
            assert_eq!(running(registry.state.0.load(SeqCst)), RUNNING_LIMIT - 2);
            registry
                .state
                .0
                .store(UNTRUSTED | (9 << RUNNING_BITS), SeqCst);
            assert_eq!(registry.active(), UNTRUSTED_ACTIVE);
            assert!(registry.quiet().is_none() && registry.sequence(8).is_none());
            let installed = registry.table.load(SeqCst);
            registry.offer(1_000);
            for invalidate in [false, true] {
                assert!(!registry.boundary(invalidate).installed());
            }
            assert_eq!(registry.table.load(SeqCst), installed, "no install");
            assert_eq!(registry.draining.load(SeqCst), draining, "not retired");
            assert!(registry.retired.load(SeqCst).is_null());
        }
    }

    #[test]
    #[ignore = "release per-call notification cost; run with --release --ignored --nocapture --test-threads=1"]
    fn hook_cost_watch_registry() {
        const RECEIVERS: usize = 1500;
        let registry = registry(8192);
        let receiver = |index: usize| 0x10_0000 + index * 0x200;
        for index in 0..RECEIVERS {
            registry.sequence(receiver(index)).unwrap();
        }
        let one = crate::test_support::per_call_ns(RECEIVERS, || {
            for index in 0..RECEIVERS {
                drop(std::hint::black_box(registry.notify(receiver(index))));
            }
        });
        let four = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        crate::test_support::per_call_ns(RECEIVERS, || {
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
        assert_eq!(registry.register(48).map(|(_, fresh)| fresh), Some(true));
        assert_eq!(registry.register(48).map(|(_, fresh)| fresh), Some(false));
        drop(registry.notify(16));
        drop(registry.notify(16));
        drop(registry.notify(40));
        assert_eq!(take(&registry), (Some(epoch), vec![16]));
        assert_eq!(take(&registry), (Some(epoch), vec![]));
        assert_eq!(registry.takes(), (1, 0));
        let running = registry.notify(24);
        assert_eq!(take(&registry), (None, vec![]));
        drop(running);
        assert_eq!(take(&registry), (Some(epoch), vec![24]));
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
        registry.boundary(true);
        assert_ne!(take(&registry).0, Some(epoch));
    }

    #[test]
    fn repeated_marks_keep_both_writers_running_and_reassert_after_a_take() {
        let registry = marking(64);
        let receiver = 8;
        let before = registry.sequence(receiver).unwrap();
        let epoch = before.0;
        let first = registry.notify(receiver);
        let second = registry.notify(receiver);
        assert_eq!(registry.active(), 2);
        assert_eq!(registry.sequence(receiver), None);
        assert_eq!(take(&registry), (None, vec![]));
        drop(first);
        assert_eq!(registry.active(), 1);
        assert_eq!(take(&registry), (None, vec![]));
        drop(second);
        assert_eq!(registry.sequence(receiver), Some((epoch, before.1 + 2)));
        assert_eq!(take(&registry), (Some(epoch), vec![receiver]));
        let mut writer = None;
        drop(registry.notify(receiver));
        assert_eq!(
            registry.take(|address| {
                assert_eq!(address, receiver);
                drop(registry.notify(receiver));
                writer = Some(registry.notify(receiver));
            }),
            None
        );
        drop(writer);
        assert_eq!(take(&registry), (Some(epoch), vec![receiver]));
        assert_eq!(take(&registry), (Some(epoch), vec![]));
    }

    #[test]
    fn receiver_zero_changes_only_the_domain_revision_and_running_count() {
        let registry = registry(64);
        let before = registry.sequence(8).unwrap();
        let quiet = registry.quiet().unwrap();
        let writer = registry.notify(0);
        assert_eq!(registry.active(), 1);
        assert_eq!(registry.sequence(8), None);
        assert_eq!(registry.quiet(), None);
        drop(writer);
        assert_eq!(registry.sequence(8), Some(before));
        assert_eq!(registry.quiet(), Some((quiet.0, quiet.1 + 1)));
        assert_eq!(registry.registered(), 1);
        assert_eq!(registry.register(0), None);
    }

    #[test]
    #[ignore = "per-call bitmap mark microbenchmark; run only without a live game benchmark"]
    fn hook_cost_repeated_marks() {
        const CALLS: usize = 100_000;
        let word = AtomicU64::new(1);
        let unconditional = crate::test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                std::hint::black_box(&word).fetch_or(1, SeqCst);
                std::hint::black_box(&word).fetch_or(1, SeqCst);
            }
        });
        let already_set = crate::test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                mark_slot(std::hint::black_box(&word), 1);
                mark_slot(std::hint::black_box(&word), 1);
            }
        });
        let after_take = crate::test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                word.swap(0, SeqCst);
                mark_slot(std::hint::black_box(&word), 1);
                mark_slot(std::hint::black_box(&word), 1);
            }
        });
        println!(
            "hook_cost mark entry+return: unconditional={unconditional:.1} ns, already_set={already_set:.1} ns, take+first_mark={after_take:.1} ns"
        );
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
    #[test]
    fn faction_goal_worker_growth_preserves_live_certificates_and_quiet_token() {
        let registry = WatchRegistry::enabled(64);
        let first = registry.sequence(0x1000).unwrap();
        drop(registry.notify(0x2000));
        let second = registry.sequence(0x2000).unwrap();
        drop(registry.notify(0x2000));
        let second = registry
            .sequence(0x2000)
            .filter(|value| *value != second)
            .unwrap();
        let quiet = registry.quiet().unwrap();
        registry.offer(5000);
        crate::test_support::with_allocations_denied(|| {
            assert!(unsafe { registry.install_growth_quiescent() });
        });
        assert_eq!(registry.quiet(), Some(quiet));
        assert_eq!(registry.sequence(0x1000), Some(first));
        assert_eq!(registry.sequence(0x2000), Some(second));
        for index in 0..5000 {
            assert!(registry.sequence(0x10000 + index * 0x2000).is_some());
        }
        registry.collect();
        unsafe {
            registry.release();
        }
    }
    #[test]
    fn faction_goal_worker_growth_waits_for_writers_and_never_takes_marking_storage() {
        let registry = WatchRegistry::enabled(64);
        registry.offer(5000);
        let running = registry.notify(0x1000);
        assert!(!unsafe { registry.install_growth_quiescent() });
        assert!(!registry.offered.load(SeqCst).is_null());
        drop(running);
        assert!(unsafe { registry.install_growth_quiescent() });
        registry.collect();
        unsafe {
            registry.release();
        }
        let marking = WatchRegistry::marking();
        marking.offer(1);
        marking.boundary(false);
        marking.set_enabled(true);
        marking.offer(5000);
        assert!(!unsafe { marking.install_growth_quiescent() });
        assert!(!marking.offered.load(SeqCst).is_null());
        unsafe {
            marking.release();
        }
    }
    #[test]
    fn faction_goal_worker_growth_rejection_retires_off_lease_and_allows_retry() {
        let registry = WatchRegistry::enabled(64);
        let certificate = registry.sequence(0x1000).unwrap();
        let rejected = Table::new(128, false).unwrap();
        for (index, slot) in rejected.slots.iter().enumerate() {
            slot.address.store(0x10000 + index * 8, SeqCst);
        }
        registry.offered.store(Box::into_raw(rejected), SeqCst);
        registry.requested.store(128, SeqCst);
        crate::test_support::with_allocations_denied(|| {
            assert!(!unsafe { registry.install_growth_quiescent() });
        });
        assert!(!registry.retired.load(SeqCst).is_null());
        assert_eq!(registry.sequence(0x1000), Some(certificate));
        registry.collect();
        registry.offer_slots(128);
        assert!(unsafe { registry.install_growth_quiescent() });
        assert_eq!(registry.sequence(0x1000), Some(certificate));
        registry.collect();
        unsafe {
            registry.release();
        }
    }
    #[test]
    fn quiescent_growth_boundary_preserves_sequences_marks_and_self_heals_refusals() {
        let marking = WatchRegistry::marking();
        marking.offer_slots(64);
        marking.boundary(false);
        marking.set_enabled(true);
        let a = marking.sequence(0x1000).unwrap();
        let b = marking.sequence(0x2000).unwrap();
        drop(marking.notify(0x2000));
        let b_after = marking.sequence(0x2000).unwrap();
        assert_ne!(b_after, b);
        let quiet = marking.quiet().unwrap();
        marking.offer(500);
        assert_eq!(marking.boundary(false), Boundary::Unchanged);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(
                unsafe { with_quiescent_growth(|| marking.boundary(false)) },
                Boundary::Installed
            );
        });
        assert_eq!(marking.quiet(), Some(quiet));
        assert_eq!(marking.sequence(0x1000), Some(a));
        assert_eq!(marking.sequence(0x2000), Some(b_after));
        assert_eq!(take(&marking), (Some(a.0), vec![0x2000]));
        assert_eq!(marking.registered(), 2);
        marking.collect();
        unsafe { marking.release() };

        let small = registry(4);
        for address in [8, 16, 24, 32] {
            small.sequence(address).unwrap();
        }
        assert!(small.sequence(40).is_none());
        assert_eq!(small.refused(), 1);
        small.offer(1);
        assert_eq!(
            unsafe { with_quiescent_growth(|| small.boundary(false)) },
            Boundary::Installed
        );
        assert!(small.capacity() >= 1024);
        assert!(small.sequence(40).is_some());
        small.collect();
        unsafe { small.release() };
    }

    #[test]
    fn concurrent_smaller_offer_does_not_replace_larger_pending_offer() {
        let registry = WatchRegistry::enabled(64);
        registry.offer_slots_with(128, || {
            registry.offer_slots(512);
        });
        assert_eq!(registry.requested.load(SeqCst), 512);
        assert!(registry.boundary(true).installed());
        assert_eq!(registry.capacity(), 512);
        registry.collect();
        unsafe {
            registry.release();
        }
    }
}
