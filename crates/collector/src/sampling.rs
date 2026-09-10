//! The sole game-thread callback. Buffers and loss witnesses are allocated before hooking.

use crate::roster::Roster;
use crossbeam_queue::ArrayQueue;
use engine::{
    carmy, ccountry, cfaction, cfaction_system,
    cgame_state::{self, Clock},
    cprovince, cstate, cwar_relation,
    memory::LiveMemory,
    msvc_string::Text,
    walk::Outcome,
};
use replay::frame::TABLE_COUNT;
use std::{
    cell::UnsafeCell,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

pub const BUFFERS: usize = 256;
pub const CONSISTENCY_TARGET: u16 = 200;
/// These indices describe comparison scheduling, not the replay's table indices.
pub const CONSISTENCY_TABLES: [&str; 6] = [
    "CountryLinks",
    "States",
    "Divisions",
    "WarRelations",
    "Factions",
    "FactionMembers",
];
const LOSS_RUNS: usize = 64;
pub static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn mismatches<R: PartialEq>(first: Outcome, rows: &[R], second: Outcome, other: &[R]) -> usize {
    if first != second {
        return 1;
    }
    rows[..first.len()]
        .iter()
        .zip(&other[..second.len()])
        .filter(|(a, b)| a != b)
        .count()
}

fn full_match(first: Outcome, second: Outcome, mismatches: usize) -> bool {
    matches!(first, Outcome::Rows { .. }) && first == second && mismatches == 0
}

pub(crate) fn stable_clock(before: Option<Clock>, after: Option<Clock>) -> bool {
    matches!((before, after), (Some(before), Some(after))
        if before.hour == after.hour
            && before.game_state == after.game_state
            && before.campaign_id == after.campaign_id
            && before.play_session_started_at == after.play_session_started_at)
}

#[derive(Default)]
struct ConsistencyWindow {
    successful: [u16; CONSISTENCY_TABLES.len()],
}

impl ConsistencyWindow {
    fn needed(&self) -> [bool; CONSISTENCY_TABLES.len()] {
        self.successful.map(|count| count < CONSISTENCY_TARGET)
    }

    fn commit(
        &mut self,
        matched: [bool; CONSISTENCY_TABLES.len()],
        stable: bool,
        published: bool,
    ) -> [u16; CONSISTENCY_TABLES.len()] {
        if stable && published {
            for (count, matched) in self.successful.iter_mut().zip(matched) {
                if matched && *count < CONSISTENCY_TARGET {
                    *count += 1;
                }
            }
        }
        self.successful
    }
}

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
    pub states: Vec<cstate::Row>,
    states_second: Vec<cstate::Row>,
    pub divisions: Vec<carmy::Row>,
    divisions_second: Vec<carmy::Row>,
    pub wars: Vec<cwar_relation::Row>,
    wars_second: Vec<cwar_relation::Row>,
    pub wars_matrix: Vec<cwar_relation::Row>,
    pub factions: Vec<cfaction::Row>,
    factions_second: Vec<cfaction::Row>,
    pub faction_names: Vec<u8>,
    faction_names_second: Vec<u8>,
    pub faction_names_len: usize,
    pub faction_names_required: usize,
    pub faction_members: Vec<cfaction_system::MemberRow>,
    faction_members_second: Vec<cfaction_system::MemberRow>,
    pub links: Vec<ccountry::LinkRow>,
    links_second: Vec<ccountry::LinkRow>,
    pub countries_result: Outcome,
    pub provinces_result: Outcome,
    pub states_result: Outcome,
    pub states_mismatches: usize,
    pub divisions_result: Outcome,
    pub divisions_mismatches: usize,
    pub wars_result: Outcome,
    pub wars_mismatches: usize,
    pub war_matrix_result: Option<Outcome>,
    pub war_matrix_ns: u64,
    pub factions_result: Outcome,
    pub factions_mismatches: usize,
    pub faction_members_result: Outcome,
    pub faction_members_mismatches: usize,
    pub links_result: Outcome,
    pub links_mismatches: usize,
    pub consistency_checked: [bool; CONSISTENCY_TABLES.len()],
    pub consistency_successful: [u16; CONSISTENCY_TABLES.len()],
    pub table_ns: [u64; TABLE_COUNT],
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
            states: vec![cstate::Row::default(); 2048],
            states_second: vec![cstate::Row::default(); 2048],
            divisions: vec![carmy::Row::default(); 4096],
            divisions_second: vec![carmy::Row::default(); 4096],
            wars: vec![cwar_relation::Row::default(); 1024],
            wars_second: vec![cwar_relation::Row::default(); 1024],
            wars_matrix: vec![cwar_relation::Row::default(); 1024],
            factions: vec![cfaction::Row::default(); 64],
            factions_second: vec![cfaction::Row::default(); 64],
            faction_names: vec![0; 16_384],
            faction_names_second: vec![0; 16_384],
            faction_names_len: 0,
            faction_names_required: 0,
            faction_members: vec![cfaction_system::MemberRow::default(); 1024],
            faction_members_second: vec![cfaction_system::MemberRow::default(); 1024],
            links: vec![ccountry::LinkRow::default(); 1024],
            links_second: vec![ccountry::LinkRow::default(); 1024],
            countries_result: unavailable,
            provinces_result: unavailable,
            states_result: unavailable,
            states_mismatches: 0,
            divisions_result: unavailable,
            divisions_mismatches: 0,
            wars_result: unavailable,
            wars_mismatches: 0,
            war_matrix_result: None,
            war_matrix_ns: 0,
            factions_result: unavailable,
            factions_mismatches: 0,
            faction_members_result: unavailable,
            faction_members_mismatches: 0,
            links_result: unavailable,
            links_mismatches: 0,
            consistency_checked: [false; CONSISTENCY_TABLES.len()],
            consistency_successful: [0; CONSISTENCY_TABLES.len()],
            table_ns: [0; TABLE_COUNT],
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
            + (self.states.capacity() + self.states_second.capacity()) * size_of::<cstate::Row>()
            + (self.divisions.capacity() + self.divisions_second.capacity())
                * size_of::<carmy::Row>()
            + (self.wars.capacity() + self.wars_second.capacity() + self.wars_matrix.capacity())
                * size_of::<cwar_relation::Row>()
            + (self.factions.capacity() + self.factions_second.capacity())
                * size_of::<cfaction::Row>()
            + self.faction_names.capacity()
            + self.faction_names_second.capacity()
            + (self.faction_members.capacity() + self.faction_members_second.capacity())
                * size_of::<cfaction_system::MemberRow>()
            + (self.links.capacity() + self.links_second.capacity())
                * size_of::<ccountry::LinkRow>()
    }

    /// Called only by the spool thread while it owns this buffer's queue index.
    /// Completed comparison buffers are released here and never grown again.
    pub fn grow(&mut self, capacities: &mut [usize; 9]) {
        fn resize<T: Default + Clone>(rows: &mut Vec<T>, outcome: Outcome, capacity: &mut usize) {
            if let Outcome::Truncated { len, unread_rows } = outcome {
                *capacity = (*capacity).max(len.saturating_add(unread_rows as usize));
            }
            if rows.len() < *capacity {
                rows.resize(*capacity, T::default());
            }
        }
        fn comparison<T: Default + Clone>(rows: &mut Vec<T>, successful: u16, capacity: usize) {
            if successful >= CONSISTENCY_TARGET {
                *rows = Vec::new();
            } else if rows.len() < capacity {
                rows.resize(capacity, T::default());
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
        resize(&mut self.states, self.states_result, &mut capacities[3]);
        resize(
            &mut self.divisions,
            self.divisions_result,
            &mut capacities[4],
        );
        if let Some(Outcome::Truncated { len, unread_rows }) = self.war_matrix_result {
            capacities[5] = capacities[5].max(len.saturating_add(unread_rows as usize));
        }
        resize(&mut self.wars, self.wars_result, &mut capacities[5]);
        resize(&mut self.factions, self.factions_result, &mut capacities[6]);
        resize(
            &mut self.faction_members,
            self.faction_members_result,
            &mut capacities[7],
        );
        capacities[8] = capacities[8].max(self.faction_names_required);
        comparison(
            &mut self.links_second,
            self.consistency_successful[0],
            capacities[2],
        );
        comparison(
            &mut self.states_second,
            self.consistency_successful[1],
            capacities[3],
        );
        comparison(
            &mut self.divisions_second,
            self.consistency_successful[2],
            capacities[4],
        );
        comparison(
            &mut self.wars_second,
            self.consistency_successful[3],
            capacities[5],
        );
        self.wars_matrix
            .resize(capacities[5], cwar_relation::Row::default());
        comparison(
            &mut self.factions_second,
            self.consistency_successful[4],
            capacities[6],
        );
        comparison(
            &mut self.faction_members_second,
            self.consistency_successful[5],
            capacities[7],
        );
        self.faction_names.resize(capacities[8], 0);
        comparison(
            &mut self.faction_names_second,
            self.consistency_successful[4],
            capacities[8],
        );
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
    /// Only the sole game-thread callback updates these per-attach counts.
    consistency: UnsafeCell<ConsistencyWindow>,
    /// Independent matrix census supplements the owned-vector double read for 200 full walks.
    war_matrix_remaining: AtomicUsize,
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
            consistency: UnsafeCell::new(ConsistencyWindow::default()),
            war_matrix_remaining: AtomicUsize::new(200),
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
            frame.table_ns = [0; TABLE_COUNT];
            let before = qpc();
            frame.clock_before = cgame_state::read_clock(&memory, shared.base);
            let mut clock_ticks = entry_ticks + qpc() - before;
            let unavailable = Outcome::Unavailable {
                reason: "clock unavailable",
            };
            frame.countries_result = unavailable;
            frame.provinces_result = unavailable;
            frame.states_result = unavailable;
            frame.states_mismatches = 0;
            frame.divisions_result = unavailable;
            frame.divisions_mismatches = 0;
            frame.wars_result = unavailable;
            frame.wars_mismatches = 0;
            frame.war_matrix_result = None;
            frame.war_matrix_ns = 0;
            frame.factions_result = unavailable;
            frame.factions_mismatches = 0;
            frame.faction_names_len = 0;
            frame.faction_names_required = 0;
            frame.faction_members_result = unavailable;
            frame.faction_members_mismatches = 0;
            frame.links_result = unavailable;
            frame.links_mismatches = 0;
            frame.consistency_checked = [false; CONSISTENCY_TABLES.len()];
            let needed = unsafe { (&*shared.consistency.get()).needed() };
            let mut matched = [false; CONSISTENCY_TABLES.len()];
            let mut matrix_complete = false;
            if let Some(clock) = frame.clock_before {
                let state = clock.game_state as usize;
                let begin = qpc();
                frame.countries_result =
                    ccountry::walk(&memory, shared.base, state, &mut frame.countries);
                frame.table_ns[2] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.links_result =
                    ccountry::walk_links(&memory, shared.base, state, &mut frame.links);
                if needed[0] {
                    frame.consistency_checked[0] = true;
                    let second =
                        ccountry::walk_links(&memory, shared.base, state, &mut frame.links_second);
                    frame.links_mismatches = mismatches(
                        frame.links_result,
                        &frame.links,
                        second,
                        &frame.links_second,
                    );
                    matched[0] = full_match(frame.links_result, second, frame.links_mismatches);
                }
                frame.table_ns[3] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.provinces_result =
                    cprovince::walk(&memory, shared.base, state, &mut frame.provinces);
                frame.table_ns[4] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.states_result = cstate::walk(&memory, shared.base, state, &mut frame.states);
                if needed[1] {
                    frame.consistency_checked[1] = true;
                    let second =
                        cstate::walk(&memory, shared.base, state, &mut frame.states_second);
                    frame.states_mismatches = mismatches(
                        frame.states_result,
                        &frame.states,
                        second,
                        &frame.states_second,
                    );
                    matched[1] = full_match(frame.states_result, second, frame.states_mismatches);
                }
                frame.table_ns[5] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.divisions_result =
                    carmy::walk(&memory, shared.base, state, &mut frame.divisions);
                if needed[2] {
                    frame.consistency_checked[2] = true;
                    let second =
                        carmy::walk(&memory, shared.base, state, &mut frame.divisions_second);
                    frame.divisions_mismatches = mismatches(
                        frame.divisions_result,
                        &frame.divisions,
                        second,
                        &frame.divisions_second,
                    );
                    matched[2] =
                        full_match(frame.divisions_result, second, frame.divisions_mismatches);
                }
                frame.table_ns[6] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.wars_result =
                    cwar_relation::walk(&memory, shared.base, state, &mut frame.wars);
                if needed[3] {
                    frame.consistency_checked[3] = true;
                    let second =
                        cwar_relation::walk(&memory, shared.base, state, &mut frame.wars_second);
                    frame.wars_mismatches =
                        mismatches(frame.wars_result, &frame.wars, second, &frame.wars_second);
                    matched[3] = full_match(frame.wars_result, second, frame.wars_mismatches);
                }
                if shared.war_matrix_remaining.load(Ordering::Relaxed) != 0 {
                    let matrix_begin = qpc();
                    let matrix = cwar_relation::walk_matrix(
                        &memory,
                        shared.base,
                        state,
                        &mut frame.wars_matrix,
                    );
                    frame.war_matrix_ns = shared.ns(qpc() - matrix_begin);
                    frame.war_matrix_result = Some(matrix);
                    matrix_complete = matches!(
                        (frame.wars_result, matrix),
                        (Outcome::Rows { .. }, Outcome::Rows { .. })
                    ) && frame.wars_mismatches == 0;
                }
                frame.table_ns[7] = shared.ns(qpc() - begin);
                let begin = qpc();
                let names = cfaction::walk(
                    &memory,
                    shared.base,
                    state,
                    &mut frame.factions,
                    &mut frame.faction_names,
                );
                frame.factions_result = names.outcome;
                frame.faction_names_len = names.bytes_len;
                frame.faction_names_required = names.required_bytes;
                if needed[4] {
                    frame.consistency_checked[4] = true;
                    let second = cfaction::walk(
                        &memory,
                        shared.base,
                        state,
                        &mut frame.factions_second,
                        &mut frame.faction_names_second,
                    );
                    frame.factions_mismatches = mismatches(
                        names.outcome,
                        &frame.factions,
                        second.outcome,
                        &frame.factions_second,
                    ) + usize::from(
                        names.bytes_len != second.bytes_len
                            || names.required_bytes != second.required_bytes
                            || frame.faction_names[..names.bytes_len]
                                != frame.faction_names_second[..second.bytes_len],
                    );
                    matched[4] =
                        full_match(names.outcome, second.outcome, frame.factions_mismatches);
                }
                frame.table_ns[8] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.faction_members_result = cfaction_system::walk_members(
                    &memory,
                    shared.base,
                    state,
                    &mut frame.faction_members,
                );
                if needed[5] {
                    frame.consistency_checked[5] = true;
                    let second = cfaction_system::walk_members(
                        &memory,
                        shared.base,
                        state,
                        &mut frame.faction_members_second,
                    );
                    frame.faction_members_mismatches = mismatches(
                        frame.faction_members_result,
                        &frame.faction_members,
                        second,
                        &frame.faction_members_second,
                    );
                    matched[5] = full_match(
                        frame.faction_members_result,
                        second,
                        frame.faction_members_mismatches,
                    );
                }
                frame.table_ns[9] = shared.ns(qpc() - begin);
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
                let stable = stable_clock(frame.clock_before, frame.clock_after);
                frame.consistency_successful =
                    unsafe { (&mut *shared.consistency.get()).commit(matched, stable, true) };
                if matrix_complete && stable {
                    shared.war_matrix_remaining.fetch_sub(1, Ordering::Relaxed);
                }
                // The spool thread waits for this Release before accessing the
                // frame, including the just-committed comparison snapshots.
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

    #[test]
    fn incomplete_or_mismatching_walks_do_not_complete_consistency_checks() {
        let rows = Outcome::Rows { len: 2 };
        let unavailable = Outcome::Unavailable {
            reason: "unreadable",
        };
        let truncated = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        for (first, second, mismatches) in [
            (unavailable, unavailable, 0),
            (truncated, truncated, 0),
            (rows, unavailable, 1),
            (rows, truncated, 1),
            (rows, Outcome::Rows { len: 1 }, 1),
            (rows, rows, 1),
        ] {
            assert!(!full_match(first, second, mismatches));
        }
        assert!(full_match(rows, rows, 0));
        assert!(full_match(
            Outcome::Rows { len: 0 },
            Outcome::Rows { len: 0 },
            0
        ));
    }

    #[test]
    fn consistency_windows_finish_independently_after_200_published_matches() {
        let mut window = ConsistencyWindow::default();
        for index in 0..CONSISTENCY_TARGET {
            let passed = [true, index % 2 == 0, false, false, false, false];
            window.commit(passed, true, true);
        }
        assert_eq!(window.successful, [200, 100, 0, 0, 0, 0]);
        assert_eq!(window.needed(), [false, true, true, true, true, true]);
        for _ in 0..CONSISTENCY_TARGET {
            window.commit(window.needed(), true, true);
        }
        assert_eq!(window.successful, [200; 6]);
        assert_eq!(window.needed(), [false; 6]);
        // Completion stays capped even if the caller supplies a redundant pass.
        assert_eq!(window.commit([true; 6], true, true), [200; 6]);
    }

    #[test]
    fn clock_changes_and_unpublished_buffers_leave_comparisons_pending() {
        let before = clock(10, 1);
        let mut changed_campaign = before;
        changed_campaign.campaign_id.bytes[0] = b'x';
        changed_campaign.campaign_id.len = 1;
        let mut changed_state = before;
        changed_state.game_state += 1;
        let mut window = ConsistencyWindow::default();
        for (before, after) in [
            (None, Some(before)),
            (Some(before), None),
            (Some(before), Some(clock(11, 1))),
            (Some(before), Some(clock(10, 2))),
            (Some(before), Some(changed_campaign)),
            (Some(before), Some(changed_state)),
        ] {
            assert_eq!(
                window.commit([true; 6], stable_clock(before, after), true),
                [0; 6]
            );
        }
        assert_eq!(
            window.commit([true; 6], stable_clock(Some(before), Some(before)), false),
            [0; 6]
        );
        assert_eq!(window.needed(), [true; 6]);
        assert_eq!(window.commit([true; 6], true, true), [1; 6]);
    }

    #[test]
    fn capacity_growth_keeps_comparison_buffers_and_name_arenas_in_step() {
        let mut frame = Captured::new();
        let before = frame.allocated_bytes();
        let mut capacities = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
        frame.war_matrix_result = Some(Outcome::Truncated {
            len: 1024,
            unread_rows: 7,
        });
        frame.factions_result = Outcome::Truncated {
            len: 64,
            unread_rows: 2,
        };
        frame.faction_members_result = Outcome::Truncated {
            len: 1024,
            unread_rows: 3,
        };
        frame.faction_names_required = 20_000;
        frame.grow(&mut capacities);
        assert_eq!(
            (
                frame.wars.len(),
                frame.wars_second.len(),
                frame.wars_matrix.len()
            ),
            (1031, 1031, 1031)
        );
        assert_eq!(
            (frame.factions.len(), frame.factions_second.len()),
            (66, 66)
        );
        assert_eq!(
            (
                frame.faction_members.len(),
                frame.faction_members_second.len()
            ),
            (1027, 1027)
        );
        assert_eq!(
            (frame.faction_names.len(), frame.faction_names_second.len()),
            (20_000, 20_000)
        );
        assert!(frame.allocated_bytes() > before);
        // An idle buffer with no truncation result receives the same capacities.
        let mut idle = Captured::new();
        idle.grow(&mut capacities);
        assert_eq!(idle.allocated_bytes(), frame.allocated_bytes());
    }

    fn comparison_capacities(frame: &Captured) -> [usize; 7] {
        [
            frame.links_second.capacity(),
            frame.states_second.capacity(),
            frame.divisions_second.capacity(),
            frame.wars_second.capacity(),
            frame.factions_second.capacity(),
            frame.faction_members_second.capacity(),
            frame.faction_names_second.capacity(),
        ]
    }

    #[test]
    fn each_completed_window_releases_only_its_comparison_buffers() {
        let mut frame = Captured::new();
        let mut capacities = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
        let mut expected = comparison_capacities(&frame);
        frame.consistency_successful = [CONSISTENCY_TARGET - 1; 6];
        frame.links[0].id = 7;
        frame.wars_matrix[0].first_country = 11;
        frame.faction_names[0] = b'x';
        let primary_capacities = [
            frame.links.capacity(),
            frame.states.capacity(),
            frame.divisions.capacity(),
            frame.wars.capacity(),
            frame.factions.capacity(),
            frame.faction_members.capacity(),
            frame.faction_names.capacity(),
            frame.wars_matrix.capacity(),
        ];
        frame.grow(&mut capacities);
        assert_eq!(comparison_capacities(&frame), expected);
        for index in 0..CONSISTENCY_TABLES.len() {
            let before = frame.allocated_bytes();
            frame.consistency_successful[index] = CONSISTENCY_TARGET;
            frame.grow(&mut capacities);
            expected[index] = 0;
            if index == 4 {
                expected[6] = 0;
            }
            assert_eq!(comparison_capacities(&frame), expected);
            assert!(frame.allocated_bytes() < before);
        }
        assert_eq!(
            [
                frame.links.capacity(),
                frame.states.capacity(),
                frame.divisions.capacity(),
                frame.wars.capacity(),
                frame.factions.capacity(),
                frame.faction_members.capacity(),
                frame.faction_names.capacity(),
                frame.wars_matrix.capacity(),
            ],
            primary_capacities
        );
        assert_eq!(frame.links[0].id, 7);
        assert_eq!(frame.wars_matrix[0].first_country, 11);
        assert_eq!(frame.faction_names[0], b'x');
    }

    #[test]
    fn later_growth_keeps_completed_comparisons_empty_and_pending_ones_ready() {
        let mut complete = Captured::new();
        let mut capacities = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
        complete.consistency_successful = [CONSISTENCY_TARGET; 6];
        complete.grow(&mut capacities);
        complete.divisions_result = Outcome::Truncated {
            len: 4096,
            unread_rows: 3,
        };
        complete.war_matrix_result = Some(Outcome::Truncated {
            len: 1024,
            unread_rows: 7,
        });
        complete.factions_result = Outcome::Truncated {
            len: 64,
            unread_rows: 2,
        };
        complete.faction_names_required = 20_000;
        complete.grow(&mut capacities);
        assert_eq!(comparison_capacities(&complete), [0; 7]);
        assert_eq!(complete.divisions.len(), 4099);
        assert_eq!(complete.wars.len(), 1031);
        assert_eq!(complete.wars_matrix.len(), 1031);
        assert_eq!(complete.factions.len(), 66);
        assert_eq!(complete.faction_names.len(), 20_000);

        let mut pending = Captured::new();
        pending.consistency_successful = [CONSISTENCY_TARGET - 1; 6];
        pending.grow(&mut capacities);
        assert_eq!(pending.divisions_second.len(), 4099);
        assert_eq!(pending.wars_second.len(), 1031);
        assert_eq!(pending.factions_second.len(), 66);
        assert_eq!(pending.faction_names_second.len(), 20_000);
        assert!(
            comparison_capacities(&pending)
                .into_iter()
                .all(|capacity| capacity != 0)
        );
    }
}
