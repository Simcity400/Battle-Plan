use crate::{
    decode::DecodeJob,
    sampling::{BUFFERS, Captured, FrameStage},
};
use std::{
    cell::UnsafeCell,
    io,
    ops::Index,
    ptr,
    sync::{
        Mutex, PoisonError,
        atomic::{
            AtomicBool, AtomicI64, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering,
        },
    },
};

pub const STEADY: usize = BUFFERS;

pub const AHEAD: usize = 1;

pub const MAX_LIVE_FRAMES: usize = 5;

pub const MAX_LIVE_BYTES: u64 = 384 * 1024 * 1024;

const SURPLUS_CAP: usize = STEADY + 2;

#[cfg(not(test))]
const SURPLUS_STREAK: u32 = 48;
#[cfg(test)]
const SURPLUS_STREAK: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HourFrame {
    Free(usize),
    Grow,
    Drop,
}

pub(crate) fn hour_frame(free: Option<usize>, spool_stalled: bool) -> HourFrame {
    match free {
        Some(index) => HourFrame::Free(index),
        None if spool_stalled => HourFrame::Drop,
        None => HourFrame::Grow,
    }
}

pub(crate) fn builds_ahead(free: usize, spool_stalled: bool) -> bool {
    free < AHEAD && !spool_stalled
}

const SEGMENTS: usize = 48;

const VACANT: u8 = 0;
const FREE: u8 = 1;
const HELD: u8 = 2;

const NONE: usize = usize::MAX;
const STUB: usize = usize::MAX - 1;
const DECODE: usize = 0;
const READY: usize = 1;

const STAGE_SHIFT: u32 = 56;
const STAGES: [FrameStage; 7] = [
    FrameStage::Free,
    FrameStage::Callback,
    FrameStage::Queued,
    FrameStage::Decoding,
    FrameStage::Ready,
    FrameStage::Spool,
    FrameStage::Recycle,
];

pub struct Slot {
    pub(crate) frame: AtomicPtr<Captured>,
    pub completed: AtomicI64,
    pub callback_tail: AtomicI64,
    pub bytes: AtomicU64,
    pub generation: AtomicU64,
    pub fence: AtomicU64,
    stage: AtomicU64,
    state: AtomicU8,
    returned: AtomicU64,
    ahead: AtomicBool,
    links: [AtomicUsize; 2],
    job: UnsafeCell<DecodeJob>,
}

impl Slot {
    pub(crate) fn load(&self, order: Ordering) -> *mut Captured {
        self.frame.load(order)
    }

    #[doc = "# Safety"]
    #[doc = "The frame must be occupied and may not be mutated or recycled during this borrow."]
    pub unsafe fn captured(&self) -> &Captured {
        unsafe { &*self.load(Ordering::Acquire) }
    }

    #[doc = "# Safety"]
    #[doc = "The caller must hold the occupied slot exclusively for the duration of this borrow."]
    #[allow(
        clippy::mut_from_ref,
        reason = "The frame owner has exclusive access to its slot"
    )]
    pub unsafe fn captured_mut(&self) -> &mut Captured {
        unsafe { &mut *self.load(Ordering::Acquire) }
    }

    fn new() -> Self {
        Self {
            frame: AtomicPtr::new(ptr::null_mut()),
            completed: AtomicI64::new(0),
            callback_tail: AtomicI64::new(0),
            bytes: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            fence: AtomicU64::new(0),
            stage: AtomicU64::new(0),
            state: AtomicU8::new(VACANT),
            returned: AtomicU64::new(0),
            ahead: AtomicBool::new(false),
            links: [AtomicUsize::new(NONE), AtomicUsize::new(NONE)],
            job: UnsafeCell::new(DecodeJob::default()),
        }
    }
}

struct Fifo {
    head: AtomicUsize,
    tail: Mutex<usize>,
    stub: AtomicUsize,
    len: AtomicUsize,
    link: usize,
}

impl Fifo {
    const fn new(link: usize) -> Self {
        Self {
            head: AtomicUsize::new(STUB),
            tail: Mutex::new(STUB),
            stub: AtomicUsize::new(NONE),
            len: AtomicUsize::new(0),
            link,
        }
    }
}

pub struct FramePool {
    segments: [AtomicPtr<Slot>; SEGMENTS],
    made: AtomicUsize,
    live: AtomicUsize,
    live_bytes: AtomicU64,
    free: AtomicUsize,
    returns: AtomicU64,
    grown: AtomicU64,
    grow_ns: AtomicU64,
    grow_max_ns: AtomicU64,
    built_ahead: AtomicU64,
    ahead_taken: AtomicU64,
    freed: AtomicU64,
    surplus_streak: AtomicU32,
    spool_stalled: AtomicBool,
    dropped_stalled: AtomicU64,
    decode: Fifo,
    ready: Fifo,
}

fn locate(index: usize) -> (usize, usize) {
    let group = index / STEADY + 1;
    let segment = (usize::BITS - 1 - group.leading_zeros()) as usize;
    (segment, index - STEADY * ((1 << segment) - 1))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gauge {
    pub live: usize,
    pub free: usize,
    pub in_flight: usize,
    pub in_flight_bytes: u64,
    pub live_bytes: u64,
    pub grown: u64,
    pub grow_ns: u64,
    pub grow_max_ns: u64,
    pub ahead_ready: usize,
    pub built_ahead: u64,
    pub ahead_taken: u64,
    pub freed: u64,
    pub dropped_stalled: u64,
}

impl FramePool {
    pub fn new() -> Self {
        Self {
            segments: [const { AtomicPtr::new(ptr::null_mut()) }; SEGMENTS],
            made: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            live_bytes: AtomicU64::new(0),
            free: AtomicUsize::new(0),
            returns: AtomicU64::new(0),
            grown: AtomicU64::new(0),
            grow_ns: AtomicU64::new(0),
            grow_max_ns: AtomicU64::new(0),
            built_ahead: AtomicU64::new(0),
            ahead_taken: AtomicU64::new(0),
            freed: AtomicU64::new(0),
            surplus_streak: AtomicU32::new(0),
            spool_stalled: AtomicBool::new(false),
            dropped_stalled: AtomicU64::new(0),
            decode: Fifo::new(DECODE),
            ready: Fifo::new(READY),
        }
    }

    pub fn slot(&self, index: usize) -> &Slot {
        let (segment, offset) = locate(index);
        let base = self.segments[segment].load(Ordering::Acquire);
        assert!(
            !base.is_null() && index < self.made.load(Ordering::Acquire),
            "frame slot {index} was never made"
        );
        // SAFETY: a made segment holds `STEADY << segment` slots and is freed only on drop.
        unsafe { &*base.add(offset) }
    }

    pub fn made(&self) -> usize {
        self.made.load(Ordering::Acquire)
    }

    pub fn slots(&self) -> impl Iterator<Item = &Slot> {
        (0..self.made()).map(|index| self.slot(index))
    }

    pub(crate) fn slot_bytes(&self) -> u64 {
        self.segments
            .iter()
            .take_while(|segment| !segment.load(Ordering::Acquire).is_null())
            .enumerate()
            .map(|(segment, _)| (STEADY << segment) * size_of::<Slot>())
            .sum::<usize>() as u64
    }

    pub fn live(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    pub fn live_bytes(&self) -> u64 {
        self.live_bytes.load(Ordering::Acquire)
    }

    pub fn at_limit(&self) -> bool {
        self.at_limit_with(MAX_LIVE_FRAMES, MAX_LIVE_BYTES)
    }

    pub(crate) fn at_limit_with(&self, max_frames: usize, max_bytes: u64) -> bool {
        let live = self.live();
        if live >= max_frames {
            return true;
        }
        live >= STEADY && self.live_bytes() >= max_bytes
    }

    pub(crate) fn set_slot_bytes(&self, index: usize, bytes: u64) {
        let previous = self.slot(index).bytes.swap(bytes, Ordering::Relaxed);
        self.live_bytes
            .fetch_add(bytes.wrapping_sub(previous), Ordering::Release);
    }

    pub fn len(&self) -> usize {
        self.free.load(Ordering::Acquire)
    }

    /// # Safety
    /// Claim a slot only as this frame pool's admitted capture or sizing owner. Finish or return
    /// the installed slot before any other owner can use it.
    pub unsafe fn pop(&self) -> Option<usize> {
        let previous = self
            .free
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |free| {
                free.checked_sub(1)
            })
            .ok()?;
        if previous <= AHEAD {
            self.surplus_streak.store(0, Ordering::Relaxed);
        }
        loop {
            let newest = (0..self.made())
                .filter(|&index| self.slot(index).state.load(Ordering::Acquire) == FREE)
                .max_by_key(|&index| self.slot(index).returned.load(Ordering::Relaxed));
            if let Some(index) = newest
                && self.claim(index)
            {
                return Some(index);
            }
            std::hint::spin_loop();
        }
    }

    fn claim(&self, index: usize) -> bool {
        self.slot(index)
            .state
            .compare_exchange(FREE, HELD, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn take(&self, index: usize, keep: usize) -> bool {
        if index >= self.made() || self.slot(index).state.load(Ordering::Acquire) != FREE {
            return false;
        }
        if self
            .free
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |free| {
                (free > keep).then(|| free - 1)
            })
            .is_err()
        {
            return false;
        }
        if self.claim(index) {
            return true;
        }
        self.free.fetch_add(1, Ordering::AcqRel);
        false
    }

    pub(crate) fn push(&self, index: usize) -> Result<(), usize> {
        if index >= self.made() {
            return Err(index);
        }
        let stamp = self.returns.fetch_add(1, Ordering::Relaxed) + 1;
        self.slot(index).returned.store(stamp, Ordering::Relaxed);
        self.restore(index)
    }

    pub(crate) fn restore(&self, index: usize) -> Result<(), usize> {
        if index >= self.made()
            || self.slot(index).frame.load(Ordering::Acquire).is_null()
            || self
                .slot(index)
                .state
                .compare_exchange(HELD, FREE, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(index);
        }
        self.free.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn vacant(&self) -> io::Result<usize> {
        loop {
            let made = self.made();
            for index in 0..made {
                if self
                    .slot(index)
                    .state
                    .compare_exchange(VACANT, HELD, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok(index);
                }
            }
            let (segment, offset) = locate(made);
            if offset == 0 && self.segments[segment].load(Ordering::Acquire).is_null() {
                let len = STEADY << segment;
                let mut slots = engine_allocation::vec(len)?;
                slots.extend(std::iter::repeat_with(Slot::new).take(len));
                let slots = Box::into_raw(slots.into_boxed_slice()).cast::<Slot>();
                if self.segments[segment]
                    .compare_exchange(ptr::null_mut(), slots, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    // SAFETY: the other caller's segment won; this one was never published.
                    drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(slots, len)) });
                }
            }
            if self
                .made
                .compare_exchange(made, made + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
                && self
                    .slot(made)
                    .state
                    .compare_exchange(VACANT, HELD, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return Ok(made);
            }
        }
    }

    pub(crate) fn install(&self, index: usize, frame: Box<Captured>) {
        let slot = self.slot(index);
        debug_assert!(slot.frame.load(Ordering::Relaxed).is_null());
        let bytes = frame.allocated_bytes() as u64;
        slot.bytes.store(bytes, Ordering::Relaxed);
        self.live_bytes.fetch_add(bytes, Ordering::Release);
        slot.generation.fetch_add(1, Ordering::Relaxed);
        slot.frame.store(Box::into_raw(frame), Ordering::Release);
        self.live.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn set_spool_stalled(&self, stalled: bool) {
        self.spool_stalled.store(stalled, Ordering::Release);
    }

    pub fn spool_stalled(&self) -> bool {
        self.spool_stalled.load(Ordering::Acquire)
    }

    pub(crate) fn drop_stalled(&self) {
        self.dropped_stalled.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stalled_drops(&self) -> u64 {
        self.dropped_stalled.load(Ordering::Relaxed)
    }

    pub fn grew(&self, ns: u64) {
        self.grown.fetch_add(1, Ordering::Relaxed);
        self.grow_ns.fetch_add(ns, Ordering::Relaxed);
        self.grow_max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    pub fn built_ahead(&self, index: usize) {
        self.slot(index).ahead.store(true, Ordering::Relaxed);
        self.built_ahead.fetch_add(1, Ordering::Relaxed);
    }

    pub fn taken(&self, index: usize) {
        if self.slot(index).ahead.swap(false, Ordering::Relaxed) {
            self.ahead_taken.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn abandon(&self, index: usize) {
        let slot = self.slot(index);
        debug_assert!(slot.frame.load(Ordering::Relaxed).is_null());
        slot.state.store(VACANT, Ordering::Release);
    }

    pub fn surplus(&self) -> bool {
        self.surplus_after(SURPLUS_STREAK)
    }

    fn surplus_after(&self, threshold: u32) -> bool {
        let live = self.live();
        if live <= STEADY || self.len() < AHEAD {
            self.surplus_streak.store(0, Ordering::Relaxed);
            return false;
        }
        if live > SURPLUS_CAP || self.live_bytes() > MAX_LIVE_BYTES {
            self.surplus_streak.store(0, Ordering::Relaxed);
            return true;
        }
        let streak = self.surplus_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak >= threshold {
            self.surplus_streak.store(0, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// # Safety
    /// The caller must exclusively own an occupied slot in this pool, with no outstanding users,
    /// and may vacate it only once.
    pub unsafe fn vacate(&self, index: usize) -> Box<Captured> {
        let slot = self.slot(index);
        let frame = slot.frame.swap(ptr::null_mut(), Ordering::AcqRel);
        assert!(!frame.is_null(), "a vacated slot held a frame");
        let previous = slot.bytes.swap(0, Ordering::Relaxed);
        self.live_bytes.fetch_sub(previous, Ordering::Release);
        slot.ahead.store(false, Ordering::Relaxed);
        self.live.fetch_sub(1, Ordering::AcqRel);
        self.freed.fetch_add(1, Ordering::Relaxed);
        slot.state.store(VACANT, Ordering::Release);
        // SAFETY: the frame was installed from a Box and its holder gives it up here.
        unsafe { Box::from_raw(frame) }
    }

    pub fn gauge(&self) -> Gauge {
        let mut gauge = Gauge {
            live: self.live(),
            free: self.len(),
            grown: self.grown.load(Ordering::Relaxed),
            grow_ns: self.grow_ns.load(Ordering::Relaxed),
            grow_max_ns: self.grow_max_ns.load(Ordering::Relaxed),
            built_ahead: self.built_ahead.load(Ordering::Relaxed),
            ahead_taken: self.ahead_taken.load(Ordering::Relaxed),
            freed: self.freed.load(Ordering::Relaxed),
            dropped_stalled: self.stalled_drops(),
            ..Gauge::default()
        };
        for slot in self.slots() {
            if slot.frame.load(Ordering::Acquire).is_null() {
                continue;
            }
            let bytes = slot.bytes.load(Ordering::Relaxed);
            gauge.live_bytes += bytes;
            match slot.state.load(Ordering::Acquire) {
                HELD => {
                    gauge.in_flight += 1;
                    gauge.in_flight_bytes += bytes;
                }
                FREE if slot.ahead.load(Ordering::Relaxed) => gauge.ahead_ready += 1,
                _ => {}
            }
        }
        gauge
    }

    pub fn stage(&self, index: usize, stage: FrameStage, now: u64) {
        let stamp = now & ((1 << STAGE_SHIFT) - 1);
        self.slot(index)
            .stage
            .store((stage as u64) << STAGE_SHIFT | stamp, Ordering::Relaxed);
    }

    pub fn stages(&self, now: u64) -> [(u32, u64); STAGES.len()] {
        let now = now & ((1 << STAGE_SHIFT) - 1);
        let mut stages = [(0, 0); STAGES.len()];
        for slot in self.slots() {
            if slot.frame.load(Ordering::Acquire).is_null() {
                continue;
            }
            let word = slot.stage.load(Ordering::Relaxed);
            let since = now.wrapping_sub(word & ((1 << STAGE_SHIFT) - 1));
            let stage = &mut stages[(word >> STAGE_SHIFT) as usize % STAGES.len()];
            stage.0 += 1;
            stage.1 = stage.1.max(since);
        }
        stages
    }

    pub const STAGES: [FrameStage; 7] = STAGES;

    /// The decode job of slot `index`.
    ///
    /// # Safety
    /// The caller holds `index`: no other thread touches its job, and the caller makes no other
    /// borrow of it while the returned one lives.
    #[allow(
        clippy::mut_from_ref,
        reason = "the holder of the index owns the job alone"
    )]
    pub unsafe fn job(&self, index: usize) -> &mut DecodeJob {
        unsafe { &mut *self.slot(index).job.get() }
    }

    /// # Safety
    /// Only the admitted producer may publish this installed slot to decode, with its initialized
    /// job ready and exclusively owned.
    pub unsafe fn push_decode(&self, index: usize) {
        self.enqueue(&self.decode, index);
    }

    /// # Safety
    /// Only the designated decoder may remove from this session's decode queue. It assumes
    /// responsibility for the slot until settlement.
    pub unsafe fn pop_decode(&self) -> Option<usize> {
        self.pop_from(&self.decode)
    }

    #[cfg(test)]
    pub(crate) fn decode_len(&self) -> usize {
        self.decode.len.load(Ordering::Acquire)
    }

    pub(crate) fn push_ready(&self, index: usize) {
        self.enqueue(&self.ready, index);
    }

    /// # Safety
    /// Only the originating spool may take a decoded frame, and must exclusively hold the slot
    /// until recycling.
    pub unsafe fn pop_ready(&self) -> Option<usize> {
        self.pop_from(&self.ready)
    }

    pub fn ready_len(&self) -> usize {
        self.ready.len.load(Ordering::Acquire)
    }

    pub fn ready_is_empty(&self) -> bool {
        self.ready_len() == 0
    }

    fn next<'a>(&'a self, fifo: &'a Fifo, node: usize) -> &'a AtomicUsize {
        if node == STUB {
            &fifo.stub
        } else {
            &self.slot(node).links[fifo.link]
        }
    }

    fn enqueue(&self, fifo: &Fifo, index: usize) {
        fifo.len.fetch_add(1, Ordering::AcqRel);
        self.link(fifo, index);
    }

    fn link(&self, fifo: &Fifo, node: usize) {
        self.next(fifo, node).store(NONE, Ordering::Relaxed);
        let previous = fifo.head.swap(node, Ordering::AcqRel);
        self.next(fifo, previous).store(node, Ordering::Release);
    }

    fn pop_from(&self, fifo: &Fifo) -> Option<usize> {
        let mut tail = fifo.tail.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node = *tail;
        let mut next = self.next(fifo, node).load(Ordering::Acquire);
        if node == STUB {
            if next == NONE {
                return None;
            }
            *tail = next;
            node = next;
            next = self.next(fifo, node).load(Ordering::Acquire);
        }
        if next == NONE {
            if node != fifo.head.load(Ordering::Acquire) {
                return None;
            }
            self.link(fifo, STUB);
            next = self.next(fifo, node).load(Ordering::Acquire);
            if next == NONE {
                return None;
            }
        }
        *tail = next;
        fifo.len.fetch_sub(1, Ordering::AcqRel);
        Some(node)
    }
}

impl Index<usize> for FramePool {
    type Output = Slot;

    fn index(&self, index: usize) -> &Self::Output {
        self.slot(index)
    }
}

impl Drop for FramePool {
    fn drop(&mut self) {
        let made = *self.made.get_mut();
        for (segment, base) in self.segments.iter_mut().enumerate() {
            let base = *base.get_mut();
            if base.is_null() {
                break;
            }
            let len = STEADY << segment;
            // SAFETY: `vacant` leaked exactly this boxed slice.
            let mut slots = unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(base, len)) };
            let first = STEADY * ((1 << segment) - 1);
            for (offset, slot) in slots.iter_mut().enumerate() {
                let frame = *slot.frame.get_mut();
                if first + offset < made && !frame.is_null() {
                    // SAFETY: an installed frame came from a Box and no holder remains.
                    unsafe { drop(Box::from_raw(frame)) };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
