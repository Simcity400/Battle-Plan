//! The sole game-thread callback. Buffers and loss witnesses are allocated before hooking.

use crate::roster::Roster;
use crossbeam_queue::ArrayQueue;
use engine::{
    ccountry,
    cgame_state::{self, Clock},
    cprovince,
    memory::LiveMemory,
    pdx::Text,
    walk::Outcome,
};
use std::{
    cell::UnsafeCell,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

pub const BUFFERS: usize = 256;
const LOSS_RUNS: usize = 64;
pub static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct LostObservation {
    pub campaign_id: Text<128>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub loaded_hour: Option<i32>,
    pub load_signal: Option<i64>,
    pub count: u64,
}

#[derive(Clone, Copy)]
pub struct LossBatch {
    items: [LostObservation; LOSS_RUNS],
    len: usize,
    pub unknown_count: u64,
}

impl Default for LossBatch {
    fn default() -> Self {
        Self {
            items: [LostObservation::default(); LOSS_RUNS],
            len: 0,
            unknown_count: 0,
        }
    }
}

impl LossBatch {
    pub fn iter(&self) -> std::slice::Iter<'_, LostObservation> {
        self.items[..self.len].iter()
    }

    fn push(&mut self, entry: Option<Clock>, after: Option<Clock>) {
        let Some(after) = after else {
            self.unknown_count += 1;
            return;
        };
        if let Some(last) = self.items[..self.len].last_mut()
            && last.campaign_id == after.campaign_id
            && last.load_signal == Some(after.play_session_started_at)
            && last.last_hour.checked_add(1) == Some(after.hour)
        {
            last.last_hour = after.hour;
            last.count += 1;
            return;
        }
        if self.len == LOSS_RUNS {
            self.unknown_count += 1;
            return;
        }
        self.items[self.len] = LostObservation {
            campaign_id: after.campaign_id,
            first_hour: after.hour,
            last_hour: after.hour,
            loaded_hour: entry
                .filter(|entry| entry.campaign_id == after.campaign_id)
                .map(|entry| entry.hour),
            load_signal: Some(after.play_session_started_at),
            count: 1,
        };
        self.len += 1;
    }
}

pub struct Captured {
    pub clock_entry: Option<Clock>,
    pub clock_before: Option<Clock>,
    pub clock_after: Option<Clock>,
    pub countries: Vec<ccountry::Row>,
    pub provinces: Vec<cprovince::Row>,
    pub links: Vec<ccountry::LinkRow>,
    links_second: Vec<ccountry::LinkRow>,
    pub countries_result: Outcome,
    pub provinces_result: Outcome,
    pub links_result: Outcome,
    pub links_mismatches: usize,
    pub table_ns: [u64; 5],
    pub total_ns: u64,
    pub qpc: i64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
    pub dropped_before: LossBatch,
}

impl Captured {
    pub(crate) fn new() -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        Self {
            clock_entry: None,
            clock_before: None,
            clock_after: None,
            countries: vec![ccountry::Row::default(); 512],
            provinces: vec![cprovince::Row::default(); 20_000],
            links: vec![ccountry::LinkRow::default(); 1024],
            links_second: vec![ccountry::LinkRow::default(); 1024],
            countries_result: unavailable,
            provinces_result: unavailable,
            links_result: unavailable,
            links_mismatches: 0,
            table_ns: [0; 5],
            total_ns: 0,
            qpc: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            dropped_before: LossBatch::default(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + self.provinces.capacity() * size_of::<cprovince::Row>()
            + (self.links.capacity() + self.links_second.capacity())
                * size_of::<ccountry::LinkRow>()
    }

    /// Called only by the spool thread while it owns this buffer's queue index.
    pub fn grow(&mut self, capacities: &mut [usize; 3]) {
        fn resize<T: Default + Clone>(rows: &mut Vec<T>, outcome: Outcome, capacity: &mut usize) {
            if let Outcome::Truncated { len, unread_rows } = outcome {
                *capacity = (*capacity).max(len.saturating_add(unread_rows as usize));
            }
            if rows.len() < *capacity {
                rows.resize(*capacity, T::default());
            }
        }
        resize(
            &mut self.countries,
            self.countries_result,
            &mut capacities[0],
        );
        resize(
            &mut self.provinces,
            self.provinces_result,
            &mut capacities[1],
        );
        resize(&mut self.links, self.links_result, &mut capacities[2]);
        if self.links_second.len() < capacities[2] {
            self.links_second
                .resize(capacities[2], ccountry::LinkRow::default());
        }
    }
}

pub struct Shared {
    pub base: usize,
    pub trampoline: usize,
    pub frequency: u64,
    pub frames: Vec<UnsafeCell<Captured>>,
    pub completed: Vec<AtomicI64>,
    pub free: ArrayQueue<usize>,
    pub ready: ArrayQueue<usize>,
    pub dropped: AtomicU64,
    pub pool_bytes: AtomicU64,
    pub recording: AtomicBool,
    pub stop_spool: AtomicBool,
    /// The sole callback owns this until ready is published; the spool thread
    /// then takes it once. It is independent of any particular FIFO buffer.
    pub roster: UnsafeCell<Option<Box<Roster>>>,
    pub roster_ready: AtomicBool,
    /// Only the callback writes this. The worker may take it after callback quiescence.
    pub pending: UnsafeCell<LossBatch>,
}

// Each frame index has exactly one owner: a queue, the sole callback, or the spool thread.
// The pending loss batch has one game-thread writer until the restored hook is quiescent.
unsafe impl Sync for Shared {}

impl Shared {
    pub fn new(base: usize, trampoline: usize, frequency: u64) -> Self {
        let frames: Vec<_> = (0..BUFFERS)
            .map(|_| UnsafeCell::new(Captured::new()))
            .collect();
        let roster = Box::new(Roster::new());
        let pool_bytes = frames
            .iter()
            .map(|frame| unsafe { &*frame.get() }.allocated_bytes())
            .sum::<usize>()
            + roster.allocated_bytes();
        let free = ArrayQueue::new(BUFFERS);
        for index in 0..BUFFERS {
            free.push(index).unwrap();
        }
        Self {
            base,
            trampoline,
            frequency,
            frames,
            completed: (0..BUFFERS).map(|_| AtomicI64::new(0)).collect(),
            free,
            ready: ArrayQueue::new(BUFFERS),
            dropped: AtomicU64::new(0),
            pool_bytes: AtomicU64::new(pool_bytes as u64),
            recording: AtomicBool::new(true),
            stop_spool: AtomicBool::new(false),
            roster: UnsafeCell::new(Some(roster)),
            roster_ready: AtomicBool::new(false),
            pending: UnsafeCell::new(LossBatch::default()),
        }
    }

    pub fn ns(&self, ticks: i64) -> u64 {
        ((ticks.max(0) as u128 * 1_000_000_000) / self.frequency as u128) as u64
    }

    /// Called only by the game thread. A failed clock postpones this once-only
    /// read until the next readable callback, even when no row buffer is free.
    unsafe fn sample_roster(&self, memory: &LiveMemory, clock: Option<Clock>) {
        if self.roster_ready.load(Ordering::Acquire) {
            return;
        }
        if let Some(clock) = clock {
            let roster = unsafe { (&mut *self.roster.get()).as_mut().unwrap() };
            roster.sample(memory, self.base, Some(clock.hour));
            self.roster_ready.store(roster.sampled(), Ordering::Release);
        }
    }
}

pub fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe {
        QueryPerformanceCounter(&mut ticks);
    }
    ticks
}

pub unsafe extern "system" fn tick(state: usize, commands: usize) {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let entered = qpc();
    let shared = unsafe { &*SHARED.load(Ordering::Acquire) };
    let recording = shared.recording.load(Ordering::Relaxed);
    let memory = unsafe { LiveMemory::new() };
    let clock_start = qpc();
    let entry = if recording {
        cgame_state::read_clock(&memory, shared.base)
    } else {
        None
    };
    let entry_ticks = qpc() - clock_start;
    let index = if recording { shared.free.pop() } else { None };
    let original_begin = qpc();
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(shared.trampoline) };
    unsafe {
        original(state, commands);
    }
    let original_end = qpc();
    if recording {
        if let Some(index) = index {
            let frame = unsafe { &mut *shared.frames[index].get() };
            frame.clock_entry = entry;
            frame.qpc = entered;
            frame.table_ns = [0; 5];
            let before = qpc();
            frame.clock_before = cgame_state::read_clock(&memory, shared.base);
            let mut clock_ticks = entry_ticks + qpc() - before;
            let unavailable = Outcome::Unavailable {
                reason: "clock unavailable",
            };
            frame.countries_result = unavailable;
            frame.provinces_result = unavailable;
            frame.links_result = unavailable;
            frame.links_mismatches = 0;
            if let Some(clock) = frame.clock_before {
                let state = clock.game_state as usize;
                let begin = qpc();
                frame.countries_result =
                    ccountry::walk(&memory, shared.base, state, &mut frame.countries);
                frame.table_ns[2] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.links_result =
                    ccountry::walk_links(&memory, shared.base, state, &mut frame.links);
                let second =
                    ccountry::walk_links(&memory, shared.base, state, &mut frame.links_second);
                frame.links_mismatches = if frame.links_result != second {
                    1
                } else {
                    frame.links[..second.len()]
                        .iter()
                        .zip(&frame.links_second[..second.len()])
                        .filter(|(a, b)| a != b)
                        .count()
                };
                frame.table_ns[3] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.provinces_result =
                    cprovince::walk(&memory, shared.base, state, &mut frame.provinces);
                frame.table_ns[4] = shared.ns(qpc() - begin);
            }
            let after = qpc();
            frame.clock_after = cgame_state::read_clock(&memory, shared.base);
            clock_ticks += qpc() - after;
            unsafe {
                shared.sample_roster(&memory, frame.clock_after.or(frame.clock_before));
            }
            frame.table_ns[0] = shared.ns(clock_ticks);
            frame.dropped_before = unsafe { std::mem::take(&mut *shared.pending.get()) };
            frame.pool_bytes = shared.pool_bytes.load(Ordering::Relaxed);
            frame.dropped_frames = shared.dropped.load(Ordering::Relaxed);
            shared.completed[index].store(0, Ordering::Relaxed);
            // With 256 buffers and 256 queue slots, an owned buffer guarantees space.
            if shared.ready.push(index).is_ok() {
                shared.completed[index].store(
                    (qpc() - entered - (original_end - original_begin)).max(1),
                    Ordering::Release,
                );
            } else {
                unsafe {
                    *shared.pending.get() = frame.dropped_before;
                    (&mut *shared.pending.get()).push(entry, frame.clock_after);
                }
                shared.dropped.fetch_add(1, Ordering::Relaxed);
                let _ = shared.free.push(index);
            }
        } else {
            let after = cgame_state::read_clock(&memory, shared.base);
            unsafe {
                shared.sample_roster(&memory, after);
                (&mut *shared.pending.get()).push(entry, after);
            }
            shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clock(hour: i32, signal: i64) -> Clock {
        Clock {
            game_state: 1,
            hour,
            play_time: 0,
            play_session_started_at: signal,
            seed: 1,
            campaign_id: Text::default(),
            speed: 4,
            tick_busy: 0,
        }
    }
    #[test]
    fn lost_ranges_preserve_reload_origins_and_never_bridge_discontinuities() {
        let mut batch = LossBatch::default();
        batch.push(Some(clock(10, 1)), Some(clock(11, 1)));
        batch.push(Some(clock(11, 1)), Some(clock(12, 1)));
        batch.push(Some(clock(5, 2)), Some(clock(6, 2)));
        batch.push(Some(clock(8, 2)), Some(clock(9, 2)));
        let runs: Vec<_> = batch.iter().collect();
        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].count),
            (11, 12, 2)
        );
        assert_eq!(runs[1].loaded_hour, Some(5));
        batch.push(None, None);
        assert_eq!(batch.unknown_count, 1);
    }
}
