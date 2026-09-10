//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use crate::{
    extended,
    map_capture::{MapCapacities, MapCaptured},
    roster::Roster,
    table_capture::{comparison_rows, grow_capacity},
};
use crossbeam_queue::ArrayQueue;
use engine::{
    carmy, ccountry, cfaction, cfaction_system,
    cgame_state::{self, Clock},
    country_memory::{Cache, SeedStatus},
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
// The broad capture needed one queued buffer; keep a producer/consumer pair
// plus two spare buffers, then grow off-thread when that reserve is consumed.
const INITIAL_BUFFERS: usize = 4;
const GROW_BUFFERS: usize = 4;
const FREE_RESERVE: usize = 1;
pub const INITIAL_CAPACITIES: [usize; 9] = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
pub const CONSISTENCY_TARGET: u16 = 200;
/// These indices describe comparison scheduling, not the replay's table indices.
pub const CONSISTENCY_TABLES: [&str; 17] = [
    "CountryLinks",
    "States",
    "Divisions",
    "WarRelations",
    "Factions",
    "FactionMembers",
    "Players",
    "HumanControllers",
    "LandCombats",
    "Fleets",
    "TaskForces",
    "Ships",
    "NavalCombats",
    "NavalCombatParticipants",
    "AirWings",
    "ArmyOrders",
    "Provinces",
];
pub const PROVINCE_CHECK: usize = CONSISTENCY_TABLES.len() - 1;
const LOSS_RUNS: usize = 64;
pub static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn mismatches<R: PartialEq>(
    first: Outcome,
    rows: &[R],
    second: Outcome,
    other: &[R],
) -> usize {
    if first != second {
        return 1;
    }
    rows[..first.len()]
        .iter()
        .zip(&other[..second.len()])
        .filter(|(a, b)| a != b)
        .count()
}

pub(crate) fn full_match(first: Outcome, second: Outcome, mismatches: usize) -> bool {
    matches!(first, Outcome::Rows { .. }) && first == second && mismatches == 0
}

pub(crate) fn stable_clock(before: Option<Clock>, after: Option<Clock>) -> bool {
    matches!((before, after), (Some(before), Some(after))
        if before.hour == after.hour
            && before.game_state == after.game_state
            && before.campaign_id == after.campaign_id
            && before.play_session_started_at == after.play_session_started_at)
}

struct ConsistencyWindow<const N: usize> {
    successful: [u16; N],
}

impl<const N: usize> Default for ConsistencyWindow<N> {
    fn default() -> Self {
        Self { successful: [0; N] }
    }
}

impl<const N: usize> ConsistencyWindow<N> {
    fn needed(&self) -> [bool; N] {
        self.successful.map(|count| count < CONSISTENCY_TARGET)
    }

    fn commit(&mut self, matched: [bool; N], stable: bool, published: bool) -> [u16; N] {
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
    pub map: MapCaptured,
    pub extended: extended::Captured,
    pub extended_checked: [bool; TABLE_COUNT],
    pub extended_successful: [u16; TABLE_COUNT],
    pub country_cache_status: SeedStatus,
    pub country_cache_entries: usize,
    pub country_cache_error: Option<&'static str>,
    pub country_cache_ns: u64,
    pub clock_entry: Option<Clock>,
    pub clock_before: Option<Clock>,
    pub clock_after: Option<Clock>,
    pub countries: Vec<ccountry::Row>,
    pub provinces: Vec<cprovince::Row>,
    provinces_second: Vec<cprovince::Row>,
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
    pub provinces_mismatches: usize,
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
        Self::with_capacities(
            &INITIAL_CAPACITIES,
            &MapCapacities::default(),
            [0; CONSISTENCY_TABLES.len()],
            &extended::Capacities::default(),
            [0; TABLE_COUNT],
        )
    }

    fn with_capacities(
        capacities: &[usize; 9],
        map_capacities: &MapCapacities,
        successful: [u16; CONSISTENCY_TABLES.len()],
        extended_capacities: &extended::Capacities,
        extended_successful: [u16; TABLE_COUNT],
    ) -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        Self {
            map: MapCaptured::with_capacities(
                map_capacities,
                &successful[6..PROVINCE_CHECK],
                extended::Captured::source_checks(
                    &extended_successful.map(|n| n < CONSISTENCY_TARGET),
                ),
            ),
            extended: extended::Captured::with_capacities(
                extended_capacities,
                &extended_successful,
            ),
            extended_checked: [false; TABLE_COUNT],
            extended_successful,
            country_cache_status: SeedStatus::Unavailable {
                reason: "clock unavailable",
            },
            country_cache_entries: 0,
            country_cache_error: None,
            country_cache_ns: 0,
            clock_entry: None,
            clock_before: None,
            clock_after: None,
            countries: vec![ccountry::Row::default(); capacities[0]],
            provinces: vec![cprovince::Row::default(); capacities[1]],
            provinces_second: comparison_rows(capacities[1], successful[PROVINCE_CHECK]),
            states: vec![cstate::Row::default(); capacities[3]],
            states_second: comparison_rows(capacities[3], successful[1]),
            divisions: vec![carmy::Row::default(); capacities[4]],
            divisions_second: comparison_rows(
                capacities[4],
                if extended::Captured::needs_division_check(
                    &extended_successful.map(|n| n < CONSISTENCY_TARGET),
                ) {
                    0
                } else {
                    successful[2]
                },
            ),
            wars: vec![cwar_relation::Row::default(); capacities[5]],
            wars_second: comparison_rows(capacities[5], successful[3]),
            wars_matrix: vec![cwar_relation::Row::default(); capacities[5]],
            factions: vec![cfaction::Row::default(); capacities[6]],
            factions_second: comparison_rows(capacities[6], successful[4]),
            faction_names: vec![0; capacities[8]],
            faction_names_second: comparison_rows(capacities[8], successful[4]),
            faction_names_len: 0,
            faction_names_required: 0,
            faction_members: vec![cfaction_system::MemberRow::default(); capacities[7]],
            faction_members_second: comparison_rows(capacities[7], successful[5]),
            links: vec![ccountry::LinkRow::default(); capacities[2]],
            links_second: comparison_rows(capacities[2], successful[0]),
            countries_result: unavailable,
            provinces_result: unavailable,
            provinces_mismatches: 0,
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
            consistency_successful: successful,
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
            + self.map.allocated_bytes()
            + self.extended.allocated_bytes()
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + (self.provinces.capacity() + self.provinces_second.capacity())
                * size_of::<cprovince::Row>()
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
                grow_capacity(capacity, len.saturating_add(unread_rows as usize));
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
        comparison(
            &mut self.provinces_second,
            self.consistency_successful[PROVINCE_CHECK],
            capacities[1],
        );
        resize(&mut self.states, self.states_result, &mut capacities[3]);
        resize(
            &mut self.divisions,
            self.divisions_result,
            &mut capacities[4],
        );
        if let Some(Outcome::Truncated { len, unread_rows }) = self.war_matrix_result {
            grow_capacity(&mut capacities[5], len.saturating_add(unread_rows as usize));
        }
        resize(&mut self.wars, self.wars_result, &mut capacities[5]);
        resize(&mut self.factions, self.factions_result, &mut capacities[6]);
        resize(
            &mut self.faction_members,
            self.faction_members_result,
            &mut capacities[7],
        );
        grow_capacity(&mut capacities[8], self.faction_names_required);
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
            if extended::Captured::needs_division_check(
                &self.extended_successful.map(|n| n < CONSISTENCY_TARGET),
            ) {
                0
            } else {
                self.consistency_successful[2]
            },
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
    /// Slot addresses never move. Only published slots own a Captured allocation.
    pub frames: [AtomicPtr<Captured>; BUFFERS],
    pub completed: [AtomicI64; BUFFERS],
    pub pool_buffers: AtomicUsize,
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
    consistency: UnsafeCell<ConsistencyWindow<{ CONSISTENCY_TABLES.len() }>>,
    extended_consistency: UnsafeCell<ConsistencyWindow<TABLE_COUNT>>,
    /// Fresh country authentication per callback, verified directly before publication.
    country_cache: UnsafeCell<Cache>,
    /// Independent matrix census supplements the owned-vector double read for 200 full walks.
    war_matrix_remaining: AtomicUsize,
}

// Each frame index has exactly one owner: a queue, the sole callback, or the spool thread.
// The pending loss batch has one game-thread writer until the restored hook is quiescent.
unsafe impl Sync for Shared {}

impl Shared {
    pub fn new(base: usize, trampoline: usize, frequency: u64) -> Self {
        let roster = Box::new(Roster::new());
        let country_cache = Cache::with_capacity(8192);
        let pool_bytes = roster.allocated_bytes() + country_cache.allocated_bytes();
        let shared = Self {
            base,
            trampoline,
            frequency,
            frames: std::array::from_fn(|_| AtomicPtr::new(ptr::null_mut())),
            completed: std::array::from_fn(|_| AtomicI64::new(0)),
            pool_buffers: AtomicUsize::new(0),
            free: ArrayQueue::new(BUFFERS),
            ready: ArrayQueue::new(BUFFERS),
            dropped: AtomicU64::new(0),
            pool_bytes: AtomicU64::new(pool_bytes as u64),
            recording: AtomicBool::new(true),
            stop_spool: AtomicBool::new(false),
            roster: UnsafeCell::new(Some(roster)),
            roster_ready: AtomicBool::new(false),
            pending: UnsafeCell::new(LossBatch::default()),
            consistency: UnsafeCell::new(ConsistencyWindow::default()),
            extended_consistency: UnsafeCell::new(ConsistencyWindow::default()),
            country_cache: UnsafeCell::new(country_cache),
            war_matrix_remaining: AtomicUsize::new(200),
        };
        for index in 0..INITIAL_BUFFERS {
            // No callback or spool thread can observe Shared during construction.
            unsafe { shared.publish_frame(index, Box::new(Captured::new())) };
        }
        shared
    }

    /// Publish a previously unused slot; the caller is the sole pool allocator.
    unsafe fn publish_frame(&self, index: usize, frame: Box<Captured>) {
        debug_assert!(self.frames[index].load(Ordering::Relaxed).is_null());
        let bytes = frame.allocated_bytes();
        // Acquire after popping a free index makes the entire allocation visible.
        self.frames[index].store(Box::into_raw(frame), Ordering::Release);
        self.pool_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.pool_buffers.store(index + 1, Ordering::Relaxed);
        // A newly allocated slot has no index in either queue. Both queues have
        // room for every allocated slot, including ones currently held by a thread.
        let published = self.free.push(index);
        debug_assert!(published.is_ok());
    }

    /// Allocate only on the spool thread, after servicing a captured frame.
    ///
    /// # Safety
    /// The caller must be the sole pool allocator; the callback never calls this.
    pub unsafe fn grow_pool(
        &self,
        capacities: &[usize; 9],
        map_capacities: &MapCapacities,
        successful: [u16; CONSISTENCY_TABLES.len()],
        extended_capacities: &extended::Capacities,
        extended_successful: [u16; TABLE_COUNT],
    ) -> usize {
        if self.stop_spool.load(Ordering::Acquire) {
            return 0;
        }
        let allocated = self.pool_buffers.load(Ordering::Relaxed);
        let count = pool_growth(self.free.len(), allocated);
        for index in allocated..allocated + count {
            let frame = Box::new(Captured::with_capacities(
                capacities,
                map_capacities,
                successful,
                extended_capacities,
                extended_successful,
            ));
            unsafe { self.publish_frame(index, frame) };
        }
        count
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

fn pool_growth(free: usize, allocated: usize) -> usize {
    if free <= FREE_RESERVE {
        GROW_BUFFERS.min(BUFFERS.saturating_sub(allocated))
    } else {
        0
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // The loader joins the spool thread after callback quiescence before
        // dropping its last Arc. Slots stay allocated until that point.
        for slot in &mut self.frames {
            let frame = *slot.get_mut();
            if !frame.is_null() {
                unsafe { drop(Box::from_raw(frame)) };
            }
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
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
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
            frame.provinces_mismatches = 0;
            frame.map.reset();
            frame.extended.reset();
            frame.extended_checked = [false; TABLE_COUNT];
            frame.country_cache_status = SeedStatus::Unavailable {
                reason: "clock unavailable",
            };
            frame.country_cache_entries = 0;
            frame.country_cache_error = None;
            frame.country_cache_ns = 0;
            let needed = unsafe { (&*shared.consistency.get()).needed() };
            let extended_needed = unsafe { (&*shared.extended_consistency.get()).needed() };
            let mut matched = [false; CONSISTENCY_TABLES.len()];
            let mut extended_matched = [false; TABLE_COUNT];
            let mut matrix_complete = false;
            if let Some(clock) = frame.clock_before {
                let state = clock.game_state as usize;
                let cache_begin = qpc();
                let cached = unsafe { &mut *shared.country_cache.get() }.prepare(
                    &memory,
                    shared.base,
                    state,
                );
                frame.country_cache_status = cached.seed_status();
                frame.country_cache_entries = cached.cached_entries();
                let cache_prepare_ticks = qpc() - cache_begin;
                let begin = qpc();
                frame.countries_result =
                    ccountry::walk(&cached, shared.base, state, &mut frame.countries);
                frame.table_ns[2] = shared.ns(qpc() - begin);
                let begin = qpc();
                frame.links_result =
                    ccountry::walk_links(&cached, shared.base, state, &mut frame.links);
                if needed[0] {
                    frame.consistency_checked[0] = true;
                    let second =
                        ccountry::walk_links(&cached, shared.base, state, &mut frame.links_second);
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
                if needed[PROVINCE_CHECK] {
                    frame.consistency_checked[PROVINCE_CHECK] = true;
                    let second =
                        cprovince::walk(&memory, shared.base, state, &mut frame.provinces_second);
                    frame.provinces_mismatches = mismatches(
                        frame.provinces_result,
                        &frame.provinces,
                        second,
                        &frame.provinces_second,
                    );
                    matched[PROVINCE_CHECK] =
                        full_match(frame.provinces_result, second, frame.provinces_mismatches);
                }
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
                let mut division_source_ticks = 0_u64;
                frame.divisions_result = carmy::walk_with(
                    &cached,
                    shared.base,
                    state,
                    &mut frame.divisions,
                    |pointer, row| {
                        division_source_ticks += frame.extended.visit_division(pointer, row, false);
                    },
                );
                division_source_ticks += frame
                    .extended
                    .finish_divisions(frame.divisions_result, false);
                if needed[2] || extended::Captured::needs_division_check(&extended_needed) {
                    frame.consistency_checked[2] = needed[2];
                    let second = carmy::walk_with(
                        &cached,
                        shared.base,
                        state,
                        &mut frame.divisions_second,
                        |pointer, row| {
                            division_source_ticks +=
                                frame.extended.visit_division(pointer, row, true);
                        },
                    );
                    division_source_ticks += frame.extended.finish_divisions(second, true);
                    frame.divisions_mismatches = mismatches(
                        frame.divisions_result,
                        &frame.divisions,
                        second,
                        &frame.divisions_second,
                    );
                    matched[2] = needed[2]
                        && full_match(frame.divisions_result, second, frame.divisions_mismatches);
                }
                frame.table_ns[6] =
                    shared.ns((qpc() - begin).saturating_sub(division_source_ticks as i64));
                let begin = qpc();
                frame.wars_result =
                    cwar_relation::walk(&cached, shared.base, state, &mut frame.wars);
                if needed[3] {
                    frame.consistency_checked[3] = true;
                    let second =
                        cwar_relation::walk(&cached, shared.base, state, &mut frame.wars_second);
                    frame.wars_mismatches =
                        mismatches(frame.wars_result, &frame.wars, second, &frame.wars_second);
                    matched[3] = full_match(frame.wars_result, second, frame.wars_mismatches);
                }
                if shared.war_matrix_remaining.load(Ordering::Relaxed) != 0 {
                    let matrix_begin = qpc();
                    let matrix = cwar_relation::walk_matrix(
                        &cached,
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
                    &cached,
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
                        &cached,
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
                    &cached,
                    shared.base,
                    state,
                    &mut frame.faction_members,
                );
                if needed[5] {
                    frame.consistency_checked[5] = true;
                    let second = cfaction_system::walk_members(
                        &cached,
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
                let (map_matched, map_ticks) = frame.map.sample(
                    &cached,
                    shared.base,
                    state,
                    &needed[6..PROVINCE_CHECK],
                    &mut frame
                        .extended
                        .sources(&cached, shared.base, &extended_needed),
                );
                matched[6..PROVINCE_CHECK].copy_from_slice(&map_matched);
                frame.consistency_checked[6..PROVINCE_CHECK]
                    .copy_from_slice(&needed[6..PROVINCE_CHECK]);
                for (total, ticks) in frame.table_ns[10..].iter_mut().zip(map_ticks) {
                    *total = shared.ns(ticks as i64);
                }
                let (checks, ticks) = frame.extended.capture(
                    &cached,
                    shared.base,
                    state,
                    clock.hour,
                    &extended_needed,
                );
                extended_matched = checks;
                for table in extended::TABLES {
                    let index = table.index();
                    frame.extended_checked[index] = extended_needed[index];
                    frame.table_ns[index] = shared.ns(ticks[index] as i64);
                }
                let cache_verify_begin = qpc();
                frame.country_cache_error = cached.verify().err();
                frame.country_cache_ns =
                    shared.ns(cache_prepare_ticks + qpc() - cache_verify_begin);
                // Authentication is shared by country-dependent tables. This
                // accounting slot includes the seed and final direct check once.
                frame.table_ns[2] += frame.country_cache_ns;
                if frame.country_cache_error.is_some() {
                    matched.fill(false);
                    extended_matched.fill(false);
                    matrix_complete = false;
                }
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
                frame.extended_successful = unsafe {
                    (&mut *shared.extended_consistency.get()).commit(extended_matched, stable, true)
                };
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

    #[test]
    fn pool_growth_preserves_reserve_threshold_and_hard_queue_bound() {
        assert_eq!(pool_growth(FREE_RESERVE + 1, INITIAL_BUFFERS), 0);
        assert_eq!(pool_growth(FREE_RESERVE, INITIAL_BUFFERS), GROW_BUFFERS);
        assert_eq!(pool_growth(0, BUFFERS - 1), 1);
        assert_eq!(pool_growth(0, BUFFERS), 0);
        assert_eq!(pool_growth(0, BUFFERS + 1), 0);
    }

    #[test]
    fn pool_initialization_allocates_only_published_slots_and_counts_actual_bytes() {
        let shared = Shared::new(0, 0, 1);
        assert_eq!(shared.free.capacity(), BUFFERS);
        assert_eq!(shared.ready.capacity(), BUFFERS);
        assert_eq!(shared.free.len(), INITIAL_BUFFERS);
        assert_eq!(shared.pool_buffers.load(Ordering::Relaxed), INITIAL_BUFFERS);
        let mut bytes = unsafe { (&*shared.roster.get()).as_ref().unwrap() }.allocated_bytes();
        bytes += unsafe { &*shared.country_cache.get() }.allocated_bytes();
        for (index, slot) in shared.frames.iter().enumerate() {
            let frame = slot.load(Ordering::Acquire);
            if index < INITIAL_BUFFERS {
                assert!(!frame.is_null());
                bytes += unsafe { &*frame }.allocated_bytes();
            } else {
                assert!(frame.is_null());
            }
        }
        assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), bytes as u64);
        assert_eq!(
            unsafe {
                shared.grow_pool(
                    &INITIAL_CAPACITIES,
                    &MapCapacities::default(),
                    [0; CONSISTENCY_TABLES.len()],
                    &extended::Capacities::default(),
                    [0; TABLE_COUNT],
                )
            },
            0
        );
    }

    #[test]
    fn new_pool_slots_publish_current_capacities_and_completed_proofs_without_moving_old_slots() {
        let shared = Shared::new(0, 0, 1);
        let original = shared.frames[0].load(Ordering::Acquire);
        let held: Vec<_> = (0..INITIAL_BUFFERS)
            .map(|_| shared.free.pop().unwrap())
            .collect();
        let mut capacities = INITIAL_CAPACITIES;
        capacities[0] += 9;
        capacities[4] += 17;
        capacities[8] += 23;
        let mut successful = [CONSISTENCY_TARGET; CONSISTENCY_TABLES.len()];
        successful[2] -= 1;
        let before_bytes = shared.pool_bytes.load(Ordering::Relaxed);
        let published = AtomicBool::new(false);
        let slots = std::thread::scope(|scope| {
            let consumer = scope.spawn(|| {
                let mut slots = Vec::new();
                while slots.len() < GROW_BUFFERS {
                    let Some(index) = shared.free.pop() else {
                        if published.load(Ordering::Acquire) && shared.free.is_empty() {
                            break;
                        }
                        std::thread::yield_now();
                        continue;
                    };
                    let pointer = shared.frames[index].load(Ordering::Acquire);
                    assert!(!pointer.is_null());
                    let frame = unsafe { &*pointer };
                    assert_eq!(frame.countries.len(), capacities[0]);
                    assert_eq!(frame.divisions.len(), capacities[4]);
                    assert_eq!(frame.divisions_second.len(), capacities[4]);
                    assert_eq!(frame.faction_names.len(), capacities[8]);
                    assert_eq!(frame.consistency_successful, successful);
                    let second = comparison_capacities(frame);
                    assert_eq!(second, [0, 0, capacities[4], 0, 0, 0, 0]);
                    slots.push((index, frame.allocated_bytes()));
                }
                assert_eq!(slots.len(), GROW_BUFFERS);
                slots
            });
            let added = unsafe {
                shared.grow_pool(
                    &capacities,
                    &MapCapacities::default(),
                    successful,
                    &extended::Capacities::default(),
                    [CONSISTENCY_TARGET; TABLE_COUNT],
                )
            };
            published.store(true, Ordering::Release);
            assert_eq!(added, GROW_BUFFERS);
            consumer.join().unwrap()
        });
        assert_eq!(shared.frames[0].load(Ordering::Acquire), original);
        assert_eq!(
            shared.pool_buffers.load(Ordering::Relaxed),
            INITIAL_BUFFERS + GROW_BUFFERS
        );
        assert_eq!(
            shared.pool_bytes.load(Ordering::Relaxed) - before_bytes,
            slots.iter().map(|(_, bytes)| *bytes as u64).sum::<u64>()
        );
        for index in held
            .into_iter()
            .chain(slots.into_iter().map(|(index, _)| index))
        {
            shared.free.push(index).unwrap();
        }
        assert_eq!(shared.free.len(), INITIAL_BUFFERS + GROW_BUFFERS);
    }

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
            let mut passed = [false; CONSISTENCY_TABLES.len()];
            passed[0] = true;
            passed[1] = index % 2 == 0;
            window.commit(passed, true, true);
        }
        assert_eq!(&window.successful[..6], &[200, 100, 0, 0, 0, 0]);
        assert_eq!(
            &window.needed()[..6],
            &[false, true, true, true, true, true]
        );
        for _ in 0..CONSISTENCY_TARGET {
            window.commit(window.needed(), true, true);
        }
        assert_eq!(window.successful, [200; CONSISTENCY_TABLES.len()]);
        assert_eq!(window.needed(), [false; CONSISTENCY_TABLES.len()]);
        // Completion stays capped even if the caller supplies a redundant pass.
        assert_eq!(
            window.commit([true; CONSISTENCY_TABLES.len()], true, true),
            [200; CONSISTENCY_TABLES.len()]
        );
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
                window.commit(
                    [true; CONSISTENCY_TABLES.len()],
                    stable_clock(before, after),
                    true
                ),
                [0; CONSISTENCY_TABLES.len()]
            );
        }
        assert_eq!(
            window.commit(
                [true; CONSISTENCY_TABLES.len()],
                stable_clock(Some(before), Some(before)),
                false
            ),
            [0; CONSISTENCY_TABLES.len()]
        );
        assert_eq!(window.needed(), [true; CONSISTENCY_TABLES.len()]);
        assert_eq!(
            window.commit([true; CONSISTENCY_TABLES.len()], true, true),
            [1; CONSISTENCY_TABLES.len()]
        );
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
            (2048, 2048, 2048)
        );
        assert_eq!(
            (frame.factions.len(), frame.factions_second.len()),
            (128, 128)
        );
        assert_eq!(
            (
                frame.faction_members.len(),
                frame.faction_members_second.len()
            ),
            (2048, 2048)
        );
        assert_eq!(
            (frame.faction_names.len(), frame.faction_names_second.len()),
            (32768, 32768)
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
        frame.extended_successful.fill(CONSISTENCY_TARGET);
        let mut capacities = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
        let mut expected = comparison_capacities(&frame);
        frame.consistency_successful = [CONSISTENCY_TARGET - 1; CONSISTENCY_TABLES.len()];
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
        for index in 0..6 {
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
        let primary_provinces = frame.provinces.capacity();
        let before = frame.allocated_bytes();
        frame.consistency_successful[PROVINCE_CHECK] = CONSISTENCY_TARGET;
        frame.grow(&mut capacities);
        assert_eq!(frame.provinces_second.capacity(), 0);
        assert_eq!(frame.provinces.capacity(), primary_provinces);
        assert!(frame.allocated_bytes() < before);
    }

    #[test]
    fn later_growth_keeps_completed_comparisons_empty_and_pending_ones_ready() {
        let mut complete = Captured::new();
        complete.extended_successful.fill(CONSISTENCY_TARGET);
        let mut capacities = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];
        complete.consistency_successful = [CONSISTENCY_TARGET; CONSISTENCY_TABLES.len()];
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
        assert_eq!(complete.provinces_second.capacity(), 0);
        assert_eq!(complete.divisions.len(), 8192);
        assert_eq!(complete.wars.len(), 2048);
        assert_eq!(complete.wars_matrix.len(), 2048);
        assert_eq!(complete.factions.len(), 128);
        assert_eq!(complete.faction_names.len(), 32768);

        let mut pending = Captured::new();
        pending.consistency_successful = [CONSISTENCY_TARGET - 1; CONSISTENCY_TABLES.len()];
        pending.grow(&mut capacities);
        assert_eq!(pending.provinces_second.len(), capacities[1]);
        assert_eq!(pending.divisions_second.len(), 8192);
        assert_eq!(pending.wars_second.len(), 2048);
        assert_eq!(pending.factions_second.len(), 128);
        assert_eq!(pending.faction_names_second.len(), 32768);
        assert!(
            comparison_capacities(&pending)
                .into_iter()
                .all(|capacity| capacity != 0)
        );
    }
}
