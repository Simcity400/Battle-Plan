//! The capture frames and the two FIFOs they travel through. An hour that finds no frame
//! free gets a new one instead of waiting; while the spool is behind, the frame builder
//! thread keeps the next hour's frame and a spare built ahead, so that is rare. Frames beyond
//! the steady count are freed as the spool catches up. A frame index names a slot that never moves, so a
//! queue, a fence or a stage holds only the index.

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
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};

/// The frames the pool returns to once the spool has caught up.
pub(crate) const STEADY: usize = BUFFERS;

/// The free frames kept ready while the spool is behind: the next hour's and a spare, so a frame
/// built ahead has an hour's start on the hour that takes it.
pub(crate) const AHEAD: usize = 2;

/// Segment `k` holds `STEADY << k` slots: more than an address space can hold frames for.
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

/// One frame's place: its frame while it has one, and what travels with its index.
pub(crate) struct Slot {
    pub(crate) frame: AtomicPtr<Captured>,
    pub(crate) completed: AtomicI64,
    pub(crate) callback_tail: AtomicI64,
    /// The frame's measured bytes, stored by its exclusive holder: a gauge, never a delta.
    pub(crate) bytes: AtomicU64,
    /// Counts the frames this slot has held, so per-frame state kept elsewhere sees a new one.
    pub(crate) generation: AtomicU64,
    /// The capture sequence of the hour the frame holds, zero once recycled.
    pub(crate) fence: AtomicU64,
    stage: AtomicU64,
    state: AtomicU8,
    returned: AtomicU64,
    /// The frame was built ahead, off the game thread, and no hour has taken it yet.
    ahead: AtomicBool,
    links: [AtomicUsize; 2],
    /// The hour's decode job, filled in place by the frame's holder.
    job: UnsafeCell<DecodeJob>,
}

impl Slot {
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

/// An unbounded FIFO of slot indices linked through the slots themselves: pushing never
/// allocates, never fails and never waits. Producers are lock-free; consumers take turns.
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

/// Every frame slot, the free set, and the decode and ready FIFOs.
pub(crate) struct FramePool {
    segments: [AtomicPtr<Slot>; SEGMENTS],
    made: AtomicUsize,
    live: AtomicUsize,
    free: AtomicUsize,
    returns: AtomicU64,
    grown: AtomicU64,
    grow_ns: AtomicU64,
    grow_max_ns: AtomicU64,
    built_ahead: AtomicU64,
    ahead_taken: AtomicU64,
    freed: AtomicU64,
    decode: Fifo,
    ready: Fifo,
}

/// Where slot `index` lives: its segment and its place there.
fn locate(index: usize) -> (usize, usize) {
    let group = index / STEADY + 1;
    let segment = (usize::BITS - 1 - group.leading_zeros()) as usize;
    (segment, index - STEADY * ((1 << segment) - 1))
}

/// What a frame pool reports in the collector's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gauge {
    pub live: usize,
    pub free: usize,
    /// Frames taken for an hour and not yet returned by the spool.
    pub in_flight: usize,
    pub in_flight_bytes: u64,
    /// Every live frame's bytes, free or not.
    pub live_bytes: u64,
    /// Frames the game thread built itself because none was free, and their walls.
    pub grown: u64,
    pub grow_ns: u64,
    pub grow_max_ns: u64,
    /// Free frames built ahead off the game thread that no hour has taken yet.
    pub ahead_ready: usize,
    pub built_ahead: u64,
    pub ahead_taken: u64,
    pub freed: u64,
}

impl FramePool {
    pub(crate) fn new() -> Self {
        Self {
            segments: [const { AtomicPtr::new(ptr::null_mut()) }; SEGMENTS],
            made: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            free: AtomicUsize::new(0),
            returns: AtomicU64::new(0),
            grown: AtomicU64::new(0),
            grow_ns: AtomicU64::new(0),
            grow_max_ns: AtomicU64::new(0),
            built_ahead: AtomicU64::new(0),
            ahead_taken: AtomicU64::new(0),
            freed: AtomicU64::new(0),
            decode: Fifo::new(DECODE),
            ready: Fifo::new(READY),
        }
    }

    /// Slot `index`, which must have been made.
    pub(crate) fn slot(&self, index: usize) -> &Slot {
        let (segment, offset) = locate(index);
        let base = self.segments[segment].load(Ordering::Acquire);
        assert!(
            !base.is_null() && index < self.made.load(Ordering::Acquire),
            "frame slot {index} was never made"
        );
        // SAFETY: a made segment holds `STEADY << segment` slots and is freed only on drop.
        unsafe { &*base.add(offset) }
    }

    /// Slots made so far, frames or not.
    pub(crate) fn made(&self) -> usize {
        self.made.load(Ordering::Acquire)
    }

    pub(crate) fn slots(&self) -> impl Iterator<Item = &Slot> {
        (0..self.made()).map(|index| self.slot(index))
    }

    /// The bytes of every segment of slots made, jobs included.
    pub(crate) fn slot_bytes(&self) -> u64 {
        self.segments
            .iter()
            .take_while(|segment| !segment.load(Ordering::Acquire).is_null())
            .enumerate()
            .map(|(segment, _)| (STEADY << segment) * size_of::<Slot>())
            .sum::<usize>() as u64
    }

    /// Frames allocated now.
    pub(crate) fn live(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// Free frames.
    pub fn len(&self) -> usize {
        self.free.load(Ordering::Acquire)
    }

    /// The newest free frame, held by the caller.
    pub fn pop(&self) -> Option<usize> {
        self.free
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |free| free.checked_sub(1))
            .ok()?;
        // A free frame's state is set before the count and cleared after it, so the count
        // just taken names a free slot no other taker holds.
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

    /// Takes `index` if it is free and `keep` other frames stay free.
    pub fn take(&self, index: usize, keep: usize) -> bool {
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

    /// Frees a held frame as the newest.
    pub fn push(&self, index: usize) -> Result<(), usize> {
        if index >= self.made() {
            return Err(index);
        }
        let stamp = self.returns.fetch_add(1, Ordering::Relaxed) + 1;
        self.slot(index).returned.store(stamp, Ordering::Relaxed);
        self.restore(index)
    }

    /// Frees a held frame again, as old as it was when taken.
    pub fn restore(&self, index: usize) -> Result<(), usize> {
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

    /// A held slot for a new frame: a vacant one, else a new one. The game thread and the
    /// thread building frames ahead may both call it: neither ever waits on the other.
    pub(crate) fn vacant(&self) -> io::Result<usize> {
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
            // A new slot starts vacant, so whichever caller claims it first holds it.
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

    /// Gives the held slot `index` a new frame.
    pub(crate) fn install(&self, index: usize, frame: Box<Captured>) {
        let slot = self.slot(index);
        debug_assert!(slot.frame.load(Ordering::Relaxed).is_null());
        slot.bytes
            .store(frame.allocated_bytes() as u64, Ordering::Relaxed);
        slot.generation.fetch_add(1, Ordering::Relaxed);
        slot.frame.store(Box::into_raw(frame), Ordering::Release);
        self.live.fetch_add(1, Ordering::AcqRel);
    }

    /// Records an hour's frame allocation and its wall.
    pub(crate) fn grew(&self, ns: u64) {
        self.grown.fetch_add(1, Ordering::Relaxed);
        self.grow_ns.fetch_add(ns, Ordering::Relaxed);
        self.grow_max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// Marks the held slot `index`'s new frame as built ahead of any hour's need.
    pub(crate) fn built_ahead(&self, index: usize) {
        self.slot(index).ahead.store(true, Ordering::Relaxed);
        self.built_ahead.fetch_add(1, Ordering::Relaxed);
    }

    /// The game thread took `index`: counts it if it was built ahead.
    pub(crate) fn taken(&self, index: usize) {
        if self.slot(index).ahead.swap(false, Ordering::Relaxed) {
            self.ahead_taken.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A held slot whose frame could not be allocated goes back to vacant.
    pub(crate) fn abandon(&self, index: usize) {
        let slot = self.slot(index);
        debug_assert!(slot.frame.load(Ordering::Relaxed).is_null());
        slot.state.store(VACANT, Ordering::Release);
    }

    /// Whether a frame returned now is beyond what the pool keeps: more than the steady count
    /// live while more than the frames kept ahead are free. One frame between this and the
    /// building ahead keeps a backlog that sways by an hour from freeing and building each hour.
    pub(crate) fn surplus(&self) -> bool {
        self.live() > STEADY && self.len() > AHEAD
    }

    /// Takes the frame of the held slot `index` and leaves the slot vacant.
    pub(crate) fn vacate(&self, index: usize) -> Box<Captured> {
        let slot = self.slot(index);
        let frame = slot.frame.swap(ptr::null_mut(), Ordering::AcqRel);
        assert!(!frame.is_null(), "a vacated slot held a frame");
        slot.bytes.store(0, Ordering::Relaxed);
        slot.ahead.store(false, Ordering::Relaxed);
        self.live.fetch_sub(1, Ordering::AcqRel);
        self.freed.fetch_add(1, Ordering::Relaxed);
        slot.state.store(VACANT, Ordering::Release);
        // SAFETY: the frame was installed from a Box and its holder gives it up here.
        unsafe { Box::from_raw(frame) }
    }

    /// Every held slot's frame bytes and count, and the pool's counters.
    pub(crate) fn gauge(&self) -> Gauge {
        let mut gauge = Gauge {
            live: self.live(),
            free: self.len(),
            grown: self.grown.load(Ordering::Relaxed),
            grow_ns: self.grow_ns.load(Ordering::Relaxed),
            grow_max_ns: self.grow_max_ns.load(Ordering::Relaxed),
            built_ahead: self.built_ahead.load(Ordering::Relaxed),
            ahead_taken: self.ahead_taken.load(Ordering::Relaxed),
            freed: self.freed.load(Ordering::Relaxed),
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

    pub(crate) fn stage(&self, index: usize, stage: FrameStage, now: u64) {
        let stamp = now & ((1 << STAGE_SHIFT) - 1);
        self.slot(index)
            .stage
            .store((stage as u64) << STAGE_SHIFT | stamp, Ordering::Relaxed);
    }

    /// Per stage, the frames in it and the longest any has been there, in QPC ticks.
    pub(crate) fn stages(&self, now: u64) -> [(u32, u64); STAGES.len()] {
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

    pub(crate) const STAGES: [FrameStage; 7] = STAGES;

    /// The decode job of slot `index`.
    ///
    /// # Safety
    /// The caller holds `index`: no other thread touches its job.
    #[allow(clippy::mut_from_ref, reason = "the holder of the index owns the job alone")]
    pub(crate) unsafe fn job(&self, index: usize) -> &mut DecodeJob {
        // SAFETY: forwarded from the caller.
        unsafe { &mut *self.slot(index).job.get() }
    }

    pub(crate) fn push_decode(&self, index: usize) {
        self.enqueue(&self.decode, index);
    }

    pub(crate) fn pop_decode(&self) -> Option<usize> {
        self.pop_from(&self.decode)
    }

    #[cfg(test)]
    pub(crate) fn decode_len(&self) -> usize {
        self.decode.len.load(Ordering::Acquire)
    }

    pub(crate) fn push_ready(&self, index: usize) {
        self.enqueue(&self.ready, index);
    }

    pub(crate) fn pop_ready(&self) -> Option<usize> {
        self.pop_from(&self.ready)
    }

    pub(crate) fn ready_len(&self) -> usize {
        self.ready.len.load(Ordering::Acquire)
    }

    pub(crate) fn ready_is_empty(&self) -> bool {
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

    /// The oldest index, or none while the FIFO is empty or its newest push is still linking.
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
    type Output = AtomicPtr<Captured>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.slot(index).frame
    }
}

impl Drop for FramePool {
    fn drop(&mut self) {
        // The loader drops the pool only after callback quiescence and the spool's join.
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
