use std::{
    cell::Cell,
    sync::atomic::{
        AtomicBool, AtomicU8, AtomicU64, AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, SeqCst},
        compiler_fence,
    },
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn FlushProcessWriteBuffers();
}

const THREAD_SLOTS: usize = 128;
const SLOT_DEPTH: usize = 8;
const OVERFLOW_SENTINEL: usize = THREAD_SLOTS;
const QUIESCENCE_OFF: u8 = 0;
const QUIESCENCE_ARMING: u8 = 1;
const QUIESCENCE_ACTIVE: u8 = 2;
const BROKEN_WORDS: usize = 4;

#[repr(align(64))]
struct ThreadSlot {
    active: [AtomicUsize; SLOT_DEPTH],
}

impl ThreadSlot {
    const fn new() -> Self {
        Self {
            active: [const { AtomicUsize::new(0) }; SLOT_DEPTH],
        }
    }
}

#[repr(align(64))]
struct BrokenTags([AtomicU64; BROKEN_WORDS]);

static SLOTS: [ThreadSlot; THREAD_SLOTS] = [const { ThreadSlot::new() }; THREAD_SLOTS];
static CLAIMED: AtomicUsize = AtomicUsize::new(0);
static QUIESCENCE: AtomicU8 = AtomicU8::new(QUIESCENCE_OFF);
static BROKEN_TAGS: BrokenTags = BrokenTags([const { AtomicU64::new(0) }; BROKEN_WORDS]);

thread_local! {
    static SLOT: Cell<usize> = const { Cell::new(usize::MAX) };
}

#[inline(always)]
fn tag_bit(tag: usize) -> (usize, u64) {
    let hash = (tag as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56;
    let index = (hash >> 6) as usize;
    let bit = 1_u64 << (hash & 63);
    (index, bit)
}

#[inline(always)]
fn tag_broken(tag: usize) -> bool {
    let (index, bit) = tag_bit(tag);
    BROKEN_TAGS.0[index].load(SeqCst) & bit != 0
}

#[cold]
#[inline(never)]
fn mark_tag_broken(tag: usize) {
    let (index, bit) = tag_bit(tag);
    BROKEN_TAGS.0[index].fetch_or(bit, SeqCst);
}

#[inline(always)]
fn is_quiescent_for(tag: usize) -> bool {
    QUIESCENCE.load(Acquire) == QUIESCENCE_ACTIVE && !tag_broken(tag)
}

#[cold]
#[inline(never)]
fn claim_slot() -> usize {
    let slot = CLAIMED
        .fetch_update(AcqRel, Relaxed, |count| {
            (count < THREAD_SLOTS).then_some(count + 1)
        })
        .unwrap_or(OVERFLOW_SENTINEL);
    SLOT.set(slot);
    slot
}

#[inline(always)]
fn thread_slot() -> usize {
    let slot = SLOT.get();
    if slot == usize::MAX {
        claim_slot()
    } else {
        slot
    }
}

#[inline(always)]
pub fn enter_slot(tag: usize) -> Option<(u8, u8)> {
    debug_assert_ne!(tag, 0);
    let slot_idx = thread_slot();
    if slot_idx >= THREAD_SLOTS {
        return None;
    }
    let active = &SLOTS[slot_idx].active;
    if active[0].load(Relaxed) == 0 {
        active[0].store(tag, Relaxed);
        compiler_fence(SeqCst);
        if QUIESCENCE.load(Relaxed) != QUIESCENCE_OFF {
            mark_tag_broken(tag);
        }
        return Some((slot_idx as u8, 0));
    }
    for entry in 1..SLOT_DEPTH {
        if active[entry].load(Relaxed) == 0 {
            active[entry].store(tag, Relaxed);
            compiler_fence(SeqCst);
            if QUIESCENCE.load(Relaxed) != QUIESCENCE_OFF {
                mark_tag_broken(tag);
            }
            return Some((slot_idx as u8, entry as u8));
        }
    }
    None
}

#[inline(always)]
pub fn leave_slot(slot_idx: u8, entry_idx: u8) {
    compiler_fence(SeqCst);
    SLOTS[slot_idx as usize].active[entry_idx as usize].store(0, Relaxed);
}

#[inline(always)]
pub fn current_thread_has(tag: usize) -> bool {
    let slot_idx = SLOT.get();
    if slot_idx >= THREAD_SLOTS {
        return false;
    }
    let active = &SLOTS[slot_idx].active;
    if active[0].load(Relaxed) == tag {
        return true;
    }
    for entry in 1..SLOT_DEPTH {
        if active[entry].load(Relaxed) == tag {
            return true;
        }
    }
    false
}

#[inline]
pub fn any_active(tag: usize) -> bool {
    if is_quiescent_for(tag) {
        return false;
    }
    let count = CLAIMED.load(Acquire);
    for slot in &SLOTS[..count] {
        for entry in &slot.active {
            if entry.load(SeqCst) == tag {
                return true;
            }
        }
    }
    false
}

#[inline]
pub fn count_active(tag: usize) -> usize {
    if is_quiescent_for(tag) {
        return 0;
    }
    let count = CLAIMED.load(Acquire);
    let mut total = 0usize;
    for slot in &SLOTS[..count] {
        for entry in &slot.active {
            if entry.load(SeqCst) == tag {
                total += 1;
            }
        }
    }
    total
}

pub struct CaptureQuiescence {
    active: bool,
}

impl Drop for CaptureQuiescence {
    fn drop(&mut self) {
        if self.active {
            QUIESCENCE.store(QUIESCENCE_OFF, SeqCst);
        }
    }
}

pub fn begin_capture_quiescence(_all_other_hooks_quiet: impl FnMut() -> bool) -> CaptureQuiescence {
    for word in &BROKEN_TAGS.0 {
        word.store(0, SeqCst);
    }
    QUIESCENCE.store(QUIESCENCE_ARMING, SeqCst);
    // SAFETY: `FlushProcessWriteBuffers` is a stateless kernel32 synchronization barrier.
    unsafe { FlushProcessWriteBuffers() };
    let count = CLAIMED.load(Acquire);
    for slot in &SLOTS[..count] {
        for entry in &slot.active {
            let tag = entry.load(SeqCst);
            if tag != 0 {
                mark_tag_broken(tag);
            }
        }
    }
    QUIESCENCE.store(QUIESCENCE_ACTIVE, SeqCst);
    CaptureQuiescence { active: true }
}

#[inline]
fn flush_writers_uncached() -> bool {
    let claimed = CLAIMED.load(Acquire);
    if claimed == 0 || (claimed == 1 && SLOT.get() == 0) {
        return false;
    }
    // SAFETY: `FlushProcessWriteBuffers` is a stateless kernel32 synchronization barrier.
    unsafe { FlushProcessWriteBuffers() };
    true
}

#[inline]
pub fn flush_writers_for(tag: usize) -> bool {
    if is_quiescent_for(tag) {
        return false;
    }
    flush_writers_uncached()
}

#[inline]
pub fn flush_writers() -> bool {
    if QUIESCENCE.load(Acquire) == QUIESCENCE_ACTIVE
        && BROKEN_TAGS.0.iter().all(|word| word.load(SeqCst) == 0)
    {
        return false;
    }
    flush_writers_uncached()
}

pub const RETAIN_BIT: usize = 1 << (usize::BITS - 1);

#[repr(align(64))]
pub struct FlightGate {
    overflow: AtomicUsize,
    dirty: AtomicBool,
    epoch: AtomicU64,
    flush_seq: AtomicU64,
}

impl FlightGate {
    pub const fn new(initial_epoch: u64) -> Self {
        Self {
            overflow: AtomicUsize::new(0),
            dirty: AtomicBool::new(false),
            epoch: AtomicU64::new(initial_epoch),
            flush_seq: AtomicU64::new(0),
        }
    }

    #[inline(always)]
    fn tag(&self) -> usize {
        std::ptr::from_ref::<Self>(self) as usize
    }

    #[inline(always)]
    pub fn enter(&self, mark_dirty: bool) -> FlightGuard<'_> {
        let slot = if self.overflow.load(Relaxed) == 0 {
            enter_slot(self.tag())
        } else {
            None
        };
        if let Some((slot_idx, entry_idx)) = slot {
            if mark_dirty && !self.dirty.load(Relaxed) {
                self.dirty.store(true, Relaxed);
            }
            return FlightGuard {
                gate: self,
                slot: Some((slot_idx, entry_idx)),
            };
        }
        self.enter_slow(mark_dirty)
    }

    #[cold]
    #[inline(never)]
    fn enter_slow(&self, mark_dirty: bool) -> FlightGuard<'_> {
        self.overflow.fetch_add(1, SeqCst);
        if mark_dirty {
            self.dirty.store(true, SeqCst);
        }
        FlightGuard {
            gate: self,
            slot: None,
        }
    }

    #[inline(always)]
    pub fn mark_dirty(&self) {
        if self.overflow.load(Relaxed) == 0
            && let Some((slot_idx, entry_idx)) = enter_slot(self.tag())
        {
            if !self.dirty.load(Relaxed) {
                self.dirty.store(true, Relaxed);
            }
            leave_slot(slot_idx, entry_idx);
            return;
        }
        self.dirty.store(true, SeqCst);
    }

    #[inline(always)]
    pub fn is_retained(&self) -> bool {
        self.overflow.load(Relaxed) & RETAIN_BIT != 0
    }

    pub fn active(&self) -> usize {
        let tag = self.tag();
        flush_writers_for(tag);
        let before = self.overflow.load(SeqCst);
        let mut slots = count_active(tag);
        let mut after = self.overflow.load(SeqCst);
        if after != before {
            slots = count_active(tag);
            after = self.overflow.load(SeqCst);
        }
        let retained = after & RETAIN_BIT;
        let count = (after & !RETAIN_BIT).saturating_add(slots);
        retained | (count & !RETAIN_BIT)
    }

    pub fn any_active(&self) -> bool {
        let tag = self.tag();
        if current_thread_has(tag)
            || any_active(tag)
            || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0
        {
            return true;
        }
        if flush_writers_for(tag) {
            any_active(tag) || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0
        } else {
            false
        }
    }

    fn lock_flush(&self) -> u64 {
        loop {
            let seq = self.flush_seq.load(Acquire);
            if seq & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            if self
                .flush_seq
                .compare_exchange_weak(seq, seq + 1, SeqCst, Relaxed)
                .is_ok()
            {
                return seq + 1;
            }
        }
    }

    fn unlock_flush(&self, locked: u64) {
        self.flush_seq.store(locked + 1, SeqCst);
    }

    fn clean_epoch(&self) -> Option<u64> {
        let seq = self.flush_seq.load(SeqCst);
        if seq & 1 != 0 || self.dirty.load(SeqCst) {
            return None;
        }
        let epoch = self.epoch.load(SeqCst);
        (self.flush_seq.load(SeqCst) == seq).then_some(epoch)
    }

    pub fn quiet_epoch(&self) -> Option<u64> {
        let tag = self.tag();
        if current_thread_has(tag)
            || any_active(tag)
            || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0
        {
            return None;
        }
        if flush_writers_for(tag)
            && (any_active(tag) || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0)
        {
            return None;
        }
        if let Some(epoch) = self.clean_epoch() {
            return Some(epoch);
        }
        let locked = self.lock_flush();
        let result = (|| {
            if self.dirty.swap(false, SeqCst) {
                self.epoch.fetch_add(1, SeqCst);
                flush_writers_for(tag);
            }
            if any_active(tag) || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0 {
                self.dirty.store(true, SeqCst);
                return None;
            }
            Some(self.epoch.load(SeqCst))
        })();
        self.unlock_flush(locked);
        result
    }

    pub fn revision(&self) -> u64 {
        let tag = self.tag();
        flush_writers_for(tag);
        if let Some(epoch) = self.clean_epoch() {
            return epoch;
        }
        let locked = self.lock_flush();
        let epoch = if self.dirty.swap(false, SeqCst) {
            let next = self.epoch.fetch_add(1, SeqCst).wrapping_add(1);
            flush_writers_for(tag);
            if any_active(tag) || (self.overflow.load(SeqCst) & !RETAIN_BIT) != 0 {
                self.dirty.store(true, SeqCst);
            }
            next
        } else {
            self.epoch.load(SeqCst)
        };
        self.unlock_flush(locked);
        epoch
    }
}

pub struct FlightGuard<'a> {
    gate: &'a FlightGate,
    slot: Option<(u8, u8)>,
}

impl FlightGuard<'_> {
    #[inline(always)]
    pub fn mark_dirty(&self) {
        if !self.gate.dirty.load(Relaxed) {
            self.gate.dirty.store(true, Relaxed);
        }
    }

    pub fn leak_and_retain(mut self) {
        self.gate.dirty.store(true, SeqCst);
        if let Some((slot_idx, entry_idx)) = self.slot.take() {
            let _ = self.gate.overflow.fetch_update(SeqCst, SeqCst, |current| {
                let count = (current & !RETAIN_BIT)
                    .saturating_add(1)
                    .min(!RETAIN_BIT);
                Some(RETAIN_BIT | count)
            });
            SLOTS[slot_idx as usize].active[entry_idx as usize].store(0, SeqCst);
        } else {
            self.gate.overflow.fetch_or(RETAIN_BIT, SeqCst);
        }
        std::mem::forget(self);
    }
}

impl Drop for FlightGuard<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some((slot_idx, entry_idx)) = self.slot {
            leave_slot(slot_idx, entry_idx);
        } else {
            self.gate.overflow.fetch_sub(1, SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn flight_gate_tracks_nesting_epoch_and_cross_thread_quiescence() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gate = Arc::new(FlightGate::new(7));
        let initial = gate.quiet_epoch().unwrap();
        assert_eq!(initial, 7);
        assert_eq!(gate.active(), 0);
        let g1 = gate.enter(true);
        assert_eq!(gate.active(), 1);
        assert_eq!(gate.quiet_epoch(), None);
        let g2 = gate.enter(false);
        assert_eq!(gate.active(), 2);
        drop(g1);
        assert_eq!(gate.active(), 1);
        drop(g2);
        assert_eq!(gate.active(), 0);
        let after = gate.quiet_epoch().unwrap();
        assert!(after > initial);
        assert_eq!(gate.quiet_epoch(), Some(after));

        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let (gate, entered, release) = (Arc::clone(&gate), Arc::clone(&entered), Arc::clone(&release));
            std::thread::spawn(move || {
                let guard = gate.enter(true);
                entered.wait();
                release.wait();
                drop(guard);
            })
        };
        entered.wait();
        assert_eq!(gate.active(), 1);
        assert_eq!(gate.quiet_epoch(), None);
        release.wait();
        worker.join().unwrap();
        let final_epoch = gate.quiet_epoch().unwrap();
        assert!(final_epoch > after);
        assert_eq!(gate.quiet_epoch(), Some(final_epoch));

        gate.mark_dirty();
        let bumped = gate.quiet_epoch().unwrap();
        assert!(bumped > final_epoch);
    }

    #[test]
    fn flight_gate_retains_without_leaking_thread_slot() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gate = FlightGate::new(0);
        let guard = gate.enter(true);
        guard.leak_and_retain();
        assert_ne!(gate.active() & RETAIN_BIT, 0);
        assert_ne!(gate.active() & !RETAIN_BIT, 0);
        assert_eq!(gate.quiet_epoch(), None);
        let extra = gate.enter(false);
        drop(extra);
        assert_eq!(gate.active() & !RETAIN_BIT, 1);

        let concurrent = FlightGate::new(0);
        let slot_guard = concurrent.enter(false);
        let fillers: Vec<_> = (1..SLOT_DEPTH).map(|_| concurrent.enter(false)).collect();
        let overflow_guard = concurrent.enter(false);
        assert!(slot_guard.slot.is_some());
        assert!(overflow_guard.slot.is_none());
        drop(fillers);
        assert_eq!(concurrent.active(), 2);
        slot_guard.leak_and_retain();
        assert_eq!(concurrent.active(), RETAIN_BIT | 2);
        drop(overflow_guard);
        assert!(concurrent.is_retained());
        assert_eq!(concurrent.active(), RETAIN_BIT | 1);
        assert_eq!(concurrent.quiet_epoch(), None);

        for _ in 0..32 {
            let live = Arc::new(FlightGate::new(0));
            let entered = Arc::new(Barrier::new(2));
            let go = Arc::new(Barrier::new(2));
            let saw_zero = Arc::new(AtomicBool::new(false));
            let done = Arc::new(AtomicBool::new(false));
            let writer = {
                let (live, entered, go, done) = (
                    Arc::clone(&live),
                    Arc::clone(&entered),
                    Arc::clone(&go),
                    Arc::clone(&done),
                );
                std::thread::spawn(move || {
                    let guard = live.enter(false);
                    assert!(guard.slot.is_some());
                    entered.wait();
                    go.wait();
                    guard.leak_and_retain();
                    done.store(true, std::sync::atomic::Ordering::Release);
                })
            };
            entered.wait();
            assert_eq!(live.active(), 1);
            go.wait();
            while !done.load(Acquire) {
                if (live.active() & !RETAIN_BIT) == 0 {
                    saw_zero.store(true, Relaxed);
                }
            }
            writer.join().unwrap();
            assert!(!saw_zero.load(Relaxed));
            assert_eq!(live.active(), RETAIN_BIT | 1);
        }
    }

    #[test]
    fn clean_reader_waits_for_in_flight_flush_before_certifying_epoch() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gate = Arc::new(FlightGate::new(10));
        drop(gate.enter(true));
        let locked = gate.lock_flush();
        assert!(gate.dirty.swap(false, SeqCst));
        assert_eq!(gate.clean_epoch(), None);
        let started = Arc::new(Barrier::new(3));
        let rev_reader = {
            let (gate, started) = (Arc::clone(&gate), Arc::clone(&started));
            std::thread::spawn(move || {
                started.wait();
                gate.revision()
            })
        };
        let quiet_reader = {
            let (gate, started) = (Arc::clone(&gate), Arc::clone(&started));
            std::thread::spawn(move || {
                started.wait();
                gate.quiet_epoch()
            })
        };
        started.wait();
        gate.epoch.fetch_add(1, SeqCst);
        gate.unlock_flush(locked);
        assert_eq!(rev_reader.join().unwrap(), 11);
        assert_eq!(quiet_reader.join().unwrap(), Some(11));
        assert_eq!(gate.clean_epoch(), Some(11));
    }

    #[test]
    fn capture_quiescence_fast_path_breaks_immediately_on_slot_entry() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gate = FlightGate::new(3);
        let other = FlightGate::new(11);
        {
            let _held = gate.enter(false);
            let q = begin_capture_quiescence(|| true);
            assert!(q.active);
            assert!(gate.any_active());
            assert_eq!(gate.active(), 1);
            assert!(!other.any_active());
            assert_eq!(other.active(), 0);
            assert!(!flush_writers_for(other.tag()));
        }
        {
            let q = begin_capture_quiescence(|| true);
            assert!(q.active);
            assert!(!flush_writers());
            assert!(!flush_writers_for(gate.tag()));
            assert!(!gate.any_active());
            assert_eq!(gate.active(), 0);
            let guard = gate.enter(true);
            assert!(gate.any_active());
            assert_eq!(gate.active(), 1);
            assert_eq!(gate.quiet_epoch(), None);
            assert!(!other.any_active());
            assert!(!flush_writers_for(other.tag()));
            drop(guard);
            assert!(!gate.any_active());
            assert!(gate.quiet_epoch().unwrap() > 3);
            assert_eq!(other.quiet_epoch(), Some(11));
        }
    }
}
