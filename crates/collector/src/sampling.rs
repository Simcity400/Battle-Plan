//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use crate::{
    acquisition, decode, extended,
    map_capture::{MapCapacities, MapCaptured},
    raw::{RawLog, Recorded},
    roster::Roster,
    table_capture::{comparison_rows, grow_capacity},
};
use crossbeam_queue::ArrayQueue;
use engine::{
    army_upgrade_request_row, carmy, carmy_upgrades_requests, ccountry, cequipment_variant_pool,
    cfaction, cfaction_system,
    cgame_state::{self, Clock},
    country_memory::{Cache, SeedStatus},
    cpdx_array::Array,
    cprovince, creferenced_division_template, cstate, cunit, cupgrade_delivery, cwar_relation,
    equipment_entry,
    memory::LiveMemory,
    msvc_string::Text,
    save_allocation_counters::{self, AllocationCounters},
    walk::Outcome,
};
use replay::frame::TABLE_COUNT;
use std::{
    cell::UnsafeCell,
    mem::size_of,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
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

#[derive(Clone, Copy)]
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

// Staggered cadence branch/keyframe state. Game thread only.
#[derive(Clone, Copy, Debug, Default)]
struct CadenceState {
    initialized: bool,
    prev_hour: i32,
    prev_signal: i64,
    prev_campaign: Text<128>,
    base_hour: i32,
}

impl CadenceState {
    // New branches never open on stale data: first, signal, campaign,
    // discontinuity and 720-hour keyframes all force a full refresh.
    fn force_full(&mut self, clock: &Clock) -> bool {
        if !self.initialized {
            self.initialized = true;
            self.prev_hour = clock.hour;
            self.prev_signal = clock.play_session_started_at;
            self.prev_campaign = clock.campaign_id;
            self.base_hour = clock.hour;
            return true;
        }
        let signal = self.prev_signal != clock.play_session_started_at;
        let campaign = self.prev_campaign != clock.campaign_id;
        let gap = self.prev_hour.checked_add(1) != Some(clock.hour);
        let keyframe = i64::from(clock.hour) - i64::from(self.base_hour)
            >= i64::from(replay::KEYFRAME_INTERVAL);
        let force = signal || campaign || gap || keyframe;
        if force {
            self.base_hour = clock.hour;
        }
        self.prev_hour = clock.hour;
        self.prev_signal = clock.play_session_started_at;
        self.prev_campaign = clock.campaign_id;
        force
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

/// Owned diagnostic bytes, separate from replay rows and availability.
pub struct OrderCounterCapture {
    pub forced: bool,
    pub rows: Vec<ccountry::OrderInstanceCounter>,
    pub read: Option<ccountry::OrderInstanceCounterRead>,
    pub qpc_begin: i64,
    pub qpc_end: i64,
    pub elapsed_ns: u64,
}

impl OrderCounterCapture {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            forced: false,
            rows: vec![ccountry::OrderInstanceCounter::default(); capacity],
            read: None,
            qpc_begin: 0,
            qpc_end: 0,
            elapsed_ns: 0,
        }
    }

    fn reset(&mut self) {
        self.forced = false;
        self.read = None;
        self.qpc_begin = 0;
        self.qpc_end = 0;
        self.elapsed_ns = 0;
    }

    fn capture(
        &mut self,
        force_full: bool,
        memory: &impl engine::Memory,
        roster: Option<ccountry::AuthenticatedRoster<'_>>,
    ) {
        self.reset();
        if force_full {
            self.forced = true;
            self.read = ccountry::read_order_instance_counters(memory, roster, &mut self.rows);
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<ccountry::OrderInstanceCounter>()
    }

    // Off-thread only, on buffers the existing pool has actually allocated.
    fn grow(&mut self, country_capacity: usize) {
        let required = self
            .read
            .map_or(0, |read| read.required)
            .max(country_capacity);
        if self.rows.len() < required {
            self.rows
                .resize(required, ccountry::OrderInstanceCounter::default());
        }
    }
}

pub struct Captured {
    /// The same sequence carried by this frame's sole DecodeJob (zero is valid).
    pub capture_seq: u64,
    pub order_counters: OrderCounterCapture,
    pub map: MapCaptured,
    pub extended: extended::Captured,
    pub extended_checked: [bool; TABLE_COUNT],
    // Raw-copy verdicts survive pack resets between staging and worker decode.
    pub extended_raw_mismatches: [bool; TABLE_COUNT],
    pub extended_successful: [u16; TABLE_COUNT],
    pub country_cache_status: SeedStatus,
    pub country_cache_entries: usize,
    pub country_cache_error: Option<&'static str>,
    pub country_cache_ns: u64,
    pub clock_entry: Option<Clock>,
    pub clock_before: Option<Clock>,
    pub clock_after: Option<Clock>,
    /// The engine's monotonic allocation counters at this sampled tick.
    pub counters: Option<AllocationCounters>,
    pub countries: Vec<ccountry::Row>,
    pub provinces: Vec<cprovince::Row>,
    provinces_second: Vec<cprovince::Row>,
    pub states: Vec<cstate::Row>,
    states_second: Vec<cstate::Row>,
    pub divisions: Vec<carmy::Row>,
    divisions_second: Vec<carmy::Row>,
    pub wars: Vec<cwar_relation::Row>,
    pub(crate) wars_snapshot: cwar_relation::OwnedSnapshotScratch,
    pub(crate) wars_snapshot_sampled: bool,
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
            capture_seq: 0,
            order_counters: OrderCounterCapture::with_capacity(capacities[0]),
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
            extended_raw_mismatches: [false; TABLE_COUNT],
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
            counters: None,
            countries: vec![ccountry::Row::default(); capacities[0]],
            provinces: vec![cprovince::Row::default(); capacities[1]],
            provinces_second: comparison_rows(capacities[1], successful[PROVINCE_CHECK]),
            states: vec![cstate::Row::default(); capacities[3]],
            states_second: comparison_rows(
                capacities[3],
                if extended::Captured::needs_state_check(
                    &extended_successful.map(|n| n < CONSISTENCY_TARGET),
                ) {
                    0
                } else {
                    successful[1]
                },
            ),
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
            wars_snapshot: cwar_relation::OwnedSnapshotScratch::with_capacity(capacities[0]),
            wars_snapshot_sampled: false,
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
            + self.order_counters.allocated_bytes()
            + self.map.allocated_bytes()
            + self.extended.allocated_bytes()
            + self.wars_snapshot.allocated_bytes()
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
        self.order_counters.grow(capacities[0]);
        self.wars_snapshot.grow(capacities[0]);
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
            if extended::Captured::needs_state_check(
                &self.extended_successful.map(|n| n < CONSISTENCY_TARGET),
            ) {
                0
            } else {
                self.consistency_successful[1]
            },
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
    /// Initialized before hook publication; workers own raw jobs, never frames.
    pub acquisition: OnceLock<acquisition::Pool>,
    pub acquisition_dispatch_ns: AtomicU64,
    pub acquisition_wait_ns: AtomicU64,
    /// Existing QPC brackets around the original native tick, not collector
    /// acquisition. These diagnostic counters also include ticks with no free
    /// recording buffer and exclude game-loop work outside the hooked function.
    pub native_tick_ns: AtomicU64,
    pub native_tick_calls: AtomicU64,
    pub base: usize,
    pub trampoline: usize,
    pub frequency: u64,
    /// Slot addresses never move. Only published slots own a Captured allocation.
    pub frames: [AtomicPtr<Captured>; BUFFERS],
    pub completed: [AtomicI64; BUFFERS],
    pub callback_tail: [AtomicI64; BUFFERS],
    pub pool_buffers: AtomicUsize,
    pub free: ArrayQueue<usize>,
    pub ready: ArrayQueue<usize>,
    /// Single FIFO decode queue between the game thread and the spool ready
    /// queue. Every frame transits it in hour order (lightweight jobs; raw
    /// payloads are bounded recycled boxes, never pool memory).
    pub decode_queue: decode::DecodeQueue,
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
    cadence: UnsafeCell<CadenceState>,
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
            acquisition: OnceLock::new(),
            acquisition_dispatch_ns: AtomicU64::new(0),
            acquisition_wait_ns: AtomicU64::new(0),
            native_tick_ns: AtomicU64::new(0),
            native_tick_calls: AtomicU64::new(0),
            base,
            trampoline,
            frequency,
            frames: std::array::from_fn(|_| AtomicPtr::new(ptr::null_mut())),
            completed: std::array::from_fn(|_| AtomicI64::new(0)),
            callback_tail: std::array::from_fn(|_| AtomicI64::new(0)),
            pool_buffers: AtomicUsize::new(0),
            free: ArrayQueue::new(BUFFERS),
            ready: ArrayQueue::new(BUFFERS),
            decode_queue: decode::new_queue(),
            dropped: AtomicU64::new(0),
            pool_bytes: AtomicU64::new(pool_bytes as u64),
            recording: AtomicBool::new(true),
            stop_spool: AtomicBool::new(false),
            roster: UnsafeCell::new(Some(roster)),
            roster_ready: AtomicBool::new(false),
            pending: UnsafeCell::new(LossBatch::default()),
            consistency: UnsafeCell::new(ConsistencyWindow::default()),
            extended_consistency: UnsafeCell::new(ConsistencyWindow::default()),
            cadence: UnsafeCell::new(CadenceState::default()),
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

pub(crate) fn completed_callback_ticks(base: &AtomicI64, tail: &AtomicI64) -> Option<i64> {
    let base = base.load(Ordering::Acquire);
    let tail = tail.load(Ordering::Acquire);
    (base > 0 && tail > 0).then(|| base.saturating_add(tail))
}

fn complete_matrix_check(remaining: &AtomicUsize) {
    // A completed window stays closed even if a stale successful report is
    // observed. Wrapping zero restarts billions of expensive matrix reads.
    let _ = remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_sub(1)
    });
}

fn pool_growth(free: usize, allocated: usize) -> usize {
    if free <= FREE_RESERVE {
        GROW_BUFFERS.min(BUFFERS.saturating_sub(allocated))
    } else {
        0
    }
}

/// Whole-frame drop for an owned buffer: restores the witnesses the frame
/// already absorbed, records the gap through the existing `LossBatch` path
/// with the existing counters, and returns the buffer to the free queue.
/// Shared by every queue-full branch; no new loss reasons.
fn drop_owned_frame(shared: &Shared, frame: &mut Captured, index: usize, entry: Option<Clock>) {
    unsafe {
        *shared.pending.get() = frame.dropped_before;
        (&mut *shared.pending.get()).push(entry, frame.clock_after);
    }
    shared.dropped.fetch_add(1, Ordering::Relaxed);
    let _ = shared.free.push(index);
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
        shared
            .native_tick_ns
            .fetch_add(shared.ns(original_end - original_begin), Ordering::Relaxed);
        shared.native_tick_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(index) = index {
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.clock_entry = entry;
            frame.qpc = entered;
            frame.table_ns = [0; TABLE_COUNT];
            let before = qpc();
            frame.clock_before = cgame_state::read_clock(&memory, shared.base);
            // Passive scalar reads of the same counters the save header emits.
            frame.counters = frame.clock_before.map(|clock| {
                save_allocation_counters::read_counters(
                    &memory,
                    shared.base,
                    clock.game_state as usize,
                )
            });
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
            frame.wars_snapshot_sampled = false;
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
            frame.extended_raw_mismatches = [false; TABLE_COUNT];
            frame.country_cache_status = SeedStatus::Unavailable {
                reason: "clock unavailable",
            };
            frame.country_cache_entries = 0;
            frame.country_cache_error = None;
            frame.country_cache_ns = 0;
            frame.order_counters.reset();
            let needed = unsafe { (&*shared.consistency.get()).needed() };
            let extended_needed = unsafe { (&*shared.extended_consistency.get()).needed() };
            let mut matched = [false; CONSISTENCY_TABLES.len()];
            let mut extended_matched = [false; TABLE_COUNT];
            // Relations handoff state: same FIFO, same ordering guarantees.
            let mut relations_payload: Option<extended::intel::RelationsJob> = None;
            let mut relations_report = extended::intel::RelationsCopyReport::default();
            // Characters handoff state: same FIFO, same ordering guarantees.
            let mut characters_payload: Option<extended::characters::CharactersJob> = None;
            let mut characters_report = extended::characters::CharactersCopyReport::default();
            // Projects handoff state: same FIFO, same ordering guarantees.
            let mut projects_payload: Option<extended::projects::ProjectsJob> = None;
            let mut projects_report = extended::projects::ProjectsCopyReport::default();
            // Lexical handoff state: same FIFO, same ordering guarantees.
            let mut lexical_payload: Option<extended::lexical::LexicalJob> = None;
            let mut lexical_report = extended::lexical::LexicalCopyReport::default();
            // Divisions handoff state: same FIFO, same ordering guarantees.
            // The hourly payload carries the divisions walk plus the
            // equipment/upgrade detail walks sharing its census.
            let mut divisions_payload: Option<DivisionsJob> = None;
            let mut divisions_report = DivisionsCopyReport::default();
            // Typed Wars keep their independent matrix consistency window.
            let mut wars_matrix_check = false;
            // Focus handoff state: same FIFO, same ordering guarantees. The
            // hourly payload carries the summary walk plus the completed
            // roster; staging overflow falls back to the typed walk inside
            // governance capture with no payload enqueued.
            let mut focus_payload: Option<extended::governance::FocusJob> = None;
            let mut focus_report = extended::governance::FocusCopyReport::default();
            let mut air_wing_payload: Option<extended::air::AirWingJob> = None;
            let mut air_wing_report = extended::air::AirWingCopyReport::default();
            let mut air_stats_payload: Option<extended::air::AirStatsJob> = None;
            let mut air_stats_report = extended::air::AirStatsCopyReport::default();
            let mut naval_payload: Option<extended::naval_history::NavalJob> = None;
            let mut naval_report = extended::naval_history::NavalCopyReport::default();
            // Players pair handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the independent roster
            // and controller walks with its own root refs and no
            // cross-family decoded roots. Staging overflow falls back to the
            // typed walks with no payload enqueued.
            let mut players_payload: Option<crate::map_capture::PlayersJob> = None;
            let mut players_report = crate::map_capture::PlayersCopyReport::default();
            // Session family handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the manager, sources
            // and aggregates with its own root refs and no cross-family
            // decoded roots. Staging overflow falls back to the typed walks
            // with no payload enqueued.
            let mut session_payload: Option<extended::session::SessionJob> = None;
            let mut session_report = extended::session::SessionCopyReport::default();
            // Production hourly rest handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries lines, stockpiles,
            // licences, the deployment queue and licence records with its own
            // census and no cross-family decoded roots. Staging overflow
            // falls back to the typed walks with no payload enqueued.
            let mut production_rest_payload: Option<extended::production::ProductionRestJob> = None;
            let mut production_rest_report =
                extended::production::ProductionRestCopyReport::default();
            // Construction hourly rest handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries resources, the
            // construction queue, occupation, resistance and collaboration
            // with its own census and no cross-family decoded roots. Staging
            // overflow falls back to the typed walks with no payload enqueued.
            let mut construction_rest_payload: Option<extended::construction::ConstructionRestJob> =
                None;
            let mut construction_rest_report =
                extended::construction::ConstructionRestCopyReport::default();
            // Economy hourly rest handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries fuel, experience,
            // supply, hubs, disrupted, foreign, definitions, resources,
            // exchanges and purchases with its own census and no
            // cross-family decoded roots. Staging overflow falls back to the
            // typed walks with no payload enqueued.
            let mut economy_rest_payload: Option<extended::economy::EconomyRestJob> = None;
            let mut economy_rest_report = extended::economy::EconomyRestCopyReport::default();
            // Shared hourly bases handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the wing/manager census
            // once with bases, registrations and aces detail; all three
            // decodes read the shared spans. Staging overflow falls back to
            // the typed walks with no payload enqueued.
            let mut air_bases_payload: Option<extended::air::AirBasesJob> = None;
            let mut air_bases_report = extended::air::AirBasesCopyReport::default();
            // Shared hourly history handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the regional walk once
            // with history and damage detail; both decodes read the shared
            // spans. Staging overflow falls back to the typed walk with no
            // payload enqueued.
            let mut air_history_payload: Option<extended::air::AirHistoryJob> = None;
            let mut air_history_report = extended::air::AirHistoryCopyReport::default();
            // Hourly supremacy handoff state: same FIFO, same ordering
            // guarantees. Staging overflow falls back to the typed walk with
            // no payload enqueued.
            let mut supremacy_payload: Option<extended::naval_history::SupremacyJob> = None;
            let mut supremacy_report = extended::naval_history::SupremacyCopyReport::default();
            // Hourly sunk-ships handoff state: same FIFO, same ordering
            // guarantees. Staging overflow falls back to the typed walk with
            // no payload enqueued.
            let mut sunk_payload: Option<extended::naval_history::SunkJob> = None;
            let mut sunk_report = extended::naval_history::SunkCopyReport::default();
            // Hourly radar handoff state: same FIFO, same ordering
            // guarantees. Staging overflow falls back to the typed walk with
            // no payload enqueued.
            let mut radar_payload: Option<extended::fog::RadarJob> = None;
            let mut radar_report = extended::fog::RadarCopyReport::default();
            // Hourly intel handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the walk's own
            // division/fleet census once with viewer detail; both unit paths
            // decode from the shared spans. Staging overflow falls back to
            // the typed walk with no payload enqueued.
            let mut intel_payload: Option<extended::fog::IntelJob> = None;
            let mut intel_report = extended::fog::IntelCopyReport::default();
            // Hourly weather handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the manager header once
            // with limits, regions and provinces detail; all three decodes
            // read the shared spans. Staging overflow falls back to the typed
            // walks with no payload enqueued.
            let mut weather_payload: Option<extended::weather::WeatherJob> = None;
            let mut weather_report = extended::weather::WeatherCopyReport::default();
            // Hourly capitulation handoff state: same FIFO, same ordering
            // guarantees. The hourly payload carries the country census once
            // with diplomacy detail; the decode reads the shared spans.
            // Staging overflow falls back to the typed walk with no payload
            // enqueued.
            let mut capitulation_payload: Option<extended::intel::CapitulationJob> = None;
            let mut capitulation_report = extended::intel::CapitulationCopyReport::default();
            let mut decode_hour = 0;
            let mut decode_force_full = false;
            if let Some(clock) = frame.clock_before {
                let force_full = unsafe { (&mut *shared.cadence.get()).force_full(&clock) };
                let state = clock.game_state as usize;
                // Owned whole-family jobs may run concurrently with these main
                // reads. No frame, cache or live-reader reference crosses threads.
                // Every submitted job is recovered before its own fallback,
                // trailing verification, publication or return to game code.
                let acquisition = shared.acquisition.get().expect("initialized before hook");
                let dispatch_begin = qpc();
                let characters_refresh = decode::characters_refresh_hour(clock.hour, force_full);
                let mut world_request = None;
                let mut relations_request = None;
                if decode::relations_refresh_hour(clock.hour, force_full) {
                    let needed_any = decode::RELATIONS
                        .iter()
                        .any(|table| extended_needed[table.index()]);
                    let mut first = decode::acquire_relations_job();
                    let second = needed_any.then(decode::acquire_relations_job);
                    if second
                        .as_ref()
                        .is_some_and(|job| job.countries.capacity() == 0)
                    {
                        decode::recycle_relations_job(first);
                        first = Default::default();
                    }
                    let relations = frame.extended.relations_pilot();
                    relations_request = Some(acquisition::RelationsRequest {
                        context: extended::intel::RelationsCopyContext {
                            base: shared.base,
                            game_state: state,
                            opinion_cap: relations.opinion_rows_cap(),
                            tuple_cap: relations.tuple_rows_cap(),
                        },
                        first,
                        second,
                    });
                }
                let mut characters_request = None;
                if characters_refresh {
                    let needed_any = decode::CHARACTERS
                        .iter()
                        .any(|table| extended_needed[table.index()]);
                    let mut first = decode::acquire_characters_job();
                    let second = needed_any.then(decode::acquire_characters_job);
                    if second
                        .as_ref()
                        .is_some_and(|job| job.log.spans.capacity() == 0)
                    {
                        decode::recycle_characters_job(first);
                        first = Default::default();
                    }
                    characters_request = Some(acquisition::CharactersRequest {
                        context: extended::characters::CharactersCopyContext {
                            base: shared.base,
                            game_state: state,
                        },
                        first,
                        second,
                    });
                } else {
                    world_request = Some(acquire_world_request(
                        shared.base,
                        state,
                        &extended_needed,
                        frame.extended.focus_pilot().focus_copy_context(
                            shared.base,
                            state,
                            decode::FOCUS.map(|table| extended_needed[table.index()]),
                        ),
                        frame.extended.economy_rest_pilot(),
                    ));
                }
                // Safety: the proven sole hourly callback retains the game
                // boundary until both finish calls below end every live lease.
                let relations_dispatched = relations_request.is_some()
                    && unsafe { acquisition.begin_relations(&mut relations_request) };
                let characters_dispatched = characters_request.is_some()
                    && unsafe { acquisition.begin_characters(&mut characters_request) };
                let world_dispatched = world_request.is_some()
                    && unsafe { acquisition.begin_world(&mut world_request) };
                shared
                    .acquisition_dispatch_ns
                    .fetch_add(shared.ns(qpc() - dispatch_begin), Ordering::Relaxed);
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
                // Divisions group (copy/decode split): raw copies only on
                // the game thread. Every hour stages the payload and hands
                // the frame to the decode FIFO; the worker replays the
                // divisions walk (collecting the shared sources), then
                // equipment and upgrades against the decoded sources, before
                // the spool ready handoff. Staging overflow falls back to
                // the typed walk below with no payload enqueued.
                let mut divisions_first = decode::acquire_divisions_job();
                let mut divisions_second = (needed[2]
                    || extended::Captured::needs_division_check(&extended_needed))
                .then(decode::acquire_divisions_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if divisions_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_divisions_job(divisions_first);
                    divisions_first = Default::default();
                }
                let divisions_ctx = DivisionsCopyContext {
                    base: shared.base,
                    game_state: state,
                };
                divisions_report = copy_divisions_refresh(
                    &memory,
                    divisions_ctx,
                    &mut divisions_first,
                    divisions_second.as_mut(),
                );
                frame
                    .extended
                    .set_divisions_raw_mode(divisions_report.raw_ok);
                if divisions_report.raw_ok {
                    let source_started = qpc();
                    divisions_first.supply_sources(
                        &mut frame.extended,
                        frame.divisions.len(),
                        false,
                    );
                    if let Some(second) = divisions_second.as_ref() {
                        second.supply_sources(
                            &mut frame.extended,
                            frame.divisions_second.len(),
                            true,
                        );
                    }
                    divisions_report.copy_ticks[0] += elapsed(source_started);
                    divisions_payload = Some(divisions_first);
                    matched[2] = divisions_report.matched[0];
                    if needed[2] {
                        frame.consistency_checked[2] = true;
                        // Binary per refresh: tearing detected (0/1), not a
                        // row count. Spool gates test `==0`/`!=0` only.
                        frame.divisions_mismatches = usize::from(!divisions_report.matched[0]);
                    }
                    let divisions_ns = shared.ns(divisions_report.copy_ticks[0] as i64);
                    frame.table_ns[6] = divisions_ns;
                    decode::add_divisions_copy_ns(0, divisions_ns);
                    decode::add_divisions_raw_bytes(0, divisions_report.raw_bytes[0]);
                } else {
                    let begin = qpc();
                    let mut division_source_ticks = 0_u64;
                    decode::recycle_divisions_job(divisions_first);
                    frame.extended.set_divisions_raw_mode(false);
                    frame.divisions_result = carmy::walk_with(
                        &cached,
                        shared.base,
                        state,
                        &mut frame.divisions,
                        |pointer, row| {
                            division_source_ticks +=
                                frame.extended.visit_division(pointer, row, false);
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
                            && full_match(
                                frame.divisions_result,
                                second,
                                frame.divisions_mismatches,
                            );
                    }
                    frame.table_ns[6] =
                        shared.ns((qpc() - begin).saturating_sub(division_source_ticks as i64));
                }
                if let Some(second) = divisions_second {
                    decode::recycle_divisions_job(second);
                }
                // ETA owns copies of both authenticated source prefixes and
                // its own output/cache buffers. The frame remains game-thread
                // owned; the worker must return before final verification.
                let eta_dispatch_begin = qpc();
                let mut eta_request = Some(frame.extended.take_eta_request(
                    shared.base,
                    state,
                    extended_needed[replay::rows::TableId::DivisionEta.index()],
                ));
                // SAFETY: this callback unconditionally finishes accepted ETA
                // work below before its clock check or decode publication.
                let eta_dispatched = unsafe { acquisition.begin_eta(&mut eta_request) };
                if !eta_dispatched {
                    frame.extended.restore_eta_request(
                        eta_request.take().expect("rejected ETA request retained"),
                        None,
                    );
                }
                shared
                    .acquisition_dispatch_ns
                    .fetch_add(shared.ns(qpc() - eta_dispatch_begin), Ordering::Relaxed);
                // These independent core tables do not feed the division
                // census or ETA: both authenticate their own native roots.
                // Retain the same reads/checks, now overlapping ETA acquisition.
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
                frame.states_result =
                    cstate::walk_with(&memory, shared.base, state, &mut frame.states, |source| {
                        frame.extended.visit_state(source, false)
                    });
                frame.extended.finish_states(frame.states_result, false);
                if needed[1] || extended::Captured::needs_state_check(&extended_needed) {
                    let second = cstate::walk_with(
                        &memory,
                        shared.base,
                        state,
                        &mut frame.states_second,
                        |source| frame.extended.visit_state(source, true),
                    );
                    frame.extended.finish_states(second, true);
                    if needed[1] {
                        frame.consistency_checked[1] = true;
                        frame.states_mismatches = mismatches(
                            frame.states_result,
                            &frame.states,
                            second,
                            &frame.states_second,
                        );
                        matched[1] =
                            full_match(frame.states_result, second, frame.states_mismatches);
                    }
                }
                frame.table_ns[5] = shared.ns(qpc() - begin);
                // The optimized typed walk already batches relation roots and
                // reuses callback-local country authentication. No raw Wars
                // payload, duplicate mirror walk or decode index is constructed.
                let matrix_needed = shared.war_matrix_remaining.load(Ordering::Relaxed) != 0;
                let begin = qpc();
                frame.wars_snapshot_sampled = true;
                let war_read = cwar_relation::walk_verified(
                    &cached,
                    &memory,
                    shared.base,
                    state,
                    &mut frame.wars,
                    &mut frame.wars_snapshot,
                );
                frame.wars_result = war_read.outcome;
                if needed[3] {
                    frame.consistency_checked[3] = true;
                    let second =
                        cwar_relation::walk(&cached, shared.base, state, &mut frame.wars_second);
                    frame.wars_mismatches =
                        mismatches(frame.wars_result, &frame.wars, second, &frame.wars_second);
                    frame.wars_mismatches += usize::from(war_read.verification_failed);
                    matched[3] = !war_read.verification_failed
                        && full_match(frame.wars_result, second, frame.wars_mismatches);
                }
                // Retain distinct owned/table and independent-matrix costs.
                frame.table_ns[7] = shared.ns(qpc() - begin);
                if matrix_needed {
                    let matrix_begin = qpc();
                    let matrix = cwar_relation::walk_matrix(
                        &cached,
                        shared.base,
                        state,
                        &mut frame.wars_matrix,
                    );
                    frame.war_matrix_ns = shared.ns(qpc() - matrix_begin);
                    frame.war_matrix_result = Some(matrix);
                    wars_matrix_check = matches!(
                        (frame.wars_result, matrix),
                        (Outcome::Rows { .. }, Outcome::Rows { .. })
                    ) && frame.wars_mismatches == 0;
                }
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
                // Players pair (copy/decode split): raw copies only on the
                // game thread. Every hour stages the payload and hands the
                // frame to the decode FIFO; the worker replays both walks
                // before the spool ready handoff. Staging overflow falls back
                // to the typed walks below with no payload enqueued.
                let players_second_needed = needed[6] || needed[7];
                let mut players_first = decode::acquire_players_job();
                let mut players_second = players_second_needed.then(decode::acquire_players_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if players_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_players_job(players_first);
                    players_first = Default::default();
                }
                let players_ctx = crate::map_capture::PlayersCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [needed[6], needed[7]],
                };
                players_report = frame.map.copy_players_refresh(
                    &memory,
                    players_ctx,
                    &mut players_first,
                    players_second.as_mut(),
                );
                frame.map.players_raw_mode = players_report.raw_ok;
                if players_report.raw_ok {
                    players_payload = Some(players_first);
                } else {
                    decode::recycle_players_job(players_first);
                }
                if let Some(second) = players_second {
                    decode::recycle_players_job(second);
                }
                let naval_second_needed = decode::NAVAL_ORDER
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut naval_first = decode::acquire_naval_job();
                let mut naval_second = naval_second_needed.then(decode::acquire_naval_job);
                if naval_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_naval_job(naval_first);
                    naval_first = Default::default();
                }
                let air_second_needed = needed[14]
                    || decode::AIR_WING
                        .iter()
                        .chain(decode::NAVAL_ORDER.iter())
                        .any(|table| extended_needed[table.index()]);
                let mut air_first = decode::acquire_air_wing_job();
                let mut air_second = air_second_needed.then(decode::acquire_air_wing_job);
                let wing_limits = frame.extended.air_wing_pilot().wing_limits();
                let air_ctx = extended::air::AirWingCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: decode::AIR_WING.map(|table| extended_needed[table.index()]),
                    limits: [
                        frame.map.air_wings.rows.len(),
                        wing_limits[0],
                        wing_limits[1],
                    ],
                    naval_limits: [
                        naval_first.wing_limit(),
                        naval_second.as_ref().map_or(0, |job| job.wing_limit()),
                    ],
                };
                air_wing_report = extended::air::copy_air_refresh(
                    &memory,
                    air_ctx,
                    &mut air_first,
                    air_second.as_mut(),
                );
                if air_wing_report.raw_ok {
                    air_wing_payload = Some(air_first);
                } else {
                    decode::recycle_air_wing_job(air_first);
                }
                // Hourly air statistics (copy/decode split): raw copies only
                // on the game thread, same FIFO and ordering guarantees as
                // the wing pair above. Stages its own strategic-air census
                // copies with no cross-family decoded roots. Staging overflow
                // falls back to the typed walk with no payload enqueued.
                let air_stats_second_needed = decode::AIR_STATS
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut air_stats_first = decode::acquire_air_stats_job();
                let mut air_stats_second =
                    air_stats_second_needed.then(decode::acquire_air_stats_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if air_stats_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_air_stats_job(air_stats_first);
                    air_stats_first = Default::default();
                }
                let air_stats_ctx = extended::air::AirStatsCopyContext {
                    base: shared.base,
                    game_state: state,
                };
                air_stats_report = extended::air::copy_air_stats_refresh(
                    &memory,
                    air_stats_ctx,
                    &mut air_stats_first,
                    air_stats_second.as_mut(),
                );
                frame.extended.air_stats_pilot().stats_raw_mode = air_stats_report.raw_ok;
                if air_stats_report.raw_ok {
                    air_stats_payload = Some(air_stats_first);
                } else {
                    decode::recycle_air_stats_job(air_stats_first);
                }
                if let Some(second) = air_stats_second {
                    decode::recycle_air_stats_job(second);
                }
                let naval_ctx = extended::naval_history::NavalCopyContext {
                    base: shared.base,
                    game_state: state,
                };
                let mut naval_install_ticks = 0_u64;
                if let Some(wings) = air_wing_payload.as_ref() {
                    naval_install_ticks += naval_first.install_wings(wings);
                }
                if let Some(naval) = naval_second.as_mut()
                    && let Some(wings) = air_second.as_ref()
                {
                    naval_install_ticks += naval.install_wings(wings);
                }
                naval_report = extended::naval_history::copy_naval_refresh(
                    &memory,
                    naval_ctx,
                    &mut naval_first,
                    naval_second.as_mut(),
                );
                if naval_report.raw_ok {
                    naval_report.copy_ticks[0] += naval_install_ticks;
                }
                if let Some(wings) = air_wing_payload.as_ref() {
                    naval_report.copy_ticks[0] += wings.membership_ticks;
                    if let Some(second) = air_second.as_ref() {
                        naval_report.copy_ticks[0] += second.membership_ticks;
                    }
                    air_wing_report.copy_ticks[0] += frame.map.install_owned_air(wings);
                    if !naval_report.raw_ok {
                        frame
                            .extended
                            .naval_pilot()
                            .add_wing_source_ticks(naval_install_ticks);
                        frame.extended.naval_pilot().install_map_wings(wings, false);
                        if let Some(second) = air_second.as_ref() {
                            frame.extended.naval_pilot().install_map_wings(second, true);
                        }
                    }
                }
                frame.extended.naval_pilot().raw_mode = naval_report.raw_ok;
                if naval_report.raw_ok {
                    naval_payload = Some(naval_first);
                } else {
                    decode::recycle_naval_job(naval_first);
                }
                if let Some(second) = naval_second {
                    decode::recycle_naval_job(second);
                }
                if let Some(second) = air_second {
                    decode::recycle_air_wing_job(second);
                }
                // Shared hourly bases (copy/decode split): raw copies only
                // on the game thread, same FIFO and ordering guarantees as
                // the packs above. The wing/manager census is staged once
                // with bases, registrations and aces detail; all three
                // decodes read the shared spans. Staging overflow falls back
                // to the typed walks with no payload enqueued.
                let air_bases_second_needed = decode::AIR_BASES
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut air_bases_first = decode::acquire_air_bases_job();
                let mut air_bases_second =
                    air_bases_second_needed.then(decode::acquire_air_bases_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if air_bases_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_air_bases_job(air_bases_first);
                    air_bases_first = Default::default();
                }
                let air_bases_ctx = extended::air::AirBasesCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [
                        extended_needed[decode::AIR_BASES[0].index()],
                        extended_needed[decode::AIR_BASES[1].index()],
                        extended_needed[decode::AIR_BASES[2].index()],
                    ],
                };
                air_bases_report = frame.extended.copy_air_bases_refresh(
                    &memory,
                    air_bases_ctx,
                    &mut air_bases_first,
                    air_bases_second.as_mut(),
                );
                frame.extended.air_bases_pilot().bases_raw_mode = air_bases_report.raw_ok;
                // The registrations half lives in the fog pack and decodes
                // from the same shared log.
                frame.extended.radar_pilot().registrations_raw_mode = air_bases_report.raw_ok;
                if air_bases_report.raw_ok {
                    air_bases_payload = Some(air_bases_first);
                } else {
                    decode::recycle_air_bases_job(air_bases_first);
                }
                if let Some(second) = air_bases_second {
                    decode::recycle_air_bases_job(second);
                }
                // One hourly native walk owns history/damage rows. The FIFO
                // worker installs them without an address index or re-walk.
                // Both row buffers are preflighted before acquisition; missing
                // storage uses the typed fallback. Witness overflow only fails
                // checks and grows off-thread, never rereads the source.
                let air_history_second_needed = decode::AIR_HISTORY
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut air_history_first = decode::acquire_air_history_job();
                let mut air_history_second =
                    air_history_second_needed.then(decode::acquire_air_history_job);
                let air_history_ctx = extended::air::AirHistoryCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [
                        extended_needed[decode::AIR_HISTORY[0].index()],
                        extended_needed[decode::AIR_HISTORY[1].index()],
                    ],
                };
                air_history_report = frame.extended.air_history_pilot().copy_air_history_refresh(
                    &memory,
                    air_history_ctx,
                    &mut air_history_first,
                    air_history_second.as_mut(),
                );
                frame.extended.air_history_pilot().history_raw_mode = air_history_report.raw_ok;
                if air_history_report.raw_ok {
                    air_history_payload = Some(air_history_first);
                } else {
                    decode::recycle_air_history_job(air_history_first);
                }
                if let Some(second) = air_history_second {
                    decode::recycle_air_history_job(second);
                }
                // Hourly supremacy (copy/decode split): raw copies only on
                // the game thread, same FIFO and ordering guarantees. Staging
                // overflow falls back to the typed walk with no payload
                // enqueued.
                let supremacy_second_needed = decode::SUPREMACY
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut supremacy_first = decode::acquire_supremacy_job();
                let mut supremacy_second =
                    supremacy_second_needed.then(decode::acquire_supremacy_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if supremacy_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_supremacy_job(supremacy_first);
                    supremacy_first = Default::default();
                }
                let supremacy_ctx = extended::naval_history::SupremacyCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [extended_needed[decode::SUPREMACY[0].index()]],
                };
                supremacy_report = frame.extended.supremacy_pilot().copy_supremacy_refresh(
                    &memory,
                    supremacy_ctx,
                    &mut supremacy_first,
                    supremacy_second.as_mut(),
                );
                frame.extended.supremacy_pilot().supremacy_raw_mode = supremacy_report.raw_ok;
                if supremacy_report.raw_ok {
                    supremacy_payload = Some(supremacy_first);
                } else {
                    decode::recycle_supremacy_job(supremacy_first);
                }
                if let Some(second) = supremacy_second {
                    decode::recycle_supremacy_job(second);
                }
                // Hourly sunk-ships (copy/decode split): raw copies only on
                // the game thread, same FIFO and ordering guarantees. Staging
                // overflow falls back to the typed walk with no payload
                // enqueued.
                let sunk_second_needed = decode::SUNK
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut sunk_first = decode::acquire_sunk_job();
                let mut sunk_second = sunk_second_needed.then(decode::acquire_sunk_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if sunk_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_sunk_job(sunk_first);
                    sunk_first = Default::default();
                }
                let sunk_ctx = extended::naval_history::SunkCopyContext {
                    base: shared.base,
                    game_state: state,
                    hour: clock.hour,
                    needed: [extended_needed[decode::SUNK[0].index()]],
                };
                sunk_report = frame.extended.sunk_pilot().copy_sunk_refresh(
                    &memory,
                    sunk_ctx,
                    &mut sunk_first,
                    sunk_second.as_mut(),
                );
                frame.extended.sunk_pilot().sunk_raw_mode = sunk_report.raw_ok;
                if sunk_report.raw_ok {
                    sunk_payload = Some(sunk_first);
                } else {
                    decode::recycle_sunk_job(sunk_first);
                }
                if let Some(second) = sunk_second {
                    decode::recycle_sunk_job(second);
                }
                // Hourly radar (copy/decode split): raw copies only on the
                // game thread, same FIFO and ordering guarantees. Staging
                // overflow falls back to the typed walk with no payload
                // enqueued.
                let radar_second_needed = decode::RADAR
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut radar_first = decode::acquire_radar_job();
                let mut radar_second = radar_second_needed.then(decode::acquire_radar_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if radar_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_radar_job(radar_first);
                    radar_first = Default::default();
                }
                let radar_ctx = extended::fog::RadarCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [extended_needed[decode::RADAR[0].index()]],
                };
                radar_report = frame.extended.radar_pilot().copy_radar_refresh(
                    &memory,
                    radar_ctx,
                    &mut radar_first,
                    radar_second.as_mut(),
                );
                frame.extended.radar_pilot().radar_raw_mode = radar_report.raw_ok;
                if radar_report.raw_ok {
                    radar_payload = Some(radar_first);
                } else {
                    decode::recycle_radar_job(radar_first);
                }
                if let Some(second) = radar_second {
                    decode::recycle_radar_job(second);
                }
                // Hourly intel (copy/decode split): raw copies only on the
                // game thread, same FIFO and ordering guarantees. The walk's
                // own division/fleet census is staged once with viewer
                // detail; both unit paths decode from the shared spans.
                // Staging overflow falls back to the typed walk with no
                // payload enqueued.
                let intel_second_needed = decode::INTEL
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut intel_first = decode::acquire_intel_job();
                let mut intel_second = intel_second_needed.then(decode::acquire_intel_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if intel_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_intel_job(intel_first);
                    intel_first = Default::default();
                }
                let intel_ctx = extended::fog::IntelCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [extended_needed[decode::INTEL[0].index()]],
                };
                intel_report = frame.extended.intel_pilot().copy_intel_refresh(
                    &memory,
                    intel_ctx,
                    &mut intel_first,
                    intel_second.as_mut(),
                );
                frame.extended.intel_pilot().intel_raw_mode = intel_report.raw_ok;
                if intel_report.raw_ok {
                    intel_payload = Some(intel_first);
                } else {
                    decode::recycle_intel_job(intel_first);
                }
                if let Some(second) = intel_second {
                    decode::recycle_intel_job(second);
                }
                // Hourly weather (copy/decode split): raw copies only on the
                // game thread, same FIFO and ordering guarantees. The manager
                // header is staged once with limits, regions and provinces
                // detail; all three decodes read the shared spans. Staging
                // overflow falls back to the typed walks with no payload
                // enqueued.
                let weather_second_needed = decode::WEATHER
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut weather_first = decode::acquire_weather_job();
                let mut weather_second = weather_second_needed.then(decode::acquire_weather_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if weather_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_weather_job(weather_first);
                    weather_first = Default::default();
                }
                let weather_ctx = extended::weather::WeatherCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [
                        extended_needed[decode::WEATHER[0].index()],
                        extended_needed[decode::WEATHER[1].index()],
                        extended_needed[decode::WEATHER[2].index()],
                    ],
                };
                weather_report = frame.extended.weather_pilot().copy_weather_refresh(
                    &memory,
                    weather_ctx,
                    &mut weather_first,
                    weather_second.as_mut(),
                );
                frame.extended.weather_pilot().raw_mode = weather_report.raw_ok;
                if weather_report.raw_ok {
                    weather_payload = Some(weather_first);
                } else {
                    decode::recycle_weather_job(weather_first);
                }
                if let Some(second) = weather_second {
                    decode::recycle_weather_job(second);
                }
                // Hourly capitulation (copy/decode split): raw copies only on
                // the game thread, same FIFO and ordering guarantees. The
                // country census is staged once with diplomacy detail.
                // Staging overflow falls back to the typed walk with no
                // payload enqueued.
                let capitulation_second_needed = decode::CAPITULATION
                    .iter()
                    .any(|table| extended_needed[table.index()]);
                let mut capitulation_first = decode::acquire_capitulation_job();
                let mut capitulation_second =
                    capitulation_second_needed.then(decode::acquire_capitulation_job);
                // A consistency pass needs both owned buffers; otherwise use
                // the allocation-free typed double read.
                if capitulation_second
                    .as_ref()
                    .is_some_and(|job| job.log.spans.capacity() == 0)
                {
                    decode::recycle_capitulation_job(capitulation_first);
                    capitulation_first = Default::default();
                }
                let capitulation_ctx = extended::intel::CapitulationCopyContext {
                    base: shared.base,
                    game_state: state,
                    needed: [extended_needed[decode::CAPITULATION[0].index()]],
                };
                capitulation_report = frame
                    .extended
                    .capitulation_pilot()
                    .copy_capitulation_refresh(
                        &memory,
                        capitulation_ctx,
                        &mut capitulation_first,
                        capitulation_second.as_mut(),
                    );
                frame.extended.capitulation_pilot().capitulation_raw_mode =
                    capitulation_report.raw_ok;
                if capitulation_report.raw_ok {
                    capitulation_payload = Some(capitulation_first);
                } else {
                    decode::recycle_capitulation_job(capitulation_first);
                }
                if let Some(second) = capitulation_second {
                    decode::recycle_capitulation_job(second);
                }
                let (map_matched, map_ticks) = frame.map.sample(
                    &cached,
                    shared.base,
                    state,
                    &needed[6..PROVINCE_CHECK],
                    &mut frame.extended.sources(
                        &cached,
                        shared.base,
                        &extended_needed,
                        air_wing_report.raw_ok,
                        naval_report.raw_ok,
                    ),
                );
                matched[6..PROVINCE_CHECK].copy_from_slice(&map_matched);
                frame.consistency_checked[6..PROVINCE_CHECK]
                    .copy_from_slice(&needed[6..PROVINCE_CHECK]);
                for (total, ticks) in frame.table_ns[10..].iter_mut().zip(map_ticks) {
                    *total = shared.ns(ticks as i64);
                }
                decode_hour = clock.hour;
                decode_force_full = force_full;
                // Projects family (copy/decode split): raw copies only on the
                // game thread. Refresh hours stage the payload and hand the
                // frame to the decode FIFO; the worker decodes before the
                // spool ready handoff. Staging overflow falls back to the
                // typed walk below with no payload enqueued.
                let projects_refresh = decode::projects_refresh_hour(clock.hour, force_full);
                if projects_refresh {
                    let needed_any = decode::PROJECTS
                        .iter()
                        .any(|table| extended_needed[table.index()]);
                    let mut first = decode::acquire_projects_job();
                    let mut second = needed_any.then(decode::acquire_projects_job);
                    // A consistency pass needs both owned buffers; otherwise use
                    // the allocation-free typed double read.
                    if second
                        .as_ref()
                        .is_some_and(|job| job.log.spans.capacity() == 0)
                    {
                        decode::recycle_projects_job(first);
                        first = Default::default();
                    }
                    let projects = frame.extended.projects_pilot();
                    let ctx = extended::projects::ProjectsCopyContext {
                        base: shared.base,
                        game_state: state,
                        needed: extended_needed,
                    };
                    projects_report =
                        projects.copy_projects_refresh(&memory, ctx, &mut first, second.as_mut());
                    frame.extended.projects_pilot().raw_mode = projects_report.raw_ok;
                    if projects_report.raw_ok {
                        projects_payload = Some(first);
                    } else {
                        decode::recycle_projects_job(first);
                    }
                    if let Some(second) = second {
                        decode::recycle_projects_job(second);
                    }
                }
                // Lexical family (copy/decode split): the witness is read
                // game-side as the copy gate; refresh hours stage the full
                // table payload and hand the frame to the decode FIFO.
                // Staging overflow falls back to the typed walk below with no
                // payload enqueued.
                let lexical_refresh = decode::lexical_refresh_hour(clock.hour, force_full);
                if lexical_refresh {
                    let needed_any = decode::LEXICAL
                        .iter()
                        .any(|table| extended_needed[table.index()]);
                    let mut first = decode::acquire_lexical_job();
                    let mut second = needed_any.then(decode::acquire_lexical_job);
                    // A consistency pass needs both owned buffers; otherwise use
                    // the allocation-free typed double read.
                    if second.as_ref().is_some_and(|job| !job.initialized()) {
                        decode::recycle_lexical_job(first);
                        first = Default::default();
                    }
                    let lexical = frame.extended.lexical_pilot();
                    let ctx = extended::lexical::LexicalCopyContext {
                        base: shared.base,
                        needed: extended_needed,
                    };
                    lexical_report =
                        lexical.copy_lexical_refresh(&memory, ctx, &mut first, second.as_mut());
                    frame.extended.lexical_pilot().raw_mode = lexical_report.raw_ok;
                    if lexical_report.raw_ok {
                        lexical_payload = Some(first);
                    } else {
                        decode::recycle_lexical_job(first);
                    }
                    if let Some(second) = second {
                        decode::recycle_lexical_job(second);
                    }
                }
                // Session family (copy/decode split): raw copies only on the
                // game thread. Every hour stages the payload and hands the
                // frame to the decode FIFO; the worker replays all three
                // walks before the spool ready handoff. Staging overflow falls
                // back to the typed walks below with no payload enqueued.
                {
                    let needed_any = decode::SESSION
                        .iter()
                        .any(|table| extended_needed[table.index()]);
                    let mut first = decode::acquire_session_job();
                    let mut second = needed_any.then(decode::acquire_session_job);
                    // A consistency pass needs both owned buffers; otherwise use
                    // the allocation-free typed double read.
                    if second
                        .as_ref()
                        .is_some_and(|job| job.log.spans.capacity() == 0)
                    {
                        decode::recycle_session_job(first);
                        first = Default::default();
                    }
                    let session = frame.extended.session_pilot();
                    let ctx = extended::session::SessionCopyContext {
                        base: shared.base,
                        game_state: state,
                        needed: [
                            extended_needed[decode::SESSION[0].index()],
                            extended_needed[decode::SESSION[1].index()],
                            extended_needed[decode::SESSION[2].index()],
                        ],
                    };
                    session_report =
                        session.copy_session_refresh(&memory, ctx, &mut first, second.as_mut());
                    frame.extended.session_pilot().raw_mode = session_report.raw_ok;
                    if session_report.raw_ok {
                        session_payload = Some(first);
                    } else {
                        decode::recycle_session_job(first);
                    }
                    if let Some(second) = second {
                        decode::recycle_session_job(second);
                    }
                }
                // On non-Characters hours the same existing worker acquired
                // this owned World bundle while the core/map work ran. H0 and
                // forced refresh retain the current synchronous acquisition;
                // rejected dispatch keeps its exact owned buffers for retry.
                let (world, report, world_failure) = if world_dispatched {
                    let join_begin = qpc();
                    let completed = acquisition.finish_world().expect("submitted World job");
                    shared
                        .acquisition_wait_ns
                        .fetch_add(shared.ns(qpc() - join_begin), Ordering::Relaxed);
                    let failure = match completed.report {
                        Ok(report) => report.error,
                        Err(acquisition::Failure::Panicked) => {
                            Some("World acquisition worker panicked")
                        }
                        Err(acquisition::Failure::Stopped) => {
                            Some("World acquisition worker stopped before reading")
                        }
                    };
                    (
                        completed.request,
                        completed.report.unwrap_or_default(),
                        failure,
                    )
                } else {
                    let mut request = world_request.take().unwrap_or_else(|| {
                        acquire_world_request(
                            shared.base,
                            state,
                            &extended_needed,
                            frame.extended.focus_pilot().focus_copy_context(
                                shared.base,
                                state,
                                decode::FOCUS.map(|table| extended_needed[table.index()]),
                            ),
                            frame.extended.economy_rest_pilot(),
                        )
                    });
                    let report = acquisition::run_world(&mut request, &memory);
                    (request, report, None)
                };
                production_rest_report = report.production;
                construction_rest_report = report.construction;
                economy_rest_report = report.economy;
                focus_report = report.focus;
                // A terminal worker fault suppresses typed retries and its raw
                // payloads; the affected tables receive explicit unavailability
                // after independent capture. Ordinary staging overflow retains
                // the established per-family typed fallback.
                let terminal = world_failure.is_some();
                frame
                    .extended
                    .production_rest_pilot()
                    .apply_production_rest_report(
                        world.production.context.needed,
                        &production_rest_report,
                    );
                frame.extended.production_rest_pilot().rest_raw_mode =
                    terminal || production_rest_report.raw_ok;
                frame
                    .extended
                    .construction_rest_pilot()
                    .apply_construction_rest_report(
                        world.construction.context.needed,
                        &construction_rest_report,
                    );
                frame.extended.construction_rest_pilot().rest_raw_mode =
                    terminal || construction_rest_report.raw_ok;
                frame
                    .extended
                    .economy_rest_pilot()
                    .apply_economy_rest_report(world.economy.context.needed, &economy_rest_report);
                frame.extended.economy_rest_pilot().rest_raw_mode =
                    terminal || economy_rest_report.raw_ok;
                frame
                    .extended
                    .focus_pilot()
                    .apply_focus_report(world.focus.context.needed, &focus_report);
                frame.extended.focus_pilot().focus_raw_mode = terminal || focus_report.raw_ok;
                if production_rest_report.raw_ok && !terminal {
                    production_rest_payload = Some(world.production.first);
                } else {
                    decode::recycle_production_rest_job(world.production.first);
                }
                if let Some(second) = world.production.second {
                    decode::recycle_production_rest_job(second);
                }
                if construction_rest_report.raw_ok && !terminal {
                    construction_rest_payload = Some(world.construction.first);
                } else {
                    decode::recycle_construction_rest_job(world.construction.first);
                }
                if let Some(second) = world.construction.second {
                    decode::recycle_construction_rest_job(second);
                }
                if economy_rest_report.raw_ok && !terminal {
                    economy_rest_payload = Some(world.economy.first);
                } else {
                    decode::recycle_economy_rest_job(world.economy.first);
                }
                if let Some(second) = world.economy.second {
                    decode::recycle_economy_rest_job(second);
                }
                if focus_report.raw_ok && !terminal {
                    focus_payload = Some(world.focus.first);
                } else {
                    decode::recycle_focus_job(world.focus.first);
                }
                if let Some(second) = world.focus.second {
                    decode::recycle_focus_job(second);
                }
                if terminal {
                    production_rest_report.raw_ok = false;
                    construction_rest_report.raw_ok = false;
                    economy_rest_report.raw_ok = false;
                    focus_report.raw_ok = false;
                }
                // Recover these raw families before their typed fallbacks. ETA
                // owns separate buffers and continues through the independent
                // extended captures below; its join remains before verification.
                let join_begin = qpc();
                let relations_completed = relations_dispatched.then(|| {
                    acquisition
                        .finish_relations()
                        .expect("submitted relations job")
                });
                let characters_completed = characters_dispatched.then(|| {
                    acquisition
                        .finish_characters()
                        .expect("submitted characters job")
                });
                shared
                    .acquisition_wait_ns
                    .fetch_add(shared.ns(qpc() - join_begin), Ordering::Relaxed);
                let relations_returned = if let Some(completed) = relations_completed {
                    Some((completed.request, completed.report.unwrap_or_default()))
                } else {
                    relations_request.take().map(|mut request| {
                        let report = extended::intel::copy_relations_refresh(
                            &memory,
                            request.context,
                            &mut request.first,
                            request.second.as_mut(),
                        );
                        (request, report)
                    })
                };
                if let Some((request, report)) = relations_returned {
                    relations_report = report;
                    let relations = frame.extended.relations_pilot();
                    relations.apply_relations_report(
                        [
                            extended_needed[decode::RELATIONS[0].index()],
                            extended_needed[decode::RELATIONS[1].index()],
                        ],
                        &relations_report,
                    );
                    relations.relations_raw_mode = relations_report.raw_ok;
                    if relations_report.raw_ok {
                        relations_payload = Some(request.first);
                    } else {
                        decode::recycle_relations_job(request.first);
                    }
                    if let Some(second) = request.second {
                        decode::recycle_relations_job(second);
                    }
                }
                let characters_returned = if let Some(completed) = characters_completed {
                    Some((completed.request, completed.report.unwrap_or_default()))
                } else {
                    characters_request.take().map(|mut request| {
                        let report = extended::characters::copy_characters_refresh(
                            &memory,
                            request.context,
                            &mut request.first,
                            request.second.as_mut(),
                        );
                        (request, report)
                    })
                };
                if let Some((request, report)) = characters_returned {
                    characters_report = report;
                    let characters = frame.extended.characters_pilot();
                    characters.apply_characters_report(&extended_needed, &characters_report);
                    characters.raw_mode = characters_report.raw_ok;
                    if characters_report.raw_ok {
                        characters_payload = Some(request.first);
                    } else {
                        decode::recycle_characters_job(request.first);
                    }
                    if let Some(second) = request.second {
                        decode::recycle_characters_job(second);
                    }
                }
                let (mut checks, mut ticks) = frame.extended.capture_before_eta(
                    &cached,
                    shared.base,
                    state,
                    clock.hour,
                    &extended_needed,
                    force_full,
                );
                let visibility = replay::rows::TableId::ProvinceVisibility.index();
                let visibility_source = extended::province_visibility::source_outcome(
                    frame.provinces_result,
                    frame.consistency_checked[PROVINCE_CHECK],
                    frame.provinces_mismatches,
                );
                (checks[visibility], ticks[visibility]) =
                    frame.extended.capture_province_visibility(
                        &memory,
                        shared.base,
                        state,
                        &frame.provinces,
                        visibility_source,
                        extended_needed[visibility],
                    );
                if let Some(reason) = world_failure {
                    frame
                        .extended
                        .production_rest_pilot()
                        .fail_production_rest_capture(
                            decode::PRODUCTION_REST.map(|table| extended_needed[table.index()]),
                            reason,
                        );
                    frame
                        .extended
                        .construction_rest_pilot()
                        .fail_construction_rest_capture(
                            decode::CONSTRUCTION_REST.map(|table| extended_needed[table.index()]),
                            reason,
                        );
                    frame
                        .extended
                        .economy_rest_pilot()
                        .fail_economy_rest_capture(
                            decode::ECONOMY_REST.map(|table| extended_needed[table.index()]),
                            reason,
                        );
                    frame.extended.focus_pilot().fail_focus_capture(
                        decode::FOCUS.map(|table| extended_needed[table.index()]),
                        reason,
                    );
                    for (table, cost) in decode::PRODUCTION_REST
                        .into_iter()
                        .zip(report.production.copy_ticks)
                        .chain(
                            decode::CONSTRUCTION_REST
                                .into_iter()
                                .zip(report.construction.copy_ticks),
                        )
                        .chain(
                            decode::ECONOMY_REST
                                .into_iter()
                                .zip(report.economy.copy_ticks),
                        )
                        .chain(decode::FOCUS.into_iter().zip(report.focus.copy_ticks))
                    {
                        checks[table.index()] = false;
                        ticks[table.index()] = cost;
                    }
                }
                // No independent live acquisition needs ETA's owned output.
                // Finish it only after that work, never after final country or
                // clock verification and never after the frame is published.
                let eta_join_begin = qpc();
                let eta_completed =
                    eta_dispatched.then(|| acquisition.finish_eta().expect("submitted ETA job"));
                shared
                    .acquisition_wait_ns
                    .fetch_add(shared.ns(qpc() - eta_join_begin), Ordering::Relaxed);
                if let Some(completed) = eta_completed {
                    frame
                        .extended
                        .restore_eta_request(completed.request, Some(completed.report));
                }
                let eta = replay::rows::TableId::DivisionEta.index();
                (checks[eta], ticks[eta]) =
                    frame
                        .extended
                        .capture_eta(&cached, shared.base, state, extended_needed[eta]);
                extended_matched = checks;
                for table in extended::TABLES {
                    let index = table.index();
                    let walk = replay::cadence::should_walk(table, clock.hour, force_full);
                    frame.extended_checked[index] = extended_needed[index] && walk;
                    // Skipped tables keep zero game-thread cost for measurement.
                    frame.table_ns[index] = if walk {
                        shared.ns(ticks[index] as i64)
                    } else {
                        0
                    };
                }
                // Relations tables measured copy-only above; same install.
                if relations_report.raw_ok {
                    for (slot, table) in decode::RELATIONS.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = relations_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !relations_report.matched[slot];
                        let ns = shared.ns(relations_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_relations_copy_ns(slot, ns);
                        decode::add_relations_raw_bytes(slot, relations_report.raw_bytes[slot]);
                    }
                }
                // Characters tables measured copy-only above; same install.
                if characters_report.raw_ok {
                    for (slot, table) in decode::CHARACTERS.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = characters_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !characters_report.matched[slot];
                        let ns = shared.ns(characters_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_characters_copy_ns(slot, ns);
                        decode::add_characters_raw_bytes(slot, characters_report.raw_bytes[slot]);
                    }
                }
                // Projects tables measured copy-only above; same install.
                if projects_report.raw_ok {
                    for (slot, table) in decode::PROJECTS.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = projects_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !projects_report.matched[slot];
                        let ns = shared.ns(projects_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_projects_copy_ns(slot, ns);
                        decode::add_projects_raw_bytes(slot, projects_report.raw_bytes[slot]);
                    }
                }
                // Lexical table measured copy-only above; same install.
                if lexical_report.raw_ok {
                    for (slot, table) in decode::LEXICAL.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = lexical_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !lexical_report.matched[slot];
                        let ns = shared.ns(lexical_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_lexical_copy_ns(slot, ns);
                        decode::add_lexical_raw_bytes(slot, lexical_report.raw_bytes[slot]);
                    }
                }
                // Divisions-group production tables measured copy-only above;
                // same install for the calibration counters. The upgrades
                // phase cost is anchor-charged to the groups slot.
                if divisions_report.raw_ok {
                    for (slot, table) in decode::DIVISIONS.iter().enumerate().skip(1) {
                        let index = table.index();
                        extended_matched[index] = divisions_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !divisions_report.matched[slot];
                        let ns = shared.ns(divisions_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_divisions_copy_ns(slot, ns);
                        decode::add_divisions_raw_bytes(slot, divisions_report.raw_bytes[slot]);
                    }
                }
                // Focus pair measured copy-only above; same install for the
                // calibration counters. The single copy pass is
                // anchor-charged to the focus slot, matching the read-group
                // convention.
                if focus_report.raw_ok {
                    for (slot, table) in decode::FOCUS.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = focus_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && focus_report.mismatches[slot] != 0;
                        let ns = shared.ns(focus_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_focus_copy_ns(slot, ns);
                        decode::add_focus_raw_bytes(slot, focus_report.raw_bytes[slot]);
                    }
                }
                if air_wing_report.raw_ok {
                    for (slot, table) in decode::AIR_WING.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = air_wing_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !air_wing_report.matched[slot];
                        let ns = shared.ns(air_wing_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_air_wing_copy_ns(slot, ns);
                        decode::add_air_wing_raw_bytes(slot, air_wing_report.raw_bytes[slot]);
                    }
                }
                // Hourly air statistics measured copy-only above; same
                // install for the calibration counters. The shared
                // strategic-air census is anchor-charged to the statistics
                // slot, matching the read-group convention.
                if air_stats_report.raw_ok {
                    for (slot, table) in decode::AIR_STATS.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = air_stats_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !air_stats_report.matched[slot];
                        let ns = shared.ns(air_stats_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_air_stats_copy_ns(slot, ns);
                        decode::add_air_stats_raw_bytes(slot, air_stats_report.raw_bytes[slot]);
                    }
                }
                if naval_report.raw_ok {
                    for (slot, table) in decode::NAVAL_ORDER.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = naval_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !naval_report.matched[slot];
                        let ns = shared.ns(naval_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_naval_copy_ns(slot, ns);
                        decode::add_naval_raw_bytes(slot, naval_report.raw_bytes[slot]);
                    }
                }
                // Players pair measured copy-only above; same install for the
                // calibration counters. Players tables use core consistency
                // slots (players=6, controllers=7).
                if players_report.raw_ok {
                    for (slot, table) in decode::PLAYERS.iter().enumerate() {
                        let core = 6 + slot;
                        matched[core] = players_report.matched[slot];
                        if needed[core] {
                            frame.consistency_checked[core] = true;
                        }
                        let ns = shared.ns(players_report.copy_ticks[slot] as i64);
                        frame.table_ns[table.index()] = ns;
                        decode::add_players_copy_ns(slot, ns);
                        decode::add_players_raw_bytes(slot, players_report.raw_bytes[slot]);
                    }
                }
                // Session family measured copy-only above; same install for
                // the calibration counters. The shared manager header is
                // anchor-charged to the world slot, matching the read-group
                // convention.
                if session_report.raw_ok {
                    for (slot, table) in decode::SESSION.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = session_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !session_report.matched[slot];
                        let ns = shared.ns(session_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_session_copy_ns(slot, ns);
                        decode::add_session_raw_bytes(slot, session_report.raw_bytes[slot]);
                    }
                }
                // Production hourly rest measured copy-only above; same
                // install for the calibration counters. The deployment shared
                // traversal is anchor-charged to the conveyors slot and the
                // licence walk to the licences slot, matching the read-group
                // convention.
                if production_rest_report.raw_ok {
                    for (slot, table) in decode::PRODUCTION_REST.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = production_rest_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !production_rest_report.matched[slot];
                        let ns = shared.ns(production_rest_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_production_rest_copy_ns(slot, ns);
                        decode::add_production_rest_raw_bytes(
                            slot,
                            production_rest_report.raw_bytes[slot],
                        );
                    }
                }
                // Construction hourly rest measured copy-only above; same
                // install for the calibration counters. The construction
                // queue is anchor-charged to the building slot and the
                // occupation shared walk to the settings slot, matching the
                // read-group convention.
                if construction_rest_report.raw_ok {
                    for (slot, table) in decode::CONSTRUCTION_REST.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = construction_rest_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !construction_rest_report.matched[slot];
                        let ns = shared.ns(construction_rest_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_construction_rest_copy_ns(slot, ns);
                        decode::add_construction_rest_raw_bytes(
                            slot,
                            construction_rest_report.raw_bytes[slot],
                        );
                    }
                }
                // Economy hourly rest measured copy-only above; same install
                // for the calibration counters. The supply shared traversal
                // is anchor-charged to the hubs slot and the resource
                // dictionary to the definitions slot, matching the read-group
                // convention.
                if economy_rest_report.raw_ok {
                    for (slot, table) in decode::ECONOMY_REST.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = economy_rest_report.matched[slot];
                        frame.extended_raw_mismatches[index] = extended_needed[index]
                            && if (6..9).contains(&slot) {
                                economy_rest_report.resource_mismatches[slot - 6] != 0
                            } else {
                                !economy_rest_report.matched[slot]
                            };
                        let ns = shared.ns(economy_rest_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_economy_rest_copy_ns(slot, ns);
                        decode::add_economy_rest_raw_bytes(
                            slot,
                            economy_rest_report.raw_bytes[slot],
                        );
                    }
                }
                // Shared hourly bases measured copy-only above; same install
                // for the calibration counters. The shared census is
                // anchor-charged to the bases slot; registrations and aces
                // carry their detail costs, matching the read-group
                // convention.
                if air_bases_report.raw_ok {
                    for (slot, table) in decode::AIR_BASES.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = air_bases_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !air_bases_report.matched[slot];
                        let ns = shared.ns(air_bases_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_air_bases_copy_ns(slot, ns);
                        decode::add_air_bases_raw_bytes(slot, air_bases_report.raw_bytes[slot]);
                    }
                }
                // Shared hourly history measured copy-only above; same
                // install for the calibration counters. The single regional
                // walk is anchor-charged to the history slot, matching the
                // read-group convention.
                if air_history_report.raw_ok {
                    for (slot, table) in decode::AIR_HISTORY.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = air_history_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !air_history_report.matched[slot];
                        let ns = shared.ns(air_history_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_air_history_copy_ns(slot, ns);
                        decode::add_air_history_raw_bytes(slot, air_history_report.raw_bytes[slot]);
                    }
                }
                // Hourly supremacy measured copy-only above; same install
                // for the calibration counters.
                if supremacy_report.raw_ok {
                    for (slot, table) in decode::SUPREMACY.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = supremacy_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !supremacy_report.matched[slot];
                        let ns = shared.ns(supremacy_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_supremacy_copy_ns(slot, ns);
                        decode::add_supremacy_raw_bytes(slot, supremacy_report.raw_bytes[slot]);
                    }
                }
                // Hourly sunk-ships measured copy-only above; same install
                // for the calibration counters.
                if sunk_report.raw_ok {
                    for (slot, table) in decode::SUNK.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = sunk_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !sunk_report.matched[slot];
                        let ns = shared.ns(sunk_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_sunk_copy_ns(slot, ns);
                        decode::add_sunk_raw_bytes(slot, sunk_report.raw_bytes[slot]);
                    }
                }
                // Hourly radar measured copy-only above; same install for
                // the calibration counters.
                if radar_report.raw_ok {
                    for (slot, table) in decode::RADAR.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = radar_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !radar_report.matched[slot];
                        let ns = shared.ns(radar_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_radar_copy_ns(slot, ns);
                        decode::add_radar_raw_bytes(slot, radar_report.raw_bytes[slot]);
                    }
                }
                // Hourly intel measured copy-only above; same install for
                // the calibration counters.
                if intel_report.raw_ok {
                    for (slot, table) in decode::INTEL.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = intel_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !intel_report.matched[slot];
                        let ns = shared.ns(intel_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_intel_copy_ns(slot, ns);
                        decode::add_intel_raw_bytes(slot, intel_report.raw_bytes[slot]);
                    }
                }
                // Hourly weather measured copy-only above; same install for
                // the calibration counters. The manager header is
                // anchor-charged to the limits slot, matching the read-group
                // convention.
                if weather_report.raw_ok {
                    for (slot, table) in decode::WEATHER.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = weather_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !weather_report.matched[slot];
                        let ns = shared.ns(weather_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_weather_copy_ns(slot, ns);
                        decode::add_weather_raw_bytes(slot, weather_report.raw_bytes[slot]);
                    }
                }
                // Hourly capitulation measured copy-only above; same install
                // for the calibration counters.
                if capitulation_report.raw_ok {
                    for (slot, table) in decode::CAPITULATION.iter().enumerate() {
                        let index = table.index();
                        extended_matched[index] = capitulation_report.matched[slot];
                        frame.extended_raw_mismatches[index] =
                            extended_needed[index] && !capitulation_report.matched[slot];
                        let ns = shared.ns(capitulation_report.copy_ticks[slot] as i64);
                        frame.table_ns[index] = ns;
                        decode::add_capitulation_copy_ns(slot, ns);
                        decode::add_capitulation_raw_bytes(
                            slot,
                            capitulation_report.raw_bytes[slot],
                        );
                    }
                }
                // Trailing diagnostic: no table acquisition follows this read.
                // It does not alter any table verdict or force another refresh.
                if force_full {
                    let begin = qpc();
                    frame.order_counters.capture(
                        true,
                        &memory,
                        engine::Memory::authenticated_countries(&cached, shared.base, state),
                    );
                    frame.order_counters.qpc_begin = begin;
                    frame.order_counters.qpc_end = qpc();
                    frame.order_counters.elapsed_ns =
                        shared.ns(frame.order_counters.qpc_end - begin);
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
                    wars_matrix_check = false;
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
            shared.callback_tail[index].store(0, Ordering::Relaxed);
            // Finish every frame write before publishing its index. The decode
            // worker may acquire exclusive access as soon as enqueue succeeds;
            // its later ready-queue Release cannot protect writes after enqueue.
            // Stage the window updates locally, committing them only if sent.
            let stable = stable_clock(frame.clock_before, frame.clock_after);
            let mut consistency = unsafe { *shared.consistency.get() };
            let mut extended_consistency = unsafe { *shared.extended_consistency.get() };
            frame.consistency_successful = consistency.commit(matched, stable, true);
            frame.extended_successful = extended_consistency.commit(extended_matched, stable, true);
            // Every frame transits the single decode FIFO in hour order; the
            // worker hands it to the spool ready queue after decoding any
            // refresh payload. With 256 buffers and 256 queue slots, an owned
            // buffer guarantees space, exactly as the ready push did before.
            let game_elapsed = (qpc() - entered - (original_end - original_begin)).max(1);
            frame.capture_seq = decode::next_seq();
            let decode_job = decode::DecodeJob {
                index,
                seq: frame.capture_seq,
                hour: decode_hour,
                relations_refresh: relations_report.raw_ok,
                characters_refresh: characters_report.raw_ok,
                projects_refresh: projects_report.raw_ok,
                lexical_refresh: lexical_report.raw_ok,
                divisions_refresh: divisions_report.raw_ok,
                focus_refresh: focus_report.raw_ok,
                air_wing_refresh: air_wing_report.raw_ok,
                air_stats_refresh: air_stats_report.raw_ok,
                naval_refresh: naval_report.raw_ok,
                players_refresh: players_report.raw_ok,
                session_refresh: session_report.raw_ok,
                production_rest_refresh: production_rest_report.raw_ok,
                construction_rest_refresh: construction_rest_report.raw_ok,
                economy_rest_refresh: economy_rest_report.raw_ok,
                air_bases_refresh: air_bases_report.raw_ok,
                air_history_refresh: air_history_report.raw_ok,
                supremacy_refresh: supremacy_report.raw_ok,
                sunk_refresh: sunk_report.raw_ok,
                radar_refresh: radar_report.raw_ok,
                intel_refresh: intel_report.raw_ok,
                weather_refresh: weather_report.raw_ok,
                capitulation_refresh: capitulation_report.raw_ok,
                force_full: decode_force_full,
                game_elapsed,
                relations_payload,
                characters_payload,
                projects_payload,
                lexical_payload,
                divisions_payload,
                focus_payload,
                air_wing_payload,
                air_stats_payload,
                naval_payload,
                players_payload,
                session_payload,
                production_rest_payload,
                construction_rest_payload,
                economy_rest_payload,
                air_bases_payload,
                air_history_payload,
                supremacy_payload,
                sunk_payload,
                radar_payload,
                intel_payload,
                weather_payload,
                capitulation_payload,
            };
            match decode::enqueue(shared, decode_job) {
                None => {
                    unsafe {
                        *shared.consistency.get() = consistency;
                        *shared.extended_consistency.get() = extended_consistency;
                    }
                    if wars_matrix_check && stable {
                        complete_matrix_check(&shared.war_matrix_remaining);
                    }
                    // The frame belongs to the worker now. Publish timing
                    // through a separate atomic, never by writing the frame.
                    // This includes enqueue and post-publication bookkeeping.
                    let tail = qpc() - entered - (original_end - original_begin) - game_elapsed;
                    shared.callback_tail[index].store(tail.max(1), Ordering::Release);
                }
                Some(returned) => {
                    if let Some(payload) = returned.relations_payload {
                        decode::recycle_relations_job(payload);
                    }
                    if let Some(payload) = returned.characters_payload {
                        decode::recycle_characters_job(payload);
                    }
                    if let Some(payload) = returned.projects_payload {
                        decode::recycle_projects_job(payload);
                    }
                    if let Some(payload) = returned.lexical_payload {
                        decode::recycle_lexical_job(payload);
                    }

                    if let Some(payload) = returned.divisions_payload {
                        decode::recycle_divisions_job(payload);
                    }
                    if let Some(payload) = returned.focus_payload {
                        decode::recycle_focus_job(payload);
                    }

                    if let Some(payload) = returned.air_wing_payload {
                        decode::recycle_air_wing_job(payload);
                    }
                    if let Some(payload) = returned.air_stats_payload {
                        decode::recycle_air_stats_job(payload);
                    }
                    if let Some(payload) = returned.naval_payload {
                        decode::recycle_naval_job(payload);
                    }
                    if let Some(payload) = returned.players_payload {
                        decode::recycle_players_job(payload);
                    }
                    if let Some(payload) = returned.session_payload {
                        decode::recycle_session_job(payload);
                    }
                    if let Some(payload) = returned.production_rest_payload {
                        decode::recycle_production_rest_job(payload);
                    }
                    if let Some(payload) = returned.construction_rest_payload {
                        decode::recycle_construction_rest_job(payload);
                    }
                    if let Some(payload) = returned.economy_rest_payload {
                        decode::recycle_economy_rest_job(payload);
                    }
                    if let Some(payload) = returned.air_bases_payload {
                        decode::recycle_air_bases_job(payload);
                    }
                    if let Some(payload) = returned.air_history_payload {
                        decode::recycle_air_history_job(payload);
                    }

                    if let Some(payload) = returned.supremacy_payload {
                        decode::recycle_supremacy_job(payload);
                    }
                    if let Some(payload) = returned.sunk_payload {
                        decode::recycle_sunk_job(payload);
                    }
                    if let Some(payload) = returned.radar_payload {
                        decode::recycle_radar_job(payload);
                    }
                    if let Some(payload) = returned.intel_payload {
                        decode::recycle_intel_job(payload);
                    }
                    if let Some(payload) = returned.weather_payload {
                        decode::recycle_weather_job(payload);
                    }
                    if let Some(payload) = returned.capitulation_payload {
                        decode::recycle_capitulation_job(payload);
                    }
                    // Existing whole-frame drop plus gap path, unchanged
                    // counters and no new loss reasons.
                    drop_owned_frame(shared, frame, index, entry);
                }
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

fn acquire_world_request(
    base: usize,
    game_state: usize,
    needed: &[bool],
    focus: extended::governance::FocusCopyContext,
    economy: &extended::economy::Captured,
) -> acquisition::WorldRequest {
    // Reuse the exact existing per-family buffer eligibility: a needed
    // second pass without a provisioned buffer forces typed acquisition.
    macro_rules! pair {
        ($context:expr, $acquire:path, $recycle:path) => {{
            let context = $context;
            let mut first = $acquire();
            let second = context.needed.iter().any(|&check| check).then($acquire);
            if second
                .as_ref()
                .is_some_and(|job| job.log.spans.capacity() == 0)
            {
                $recycle(first);
                first = Default::default();
            }
            acquisition::WorldPair {
                context,
                first,
                second,
            }
        }};
    }
    acquisition::WorldRequest {
        production: pair!(
            extended::production::ProductionRestCopyContext {
                base,
                game_state,
                needed: decode::PRODUCTION_REST.map(|table| needed[table.index()]),
            },
            decode::acquire_production_rest_job,
            decode::recycle_production_rest_job
        ),
        construction: pair!(
            extended::construction::ConstructionRestCopyContext {
                base,
                game_state,
                needed: decode::CONSTRUCTION_REST.map(|table| needed[table.index()]),
            },
            decode::acquire_construction_rest_job,
            decode::recycle_construction_rest_job
        ),
        economy: pair!(
            economy.resource_copy_context(
                base,
                game_state,
                decode::ECONOMY_REST.map(|table| needed[table.index()])
            ),
            decode::acquire_economy_rest_job,
            decode::recycle_economy_rest_job
        ),
        focus: {
            let mut first = decode::acquire_focus_job();
            let second = focus
                .needed
                .iter()
                .any(|&check| check)
                .then(decode::acquire_focus_job);
            if second.as_ref().is_some_and(|job| !job.is_provisioned()) {
                decode::recycle_focus_job(first);
                first = Default::default();
            }
            acquisition::WorldPair {
                context: focus,
                first,
                second,
            }
        },
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Tier-H divisions-group raw-copy split (Divisions plus DivisionEquipment
// and the four army-upgrade tables, sharing one division census).
//
// The game thread copies raw bytes only: one shared country/province
// census, per-country roster arrays, division rows with location/template/
// orders authentication, per-division equipment pools with variant
// identities, and the army-owned upgrade graph (groups, requests,
// deliveries, produced). No row is decoded on the game thread. The single
// FIFO decode worker replays the existing walks in dependency order —
// divisions first (collecting the shared sources through the same visitor),
// then equipment and upgrades against the decoded sources — so rows,
// ordinals, skips, truncation and failure reasons are identical by
// construction (see `crate::raw` for the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting the whole copy except on overflow or on a census/row
// failure the typed walk also fails entirely on: validation, precedence
// and error scope are local parses, so the replay derives every outcome
// from the same recorded spans and later extras are harmless. Per-table
// sink failures (requests, deliveries, produced) only stop that table's
// takes; the walk continues and so does the mirror.
// ===========================================================================

/// Fault-attribution bits: equipment walk and the four upgrade tables.
/// The shared census and division rows block every slot: a fault there
/// fails the divisions walk, which makes every dependent census incomplete.
const EQUIPMENT_MASK: u32 = 0x02;
const REQUESTS_MASK: u32 = 0x08;
const DELIVERIES_MASK: u32 = 0x10;
const PRODUCED_MASK: u32 = 0x20;
const DIVISIONS_CENSUS_MASK: u32 = 0x3F;
const UPGRADES_GRAPH_MASK: u32 = 0x3C;

#[derive(Debug, Default, Clone, Copy)]
struct DivisionEntry {
    pointer: usize,
    canonical: u32,
}

/// One hour of raw divisions payload: the shared census, the divisions
/// walk, and the equipment/upgrade detail walks. Flat pools recycle
/// capacities and the steady state allocates on neither thread; growth
/// happens on the decode side only (see `crate::decode::recycle_divisions_job`).
#[derive(Debug, Default)]
pub(crate) struct DivisionsJob {
    pub log: RawLog<6>,
    pub game_state: usize,
    census: Vec<DivisionEntry>,
    sources: Vec<cequipment_variant_pool::DivisionSource>,
    census_complete: bool,
    capacity_charge: crate::raw_memory::Charge,
}

impl DivisionsJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Steady state is about 2.9k divisions with ~17k equipment rows
            // plus a few thousand upgrade rows; both pools start above that
            // so steady state never overflows.
            log: RawLog::new(1 << 18, 8 << 20),
            game_state: 0,
            census: Vec::with_capacity(4096),
            sources: Vec::with_capacity(4096),
            census_complete: false,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
        self.sources.clear();
        self.census_complete = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        crate::raw::double_capacity(&mut self.sources);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census) + vector_bytes(&self.sources),
            ..Default::default()
        });
    }

    /// Supplies only copied/authenticated source rows while the callback still
    /// owns live memory. No native reads or allocation. A row failure beyond
    /// the consumer's capacity does not invalidate its earlier truncated prefix;
    /// a failed census or a failure inside that prefix remains unavailable.
    fn source_outcome(&self, capacity: usize) -> Outcome {
        if self.log.overflowed || !self.census_complete {
            return Outcome::Unavailable {
                reason: "division copied source census incomplete",
            };
        }
        let len = self.census.len().min(capacity);
        if self.sources.len() < len {
            return Outcome::Unavailable {
                reason: "division copied source row invalid",
            };
        }
        if len < self.census.len() {
            match u32::try_from(self.census.len() - len) {
                Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
                Err(_) => Outcome::Unavailable {
                    reason: "division copied source count overflow",
                },
            }
        } else {
            Outcome::Rows { len }
        }
    }

    fn supply_sources(&self, captured: &mut extended::Captured, capacity: usize, second: bool) {
        let outcome = self.source_outcome(capacity);
        for &source in &self.sources[..outcome.len()] {
            captured.visit_division_source(source, second);
        }
        captured.finish_divisions(outcome, second);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DivisionsCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Per-partition raw equality gated by live-fault conservatism, in
    /// [divisions, equipment, groups, requests, deliveries, produced] order.
    /// Shared census spans gate every dependent partition; each table's
    /// private spans gate only their own slot, so tearing equipment private
    /// bytes flips only equipment.
    pub matched: [bool; 6],
    pub copy_ticks: [u64; 6],
    pub raw_bytes: [u64; 6],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DivisionsCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn divisions_pool_u64(log: &RawLog<6>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn dspan(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Authenticated division census shared by the divisions walk and, through
/// the decoded sources, the equipment/upgrade walks. Records the exact read
/// set of the `carmy` reader: country/province/link headers, per-slot
/// pointers, vtables, identity words, link resolutions and roster arrays
/// with entry pointers. Returns the parsed arrays for the walk mirrors, or
/// `None` exactly when the typed census fails (both walks then fail
/// identically from the same recorded spans).
fn copy_divisions_census(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<DivisionEntry>,
) -> Option<(Array, Array, u32)> {
    let countries_start = dspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        DIVISIONS_CENSUS_MASK,
    )?;
    let links_start = dspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        DIVISIONS_CENSUS_MASK,
    )?;
    let provinces_start = dspan(
        log,
        memory,
        game_state.checked_add(cgame_state::PROVINCES),
        16,
        DIVISIONS_CENSUS_MASK,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    raw.copy_from_slice(&log.bytes[provinces_start..provinces_start + 16]);
    let provinces = Array::from_header(raw, 8)?;
    if countries.count < 2 || links.count == 0 || provinces.count < 2 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    let mut found_country = false;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let pointer_start = dspan(log, memory, pointer_addr, 8, DIVISIONS_CENSUS_MASK)?;
        let pointer = usize::try_from(divisions_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0);
        let Some(pointer) = pointer else {
            continue;
        };
        let country_vtable = dspan(log, memory, Some(pointer), 8, DIVISIONS_CENSUS_MASK)?;
        let id_start = dspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            DIVISIONS_CENSUS_MASK,
        )?;
        if divisions_pool_u64(log, country_vtable) != vtable {
            return None;
        }
        let id = u32::from_le_bytes(log.bytes[id_start..id_start + 4].try_into().unwrap());
        let canonical = divisions_link(log, memory, &links, countries.count, id)?;
        if divisions_link(log, memory, &links, countries.count, slot)? != canonical {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = dspan(log, memory, canonical_addr, 8, DIVISIONS_CENSUS_MASK)?;
        if divisions_pool_u64(log, canonical_pointer) != pointer as u64 {
            return None;
        }
        if slot != canonical {
            continue;
        }
        found_country = true;
        let roster_start = dspan(
            log,
            memory,
            pointer.checked_add(ccountry::DIVISIONS),
            16,
            DIVISIONS_CENSUS_MASK,
        )?;
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[roster_start..roster_start + 16]);
        let roster = Array::from_header(header, 8)?;
        for index in 0..roster.count {
            let entry_addr = roster.data.checked_add((index as usize).saturating_mul(8));
            // An unreadable entry fails the walk; a null slot is skipped.
            let entry_start = dspan(log, memory, entry_addr, 8, DIVISIONS_CENSUS_MASK)?;
            let entry = usize::try_from(divisions_pool_u64(log, entry_start))
                .ok()
                .filter(|&pointer| pointer != 0);
            let Some(division) = entry else {
                continue;
            };
            if census.len() == census.capacity() {
                log.overflowed = true;
                return None;
            }
            census.push(DivisionEntry {
                pointer: division,
                canonical,
            });
            if log.overflowed {
                return None;
            }
        }
    }
    found_country.then_some((links, provinces, countries.count))
}

/// Mirror of one link-table read with local bounds checks. Returns `None`
/// exactly when the typed resolve fails; the replay derives its own reason.
fn divisions_link(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = dspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        DIVISIONS_CENSUS_MASK,
    )?;
    let slot = u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Parsed census arrays shared by the divisions walk mirrors.
#[derive(Clone, Copy)]
struct DivisionArrays<'a> {
    links: &'a Array,
    countries: u32,
    provinces: &'a Array,
}

/// Mirror of one division row's read set: both vtables, native handle,
/// roster-owner resolution, logical/expeditionary owners, location with
/// province authentication, template with handle authentication, hit
/// points, organisation and the orders-group identity. Returns `None`
/// exactly when the typed row fails (the whole walk then fails identically
/// from the same recorded spans); null location/template/orders pointers
/// skip their authentication like the walk.
fn copy_division_row(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    base: usize,
    arrays: DivisionArrays<'_>,
    pointer: usize,
    canonical: u32,
) -> Option<cequipment_variant_pool::DivisionSource> {
    let (links, countries, provinces) = (arrays.links, arrays.countries, arrays.provinces);
    let vtable = base.checked_add(carmy::VTABLE_RVA)? as u64;
    let secondary = base.checked_add(carmy::SECONDARY_VTABLE_RVA)? as u64;
    let primary_start = dspan(log, memory, Some(pointer), 8, DIVISIONS_CENSUS_MASK)?;
    let secondary_start = dspan(
        log,
        memory,
        pointer.checked_add(carmy::SECONDARY_BASE),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    if divisions_pool_u64(log, primary_start) != vtable {
        return None;
    }
    if divisions_pool_u64(log, secondary_start) != secondary {
        return None;
    }
    let handle_start = dspan(
        log,
        memory,
        pointer.checked_add(cunit::HANDLE),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let division = divisions_pool_u64(log, handle_start);
    if division == 0 {
        return None;
    }
    let owner_start = dspan(
        log,
        memory,
        pointer.checked_add(cunit::ROSTER_OWNER),
        4,
        DIVISIONS_CENSUS_MASK,
    )?;
    let roster_owner =
        u32::from_le_bytes(log.bytes[owner_start..owner_start + 4].try_into().unwrap());
    if divisions_link(log, memory, links, countries, roster_owner)? != canonical {
        return None;
    }
    dspan(
        log,
        memory,
        pointer.checked_add(cunit::LOGICAL_OWNER),
        4,
        DIVISIONS_CENSUS_MASK,
    )?;
    dspan(
        log,
        memory,
        pointer.checked_add(cunit::EXPEDITIONARY_OWNER),
        4,
        DIVISIONS_CENSUS_MASK,
    )?;
    let location_start = dspan(
        log,
        memory,
        pointer.checked_add(cunit::LOCATION),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let location = usize::try_from(divisions_pool_u64(log, location_start)).ok()?;
    if location != 0 {
        copy_division_province(log, memory, base, provinces, location)?;
    }
    let template_start = dspan(
        log,
        memory,
        pointer.checked_add(carmy::STANDARD_TEMPLATE),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let template = usize::try_from(divisions_pool_u64(log, template_start)).ok()?;
    let template = if template == 0 {
        0
    } else {
        copy_division_template(log, memory, base, template)?
    };
    dspan(
        log,
        memory,
        pointer.checked_add(carmy::STRENGTH),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    dspan(
        log,
        memory,
        pointer.checked_add(carmy::ORGANISATION),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let group_start = dspan(
        log,
        memory,
        pointer.checked_add(carmy::ORDERS_GROUP),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let group = usize::try_from(divisions_pool_u64(log, group_start)).ok()?;
    if group != 0 {
        let start = dspan(log, memory, Some(group), 16, DIVISIONS_CENSUS_MASK)?;
        let vtable = divisions_pool_u64(log, start);
        if ![
            engine::corders_group::VTABLE_RVA,
            engine::carmy_group::VTABLE_RVA,
        ]
        .iter()
        .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
            || divisions_pool_u64(log, start + 8) as u32 != engine::corders_group::REFERENCE_TYPE
        {
            return None;
        }
    }
    Some(cequipment_variant_pool::DivisionSource {
        pointer,
        division,
        template,
        country: roster_owner,
    })
}

/// Mirror of one province authentication: vtable, identity word and the
/// registered slot pointer. Returns `None` exactly when the typed
/// authentication fails.
fn copy_division_province(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    base: usize,
    provinces: &Array,
    pointer: usize,
) -> Option<()> {
    let expected = base.checked_add(cprovince::VTABLE_RVA)? as u64;
    let vtable_start = dspan(log, memory, Some(pointer), 8, DIVISIONS_CENSUS_MASK)?;
    if divisions_pool_u64(log, vtable_start) != expected {
        return None;
    }
    let id_start = dspan(
        log,
        memory,
        pointer.checked_add(cprovince::ID),
        4,
        DIVISIONS_CENSUS_MASK,
    )?;
    let id = u32::from_le_bytes(log.bytes[id_start..id_start + 4].try_into().unwrap());
    if id == 0 || id >= provinces.count {
        return None;
    }
    let slot_addr = provinces.data.checked_add((id as usize).checked_mul(8)?);
    let slot_start = dspan(log, memory, slot_addr, 8, DIVISIONS_CENSUS_MASK)?;
    if divisions_pool_u64(log, slot_start) != pointer as u64 {
        return None;
    }
    Some(())
}

/// Mirror of one template authentication: vtable and the nonzero native
/// handle. Returns `None` exactly when the typed authentication fails.
fn copy_division_template(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> Option<u64> {
    let expected = base.checked_add(creferenced_division_template::VTABLE_RVA)? as u64;
    let vtable_start = dspan(log, memory, Some(pointer), 8, DIVISIONS_CENSUS_MASK)?;
    if divisions_pool_u64(log, vtable_start) != expected {
        return None;
    }
    let handle_start = dspan(
        log,
        memory,
        pointer.checked_add(creferenced_division_template::TYPE),
        8,
        DIVISIONS_CENSUS_MASK,
    )?;
    let handle = divisions_pool_u64(log, handle_start);
    if handle == 0 {
        return None;
    }
    Some(handle)
}

/// Mirror of one division's equipment bank: pool vtable, item array and per
/// entry the record plus the variant identity. Stops the equipment phase
/// (returns `false`) exactly when the typed walk fails the whole table;
/// later divisions stay unrecorded and the replay fails at the same read.
fn copy_division_equipment(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let pool = match pointer.checked_add(carmy::EQUIPMENT_POOL) {
        Some(pool) => pool,
        None => return false,
    };
    let expected = match base.checked_add(cequipment_variant_pool::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let vtable_start = match dspan(log, memory, Some(pool), 8, EQUIPMENT_MASK) {
        Some(start) => start,
        None => return false,
    };
    if divisions_pool_u64(log, vtable_start) != expected {
        return false;
    }
    let header_start = match dspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ITEMS),
        16,
        EQUIPMENT_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut header = [0u8; 16];
    header.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(header, equipment_entry::STRIDE) {
        Some(array) => array,
        None => return false,
    };
    for index in 0..array.count {
        let entry_addr = array
            .data
            .checked_add((index as usize).saturating_mul(equipment_entry::STRIDE));
        let entry_start = match dspan(
            log,
            memory,
            entry_addr,
            equipment_entry::STRIDE,
            EQUIPMENT_MASK,
        ) {
            Some(start) => start,
            None => return false,
        };
        let variant = match usize::try_from(u64::from_le_bytes(
            log.bytes[entry_start..entry_start + 8].try_into().unwrap(),
        ))
        .ok()
        {
            Some(variant) => variant,
            None => return false,
        };
        if dspan(log, memory, Some(variant), 16, EQUIPMENT_MASK).is_none() {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

/// Mirror of one division's army-owned upgrade graph: the requests-object
/// pointer, the group identity pair, the request vector with variant
/// identities, and the delivery vector with per-delivery authentication,
/// progress rows and produced pools. Per-table sink failures only stop that
/// table's takes; the walk continues and so does the mirror. A null or
/// unreadable requests pointer fails the whole walk, so it stops the
/// upgrades phase (returns `false`); the replay fails at the same read.
fn copy_division_upgrades(
    log: &mut RawLog<6>,
    memory: &impl engine::Memory,
    pointer: usize,
) -> bool {
    let outer_start = match dspan(
        log,
        memory,
        pointer.checked_add(carmy::REQUESTS),
        8,
        UPGRADES_GRAPH_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let outer = match usize::try_from(divisions_pool_u64(log, outer_start))
        .ok()
        .filter(|&outer| outer != 0)
    {
        Some(outer) => outer,
        None => return false,
    };
    let group = match outer.checked_add(carmy_upgrades_requests::IN_ARMY_REQUESTS) {
        Some(group) => group,
        None => return false,
    };
    // Group identity pair; validation mismatches are derived by the replay,
    // so the mirror continues past them.
    dspan(log, memory, Some(group), 12, UPGRADES_GRAPH_MASK);
    dspan(
        log,
        memory,
        group.checked_add(carmy_upgrades_requests::PERSISTENT_SUBOBJECT),
        16,
        UPGRADES_GRAPH_MASK,
    );
    if log.overflowed {
        return false;
    }
    copy_upgrade_requests(log, memory, group);
    if log.overflowed {
        return false;
    }
    copy_upgrade_deliveries(log, memory, group);
    !log.overflowed
}

/// Mirror of one group's request vector: header plus per-entry records
/// with variant identities. Vector failure only fails the requests table.
fn copy_upgrade_requests(log: &mut RawLog<6>, memory: &impl engine::Memory, group: usize) {
    let header_start = match dspan(
        log,
        memory,
        group.checked_add(carmy_upgrades_requests::REQUESTS_DATA),
        16,
        REQUESTS_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 16];
    header.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(header, army_upgrade_request_row::STRIDE) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let entry_addr = array
            .data
            .checked_add((index as usize).saturating_mul(army_upgrade_request_row::STRIDE));
        let entry_start = match dspan(
            log,
            memory,
            entry_addr,
            army_upgrade_request_row::STRIDE,
            REQUESTS_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let variant = match usize::try_from(u64::from_le_bytes(
            log.bytes[entry_start..entry_start + 8].try_into().unwrap(),
        ))
        .ok()
        {
            Some(variant) => variant,
            None => return,
        };
        // A null variant fails the row like the typed identity read; the
        // take records the failure and the replay derives the same reason.
        dspan(log, memory, Some(variant), 16, REQUESTS_MASK);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one group's delivery vector: header plus per-delivery entry
/// pointers, authentication triples, progress rows and produced pools.
/// The header feeds both deliveries and produced (both fail without it);
/// entry pointers fail the whole walk. Vector failure fails deliveries and
/// produced for this group; the walk continues with the next source and so
/// does the mirror.
fn copy_upgrade_deliveries(log: &mut RawLog<6>, memory: &impl engine::Memory, group: usize) {
    let header_start = match dspan(
        log,
        memory,
        group.checked_add(carmy_upgrades_requests::DELIVERIES_DATA),
        16,
        DELIVERIES_MASK | PRODUCED_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 16];
    header.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(header, carmy_upgrades_requests::DELIVERY_ENTRY_STRIDE) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let entry_addr = array.data.checked_add(
            (index as usize).saturating_mul(carmy_upgrades_requests::DELIVERY_ENTRY_STRIDE),
        );
        let pointer_start = match dspan(log, memory, entry_addr, 8, UPGRADES_GRAPH_MASK) {
            Some(start) => start,
            None => return,
        };
        // A null entry pointer fails the whole walk; stop the phase so the
        // replay fails at the same read.
        let delivery = match usize::try_from(divisions_pool_u64(log, pointer_start))
            .ok()
            .filter(|&delivery| delivery != 0)
        {
            Some(delivery) => delivery,
            None => return,
        };
        copy_upgrade_delivery(log, memory, delivery);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one delivery: authentication triple, progress row and produced
/// pool with variant identities. A failed authentication stops this
/// delivery's takes (the walk breaks to the next source); a failed status
/// field only fails the deliveries row while the produced pool is still
/// read, exactly like the typed walk.
fn copy_upgrade_delivery(log: &mut RawLog<6>, memory: &impl engine::Memory, delivery: usize) {
    let pool = match delivery.checked_add(cupgrade_delivery::PRODUCED_EQUIPMENT_POOL) {
        Some(pool) => pool,
        None => return,
    };
    let identity_start = dspan(
        log,
        memory,
        Some(delivery),
        8,
        DELIVERIES_MASK | PRODUCED_MASK,
    );
    let pool_start = dspan(log, memory, Some(pool), 8, DELIVERIES_MASK | PRODUCED_MASK);
    let status_start = dspan(
        log,
        memory,
        delivery.checked_add(cupgrade_delivery::STATUS),
        4,
        DELIVERIES_MASK | PRODUCED_MASK,
    );
    let (Some(_), Some(_)) = (identity_start, pool_start) else {
        return;
    };
    if log.overflowed {
        return;
    }
    let status = status_start
        .map(|start| u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap()));
    if let Some(status) = status {
        // Progress is read only for the progress-available status; the
        // scatter shape matches the typed row read exactly.
        if status == cupgrade_delivery::PROGRESS_AVAILABLE_STATUS {
            dspan(
                log,
                memory,
                delivery.checked_add(cupgrade_delivery::PROGRESS),
                8,
                DELIVERIES_MASK,
            );
        }
        dspan(
            log,
            memory,
            delivery.checked_add(cupgrade_delivery::TOTAL_PROGRESS),
            8,
            DELIVERIES_MASK,
        );
        dspan(
            log,
            memory,
            pool.checked_add(cequipment_variant_pool::ALLOW_ZERO),
            1,
            DELIVERIES_MASK,
        );
    }
    if log.overflowed {
        return;
    }
    let header_start = match dspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ITEMS),
        16,
        PRODUCED_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 16];
    header.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(header, equipment_entry::STRIDE) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let entry_addr = array
            .data
            .checked_add((index as usize).saturating_mul(equipment_entry::STRIDE));
        let entry_start = match dspan(
            log,
            memory,
            entry_addr,
            equipment_entry::STRIDE,
            PRODUCED_MASK,
        ) {
            Some(start) => start,
            None => return,
        };
        let variant = match usize::try_from(u64::from_le_bytes(
            log.bytes[entry_start..entry_start + 8].try_into().unwrap(),
        ))
        .ok()
        {
            Some(variant) => variant,
            None => return,
        };
        dspan(log, memory, Some(variant), 16, PRODUCED_MASK);
        if log.overflowed {
            return;
        }
    }
}

/// One structural copy pass: shared census, divisions walk, equipment
/// phase and upgrades phase. Copy ticks split into [divisions, equipment,
/// upgrades]; the upgrades phase cost is anchor-charged to the groups slot
/// (matching the read-group convention), with the other three upgrade
/// slots carrying zero. The job is cleared on entry so pooled hours never
/// accumulate spans.
fn copy_divisions_pass(
    job: &mut DivisionsJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; 6],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    let census = copy_divisions_census(&mut job.log, memory, base, game_state, &mut job.census);
    let Some((links, provinces, countries)) = census else {
        ticks[0] += elapsed(started);
        return;
    };
    job.census_complete = true;
    for &entry in &job.census {
        let Some(source) = copy_division_row(
            &mut job.log,
            memory,
            base,
            DivisionArrays {
                links: &links,
                countries,
                provinces: &provinces,
            },
            entry.pointer,
            entry.canonical,
        ) else {
            ticks[0] += elapsed(started);
            return;
        };
        if job.sources.len() == job.sources.capacity() {
            job.log.overflowed = true;
        } else {
            job.sources.push(source);
        }
        if job.log.overflowed {
            ticks[0] += elapsed(started);
            return;
        }
    }
    ticks[0] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let equipment_started = qpc();
    for &entry in &job.census {
        if !copy_division_equipment(&mut job.log, memory, base, entry.pointer) {
            break;
        }
    }
    ticks[1] += elapsed(equipment_started);
    if job.log.overflowed {
        return;
    }
    let upgrades_started = qpc();
    for &entry in &job.census {
        if !copy_division_upgrades(&mut job.log, memory, entry.pointer) {
            break;
        }
    }
    ticks[2] += elapsed(upgrades_started);
}

/// Game-thread raw copy for one hour: shared census, divisions walk, and
/// the equipment/upgrade detail walks. Compares raw logs while any group
/// window needs checks.
pub(crate) fn copy_divisions_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    first: &mut DivisionsJob,
    mut second: Option<&mut DivisionsJob>,
) -> DivisionsCopyReport {
    let mut ticks = [0u64; 6];
    copy_divisions_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; 6];
    if let Some(second_job) = second.as_deref_mut() {
        let mut second_ticks = [0u64; 6];
        copy_divisions_pass(
            second_job,
            memory,
            ctx.base,
            ctx.game_state,
            &mut second_ticks,
        );
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        // Per-slot partitions: shared census spans gate every dependent
        // slot, private spans only their own. Each slot keeps the
        // conservative live-fault gate.
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
    DivisionsCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

/// Decode-worker interpretation of a raw divisions payload, in dependency
/// order: the divisions walk first (collecting the shared sources through
/// the same visitor the game thread uses), then equipment and upgrades
/// against the decoded sources. Installs identical rows, truncation counts
/// and failure reasons into the frame's pooled buffers.
pub(crate) fn decode_divisions_payload(
    frame: &mut Captured,
    base: usize,
    job: &DivisionsJob,
    decode_ticks: &mut [u64; 6],
) {
    let recorded = Recorded::build(&job.log);
    let started = qpc();
    // Live presentation already consumed these shared sources in the callback.
    // Replace only the source scratch for worker-owned interpretation; preserve
    // the acquired presentation tables, their costs and consistency results.
    frame.extended.reset_division_sources();
    let outcome = carmy::walk_with(
        &recorded,
        base,
        job.game_state,
        &mut frame.divisions,
        |pointer, row| {
            frame.extended.visit_division(pointer, row, false);
        },
    );
    frame.extended.finish_divisions(outcome, false);
    frame.divisions_result = outcome;
    decode_ticks[0] += elapsed(started);
    // The detail walk needs fresh observation cursors, not a second live
    // index. Return this index before its independent reader is constructed.
    drop(recorded);
    frame
        .extended
        .decode_divisions_payload(base, &job.log, decode_ticks);
}

/// Shared synthetic address space for the typed Wars capture and FIFO tests:
/// three countries plus one alias slot, two wars with signed-extreme rows,
/// one foreign-type relation, one retired war, and matrix slots with an
/// alias raw target. Mirrors the engine `cwar_relation` fixture shape.
#[cfg(test)]
pub(crate) mod wars_test_kit {
    use engine::{
        Memory, ccountry, cdiplomacy_status, cgame_state, crelation, crelation_status,
        cwar_relation, memory::SliceMemory, walk::Outcome,
    };

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    pub(crate) const CTAB: usize = 0x11000;
    pub(crate) const LTAB: usize = 0x12000;
    pub(crate) const C0: usize = 0x20000;
    const C: [usize; 3] = [0x20000, 0x21000, 0x22000];
    pub(crate) const D0: usize = 0x30000;
    const D: [usize; 3] = [0x30000, 0x31000, 0x32000];
    pub(crate) const M0: usize = 0x33000;
    const M: [usize; 3] = [0x33000, 0x34000, 0x35000];
    pub(crate) const O0: usize = 0x36000;
    const O: [usize; 3] = [0x36000, 0x37000, 0x38000];
    pub(crate) const WAR1: usize = 0x70000;
    pub(crate) const WAR2: usize = 0x71000;
    pub(crate) const OTHER: usize = 0x72000;
    pub(crate) const RETIRED: usize = 0x73000;

    pub(crate) fn status(owner: usize, target: usize) -> usize {
        0x40000 + owner * 0x10000 + target * 0xa00
    }

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    fn entries64(bytes: &mut [u8], data: usize, values: &[u64]) {
        for (index, &value) in values.iter().enumerate() {
            p64(bytes, data + index * 8, value);
        }
    }

    fn entries32(bytes: &mut [u8], data: usize, values: &[u32]) {
        for (index, &value) in values.iter().enumerate() {
            p32(bytes, data + index * 4, value);
        }
    }

    /// One relation status with empty ownership vectors. The live pair
    /// statuses below overwrite the war link and vectors afterwards.
    fn status_init(bytes: &mut [u8], owner: usize, target: usize, raw_target: u32) {
        let at = status(owner, target);
        p64(bytes, at, crelation_status::VTABLE_RVA as u64);
        p64(
            bytes,
            at + crelation_status::DIPLOMACY_STATUS,
            D[owner] as u64,
        );
        p32(bytes, at + crelation_status::TARGET_COUNTRY, raw_target);
        p64(bytes, at + crelation_status::WAR_RELATION, 0);
        header(
            &mut *bytes,
            at + crelation_status::SUBRELATIONS,
            at + 0x400,
            0,
        );
        header(
            &mut *bytes,
            at + crelation_status::SERIALIZABLE_RELATIONS,
            at + 0x420,
            0,
        );
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x80000];
        // Slots 1..3 hold countries; slot 4 aliases the first object. Raw 4
        // resolves to slot 1 everywhere, like the engine fixture.
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 5);
        entries64(
            &mut bytes,
            CTAB,
            &[0, C[0] as u64, C[1] as u64, C[2] as u64, C[0] as u64],
        );
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 5);
        entries32(&mut bytes, LTAB, &[0, 1, 2, 3, 1]);
        for owner in 0..3 {
            p64(&mut bytes, C[owner], ccountry::VTABLE_RVA as u64);
            p32(&mut bytes, C[owner] + ccountry::ID, (owner + 1) as u32);
            p64(
                &mut bytes,
                C[owner] + ccountry::DIPLOMACY_STATUS,
                D[owner] as u64,
            );
            p64(&mut bytes, D[owner], cdiplomacy_status::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                D[owner] + cdiplomacy_status::COUNTRY,
                C[owner] as u64,
            );
            let slots: Vec<u64> = (0..5).map(|target| status(owner, target) as u64).collect();
            header(
                &mut bytes,
                D[owner] + cdiplomacy_status::RELATIONS,
                M[owner],
                5,
            );
            entries64(&mut bytes, M[owner], &slots);
            for target in 0..5 {
                // Matrix slot one of the first diplomacy deliberately uses
                // alias raw 4 as its target, mirroring the engine fixture.
                let raw = if (owner, target) == (0, 1) {
                    4
                } else {
                    target as u32
                };
                status_init(&mut bytes, owner, target, raw);
            }
        }
        // Owned vectors: every relation kind lives here, but only wars pay
        // the deep dive.
        header(
            &mut bytes,
            D[0] + cdiplomacy_status::OWNED_RELATIONS,
            O[0],
            3,
        );
        entries64(&mut bytes, O[0], &[WAR1 as u64, OTHER as u64, WAR2 as u64]);
        header(
            &mut bytes,
            D[1] + cdiplomacy_status::OWNED_RELATIONS,
            O[1],
            2,
        );
        entries64(&mut bytes, O[1], &[WAR1 as u64, RETIRED as u64]);
        header(
            &mut bytes,
            D[2] + cdiplomacy_status::OWNED_RELATIONS,
            O[2],
            1,
        );
        entries64(&mut bytes, O[2], &[WAR2 as u64]);
        // Foreign-type entry: skipped right after the type gather.
        p32(&mut bytes, OTHER + crelation::TYPE_ID, 0x28da);
        // Retired war: authenticated but inactive, skipped without rows.
        p64(&mut bytes, RETIRED, cwar_relation::VTABLE_RVA as u64);
        p32(
            &mut bytes,
            RETIRED + crelation::TYPE_ID,
            cwar_relation::TYPE,
        );
        bytes[RETIRED + crelation::RETIRED] = 1;
        p32(&mut bytes, RETIRED + crelation::FIRST_COUNTRY, 1);
        p32(&mut bytes, RETIRED + crelation::SECOND_COUNTRY, 2);
        // WAR1 (countries 0, 1) carries signed-extreme rows; WAR2 (countries
        // 2, 0) small distinct values.
        for (war, second, start, end, cancel, cas, unk, instigator) in [
            (
                WAR1,
                2u32,
                i32::MIN,
                i32::MAX,
                7u8,
                [i64::MAX, i64::MIN],
                [0x1_0000_0001i64, -0x1_0000_0001i64],
                255u8,
            ),
            (WAR2, 1u32, 5, 6, 0u8, [11, 22], [33, 44], 0u8),
        ] {
            let first = if war == WAR1 { 1u32 } else { 3u32 };
            p64(&mut bytes, war, cwar_relation::VTABLE_RVA as u64);
            p32(&mut bytes, war + crelation::TYPE_ID, cwar_relation::TYPE);
            p32(&mut bytes, war + crelation::FIRST_COUNTRY, first);
            p32(&mut bytes, war + crelation::SECOND_COUNTRY, second);
            p32(&mut bytes, war + crelation::START_HOUR, start as u32);
            p32(&mut bytes, war + crelation::END_HOUR, end as u32);
            bytes[war + crelation::CANCEL] = cancel;
            bytes[war + cwar_relation::FIRST_WAS_INSTIGATOR] = instigator;
            p64(
                &mut bytes,
                war + cwar_relation::FIRST_CASUALTIES,
                cas[0] as u64,
            );
            p64(
                &mut bytes,
                war + cwar_relation::SECOND_CASUALTIES,
                cas[1] as u64,
            );
            p64(
                &mut bytes,
                war + cwar_relation::FIRST_UNKNOWN_CASUALTIES,
                unk[0] as u64,
            );
            p64(
                &mut bytes,
                war + cwar_relation::SECOND_UNKNOWN_CASUALTIES,
                unk[1] as u64,
            );
        }
        // Live pair statuses with war links and ownership vectors.
        for (owner, target, war) in [(0, 2, WAR1), (1, 1, WAR1), (2, 1, WAR2), (0, 3, WAR2)] {
            let at = status(owner, target);
            p64(&mut bytes, at + crelation_status::WAR_RELATION, war as u64);
            let sub = &[war as u64][..];
            header(
                &mut bytes,
                at + crelation_status::SUBRELATIONS,
                at + 0x400,
                1,
            );
            entries64(&mut bytes, at + 0x400, sub);
            let serializable = (owner, target) == (0, 2) || (owner, target) == (2, 1);
            header(
                &mut bytes,
                at + crelation_status::SERIALIZABLE_RELATIONS,
                at + 0x420,
                u32::from(serializable),
            );
            if serializable {
                entries64(&mut bytes, at + 0x420, sub);
            }
        }
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Censuses {
        pub owned_outcome: Outcome,
        pub owned: Vec<cwar_relation::Row>,
        pub matrix_outcome: Outcome,
        pub matrix: Vec<cwar_relation::Row>,
    }

    /// Reference typed walks with identical pooled buffers.
    pub(crate) fn direct(memory: &impl Memory, len: usize) -> Censuses {
        let mut owned = vec![cwar_relation::Row::default(); len];
        let owned_outcome = cwar_relation::walk(memory, BASE, GAME_STATE, &mut owned);
        let mut matrix = vec![cwar_relation::Row::default(); len];
        let matrix_outcome = cwar_relation::walk_matrix(memory, BASE, GAME_STATE, &mut matrix);
        Censuses {
            owned_outcome,
            owned,
            matrix_outcome,
            matrix,
        }
    }
}

/// Shared synthetic address space for the divisions-group copy/decode
/// harness: two countries plus one alias slot, one province, three
/// divisions (null location, null template and null orders covered),
/// equipment pools with signed amounts, and upgrade groups with requests,
/// one live delivery and produced rows.
#[cfg(test)]
pub(crate) mod divisions_test_kit {
    use super::{Captured, DivisionsCopyContext, DivisionsCopyReport, DivisionsJob};
    use crate::extended::production::complete_source;
    use engine::{
        Memory, army_upgrade_request_row, carmy, carmy_upgrades_requests, carmy_upgrades_status,
        ccountry, cequipment_variant, cequipment_variant_pool, cgame_state, corders_group,
        cprovince, creferenced_division_template, cunit, cupgrade_delivery, memory::SliceMemory,
        walk::Outcome,
    };

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    pub(crate) const CTAB: usize = 0x11000;
    pub(crate) const LTAB: usize = 0x12000;
    pub(crate) const PTAB: usize = 0x13000;
    pub(crate) const C0: usize = 0x20000;
    pub(crate) const C1: usize = 0x21000;
    pub(crate) const P1: usize = 0x24000;
    pub(crate) const A1: usize = 0x30000;
    pub(crate) const A2: usize = 0x31000;
    pub(crate) const A3: usize = 0x32000;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    fn variant(bytes: &mut [u8], at: usize, id: u32) {
        p64(bytes, at, cequipment_variant::VTABLE_RVA as u64);
        p64(
            bytes,
            at + 8,
            ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64,
        );
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x68000];
        // Slots 1..2 hold countries; slot 3 aliases the first object.
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
        for (index, pointer) in [0u64, C0 as u64, C1 as u64, C0 as u64]
            .into_iter()
            .enumerate()
        {
            p64(&mut bytes, CTAB + index * 8, pointer);
        }
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
        for (index, slot) in [0u32, 1, 2, 1].into_iter().enumerate() {
            p32(&mut bytes, LTAB + index * 4, slot);
        }
        header(&mut bytes, GAME_STATE + cgame_state::PROVINCES, PTAB, 2);
        p64(&mut bytes, PTAB, 0);
        p64(&mut bytes, PTAB + 8, P1 as u64);
        for (country, id, roster, entries) in [
            (C0, 1u32, 0x23000usize, vec![A1, A2]),
            (C1, 2u32, 0x23100usize, vec![A3]),
        ] {
            p64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
            p32(&mut bytes, country + ccountry::ID, id);
            header(
                &mut bytes,
                country + ccountry::DIVISIONS,
                roster,
                entries.len() as u32,
            );
            for (index, division) in entries.into_iter().enumerate() {
                p64(&mut bytes, roster + index * 8, division as u64);
            }
        }
        p64(&mut bytes, P1, cprovince::VTABLE_RVA as u64);
        p32(&mut bytes, P1 + cprovince::ID, 1);
        // A1 is complete; A2 has null location and orders; A3 has a null
        // template.
        for (army, handle, owner, location, template, orders, hp, org) in [
            (
                A1,
                101u64,
                1u32,
                P1,
                0x40000usize,
                0x41000usize,
                100_000i64,
                50_000i64,
            ),
            (
                A2,
                102u64,
                1u32,
                0,
                0x40000usize,
                0usize,
                200_000i64,
                60_000i64,
            ),
            (
                A3,
                103u64,
                2u32,
                P1,
                0usize,
                0x41000usize,
                -300_000i64,
                70_000i64,
            ),
        ] {
            p64(&mut bytes, army, carmy::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                army + carmy::SECONDARY_BASE,
                carmy::SECONDARY_VTABLE_RVA as u64,
            );
            p64(&mut bytes, army + cunit::HANDLE, handle);
            p32(&mut bytes, army + cunit::ROSTER_OWNER, owner);
            p32(&mut bytes, army + cunit::LOGICAL_OWNER, owner);
            p32(&mut bytes, army + cunit::EXPEDITIONARY_OWNER, 0);
            p64(&mut bytes, army + cunit::LOCATION, location as u64);
            p64(&mut bytes, army + carmy::STANDARD_TEMPLATE, template as u64);
            p64(&mut bytes, army + carmy::STRENGTH, hp as u64);
            p64(&mut bytes, army + carmy::ORGANISATION, org as u64);
            p64(&mut bytes, army + carmy::ORDERS_GROUP, orders as u64);
        }
        p64(
            &mut bytes,
            0x40000,
            creferenced_division_template::VTABLE_RVA as u64,
        );
        p64(
            &mut bytes,
            0x40000 + creferenced_division_template::TYPE,
            0x3455_0000_0034u64,
        );
        p64(&mut bytes, 0x41000, corders_group::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            0x41008,
            ((57u64) << 32) | corders_group::REFERENCE_TYPE as u64,
        );
        // Equipment pools: two signed entries, one entry, and empty.
        variant(&mut bytes, 0x50000, 11);
        variant(&mut bytes, 0x51000, 12);
        for (army, items, entries) in [
            (
                A1,
                0x42000usize,
                vec![(0x50000usize, 100i64), (0x51000usize, -50i64)],
            ),
            (A2, 0x42100usize, vec![(0x50000usize, 7i64)]),
            (A3, 0x42200usize, vec![]),
        ] {
            let pool = army + carmy::EQUIPMENT_POOL;
            p64(&mut bytes, pool, cequipment_variant_pool::VTABLE_RVA as u64);
            header(
                &mut bytes,
                pool + cequipment_variant_pool::ITEMS,
                items,
                entries.len() as u32,
            );
            for (index, (pointer, amount)) in entries.into_iter().enumerate() {
                p64(&mut bytes, items + index * 16, pointer as u64);
                p64(&mut bytes, items + index * 16 + 8, amount as u64);
            }
        }
        // Upgrade groups: live group with one request and one delivery,
        // empty group, and a group with a request but no deliveries.
        for (army, group, priority, requests, deliveries) in [
            (
                A1,
                0x60070usize,
                3u32,
                vec![(0x50000usize, 1000i64, 2000i64)],
                vec![0x64000usize],
            ),
            (A2, 0x61070usize, 5u32, vec![], vec![]),
            (
                A3,
                0x62070usize,
                7u32,
                vec![(0x51000usize, 50i64, 60i64)],
                vec![],
            ),
        ] {
            p64(&mut bytes, army + carmy::REQUESTS, (group - 0x70) as u64);
            p64(
                &mut bytes,
                group,
                carmy_upgrades_requests::VTABLE_RVA as u64,
            );
            p32(
                &mut bytes,
                group + carmy_upgrades_requests::PRIORITY,
                priority,
            );
            p64(
                &mut bytes,
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
                carmy_upgrades_requests::PERSISTENT_VTABLE_RVA as u64,
            );
            p64(
                &mut bytes,
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT + 8,
                army as u64,
            );
            header(
                &mut bytes,
                group + carmy_upgrades_requests::REQUESTS_DATA,
                group + 0x200,
                requests.len() as u32,
            );
            for (index, (pointer, request, total)) in requests.into_iter().enumerate() {
                let at = group + 0x200 + index * 0x18;
                p64(&mut bytes, at, pointer as u64);
                p64(&mut bytes, at + 8, request as u64);
                p64(&mut bytes, at + 16, total as u64);
            }
            header(
                &mut bytes,
                group + carmy_upgrades_requests::DELIVERIES_DATA,
                group + 0x300,
                deliveries.len() as u32,
            );
            for (index, delivery) in deliveries.into_iter().enumerate() {
                p64(&mut bytes, group + 0x300 + index * 8, delivery as u64);
            }
        }
        // The one live delivery: progress-available status with a produced row.
        p64(&mut bytes, 0x64000, cupgrade_delivery::VTABLE_RVA as u64);
        p32(&mut bytes, 0x64000 + cupgrade_delivery::STATUS, 1);
        p64(&mut bytes, 0x64000 + cupgrade_delivery::PROGRESS, 300);
        p64(&mut bytes, 0x64000 + cupgrade_delivery::TOTAL_PROGRESS, 400);
        p64(
            &mut bytes,
            0x64020,
            cequipment_variant_pool::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            0x64020 + cequipment_variant_pool::ITEMS,
            0x64100,
            1,
        );
        p64(&mut bytes, 0x64100, 0x51000);
        p64(&mut bytes, 0x64108, 9);
        p64(&mut bytes, 0x64020 + cequipment_variant_pool::ALLOW_ZERO, 1);
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub divisions_outcome: Outcome,
        pub divisions: Vec<carmy::Row>,
        pub equipment_outcome: Outcome,
        pub equipment: Vec<cequipment_variant_pool::DivisionRow>,
        pub groups_outcome: Outcome,
        pub groups: Vec<carmy_upgrades_requests::Row>,
        pub requests_outcome: Outcome,
        pub requests: Vec<army_upgrade_request_row::Row>,
        pub deliveries_outcome: Outcome,
        pub deliveries: Vec<cupgrade_delivery::Row>,
        pub produced_outcome: Outcome,
        pub produced: Vec<cupgrade_delivery::ProducedRow>,
    }

    /// Reference typed walks with identical pooled buffers, mirroring the
    /// production acquire's equipment/upgrade sections exactly.
    pub(crate) fn direct(memory: &impl Memory, lens: [usize; 6]) -> Split {
        let mut divisions = vec![carmy::Row::default(); lens[0]];
        let mut visited = Vec::new();
        let divisions_outcome =
            carmy::walk_with(memory, BASE, GAME_STATE, &mut divisions, |pointer, row| {
                visited.push(cequipment_variant_pool::DivisionSource {
                    pointer,
                    division: row.id,
                    template: row.template,
                    country: row.roster_owner,
                });
            });
        let first_sources = complete_source(&visited, divisions_outcome);
        let mut equipment = vec![cequipment_variant_pool::DivisionRow::default(); lens[1]];
        let equipment_outcome = match first_sources {
            Ok(source) => {
                cequipment_variant_pool::walk_divisions(memory, BASE, source, &mut equipment)
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        let mut groups = vec![carmy_upgrades_requests::Row::default(); lens[2]];
        let mut requests = vec![army_upgrade_request_row::Row::default(); lens[3]];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); lens[4]];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
        let result = match first_sources {
            Ok(source) => carmy_upgrades_status::walk(
                memory,
                BASE,
                source,
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced,
            ),
            Err(reason) => carmy_upgrades_status::Outcomes::unavailable(reason),
        };
        Split {
            divisions_outcome,
            divisions,
            equipment_outcome,
            equipment,
            groups_outcome: result.groups,
            groups,
            requests_outcome: result.requests,
            requests,
            deliveries_outcome: result.deliveries,
            deliveries,
            produced_outcome: result.produced,
            produced,
        }
    }

    /// Raw-copy split: two raw passes compared, then the first payload
    /// decoded into a pooled frame with the same buffer lengths.
    pub(crate) fn split(
        memory: &impl Memory,
        lens: [usize; 6],
    ) -> (Split, DivisionsCopyReport, DivisionsJob) {
        let ctx = DivisionsCopyContext {
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = super::copy_divisions_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut frame = Captured::new();
        frame.divisions = vec![carmy::Row::default(); lens[0]];
        frame.extended.test_resize_division_buffers(lens);
        let mut ticks = [0u64; 6];
        super::decode_divisions_payload(&mut frame, BASE, &first, &mut ticks);
        let detail = frame.extended.test_division_detail();
        // The worker clears the raw-mode gate it consumed.
        assert!(!detail.divisions_raw_mode);
        (
            Split {
                divisions_outcome: frame.divisions_result,
                divisions: std::mem::take(&mut frame.divisions),
                equipment_outcome: detail.division_equipment.outcome,
                equipment: std::mem::take(&mut detail.division_equipment.rows),
                groups_outcome: detail.groups.outcome,
                groups: std::mem::take(&mut detail.groups.rows),
                requests_outcome: detail.requests.outcome,
                requests: std::mem::take(&mut detail.requests.rows),
                deliveries_outcome: detail.deliveries.outcome,
                deliveries: std::mem::take(&mut detail.deliveries.rows),
                produced_outcome: detail.produced.outcome,
                produced: std::mem::take(&mut detail.produced.rows),
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(
            split.divisions_outcome, direct.divisions_outcome,
            "divisions outcome"
        );
        assert_eq!(split.divisions, direct.divisions, "divisions rows");
        assert_eq!(
            split.equipment_outcome, direct.equipment_outcome,
            "equipment outcome"
        );
        assert_eq!(split.equipment, direct.equipment, "equipment rows");
        assert_eq!(
            split.groups_outcome, direct.groups_outcome,
            "groups outcome"
        );
        assert_eq!(split.groups, direct.groups, "groups rows");
        assert_eq!(
            split.requests_outcome, direct.requests_outcome,
            "requests outcome"
        );
        assert_eq!(split.requests, direct.requests, "requests rows");
        assert_eq!(
            split.deliveries_outcome, direct.deliveries_outcome,
            "deliveries outcome"
        );
        assert_eq!(split.deliveries, direct.deliveries, "deliveries rows");
        assert_eq!(
            split.produced_outcome, direct.produced_outcome,
            "produced outcome"
        );
        assert_eq!(split.produced, direct.produced, "produced rows");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_counter_capture_gates_and_clears_recycled_metadata_without_allocation() {
        let mut bytes = vec![0_u8; 0x4000];
        let mut put = |address: usize, value: &[u8]| {
            bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
        };
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x1500_u64, 2_u32),
            (cgame_state::COUNTRY_LINKS, 0x1600, 2),
        ] {
            put(0x1000 + offset, &data.to_le_bytes());
            put(0x1000 + offset + 8, &count.to_le_bytes());
            put(0x1000 + offset + 12, &count.to_le_bytes());
        }
        put(0x1508, &0x2000_u64.to_le_bytes());
        put(0x1604, &1_u32.to_le_bytes());
        put(0x2000, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
        put(0x2008, &1_u32.to_le_bytes());
        put(
            0x2000 + ccountry::ORDER_INSTANCE_COUNTER,
            &3812_u32.to_le_bytes(),
        );
        let memory = engine::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut cache = Cache::with_capacity(32);
        let cached = cache.prepare(&memory, 0, 0x1000);
        assert_eq!(cached.seed_status(), SeedStatus::Ready);
        let mut sample = OrderCounterCapture::with_capacity(2);
        let pointer = sample.rows.as_ptr();
        let capacity = sample.rows.capacity();
        sample.capture(
            true,
            &memory,
            engine::Memory::authenticated_countries(&cached, 0, 0x1000),
        );
        assert!(sample.forced);
        assert!(sample.read.unwrap().complete());
        assert_eq!(sample.read.unwrap().len, 1);
        assert_eq!(sample.rows[0].value, Some(3812));
        sample.qpc_begin = 10;
        sample.qpc_end = 12;
        sample.elapsed_ns = 20;
        sample.capture(
            false,
            &memory,
            engine::Memory::authenticated_countries(&cached, 0, 0x1000),
        );
        assert!(!sample.forced);
        assert_eq!(sample.read, None);
        assert_eq!(
            (sample.qpc_begin, sample.qpc_end, sample.elapsed_ns),
            (0, 0, 0)
        );
        sample.capture(true, &memory, None);
        assert!(sample.forced);
        assert_eq!(sample.read, None);
        assert_eq!(sample.rows.as_ptr(), pointer);
        assert_eq!(sample.rows.capacity(), capacity);
        assert!(cached.verify().is_ok());
    }

    #[test]
    fn order_counter_capture_accounts_actual_capacity_and_grows_only_its_storage() {
        let mut sample = OrderCounterCapture::with_capacity(2);
        assert_eq!(
            sample.allocated_bytes(),
            sample.rows.capacity() * size_of::<ccountry::OrderInstanceCounter>()
        );
        sample.read = Some(ccountry::OrderInstanceCounterRead {
            len: 2,
            required: 5,
            unreadable: 0,
        });
        let original = sample.allocated_bytes();
        sample.grow(3);
        assert_eq!(sample.rows.len(), 5);
        assert!(sample.allocated_bytes() > original);
        assert_eq!(
            sample.allocated_bytes(),
            sample.rows.capacity() * size_of::<ccountry::OrderInstanceCounter>()
        );
        let grown = sample.allocated_bytes();
        sample.reset();
        sample.grow(1);
        assert_eq!(sample.allocated_bytes(), grown);
        assert_eq!(sample.rows.len(), 5);
    }

    #[test]
    fn divisions_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = DivisionsJob::default();
        job.census.reserve_exact(3);
        job.sources.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &DivisionsJob| vector_bytes(&job.census) + vector_bytes(&job.sources);
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

    #[test]
    fn callback_time_waits_for_decode_and_includes_the_game_thread_tail() {
        let base = AtomicI64::new(0);
        let tail = AtomicI64::new(0);
        assert_eq!(completed_callback_ticks(&base, &tail), None);
        base.store(17, Ordering::Release);
        assert_eq!(completed_callback_ticks(&base, &tail), None);
        tail.store(3, Ordering::Release);
        assert_eq!(completed_callback_ticks(&base, &tail), Some(20));
        base.store(0, Ordering::Release);
        assert_eq!(completed_callback_ticks(&base, &tail), None);
        base.store(23, Ordering::Release);
        assert_eq!(completed_callback_ticks(&base, &tail), Some(26));
    }

    #[test]
    fn matrix_window_stays_closed_after_its_last_success() {
        let remaining = AtomicUsize::new(200);
        for _ in 0..200 {
            complete_matrix_check(&remaining);
        }
        assert_eq!(remaining.load(Ordering::Relaxed), 0);
        complete_matrix_check(&remaining);
        assert_eq!(remaining.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pool_growth_preserves_reserve_threshold_and_hard_queue_bound() {
        assert_eq!(pool_growth(FREE_RESERVE + 1, INITIAL_BUFFERS), 0);
        assert_eq!(pool_growth(FREE_RESERVE, INITIAL_BUFFERS), GROW_BUFFERS);
        assert_eq!(pool_growth(0, BUFFERS - 1), 1);
        assert_eq!(pool_growth(0, BUFFERS), 0);
        assert_eq!(pool_growth(0, BUFFERS + 1), 0);
    }

    #[test]
    fn decode_queue_bound_matches_pool_invariant() {
        let shared = Shared::new(0, 0, 1);
        assert_eq!(shared.decode_queue.capacity(), BUFFERS);
        assert_eq!(shared.free.capacity(), BUFFERS);
        assert_eq!(shared.ready.capacity(), BUFFERS);
    }

    #[test]
    fn decode_queue_full_takes_the_existing_gap_branch_with_existing_counters() {
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        for seq in 0..BUFFERS {
            shared
                .decode_queue
                .push(decode::DecodeJob {
                    index,
                    seq: seq as u64,
                    hour: 0,
                    relations_refresh: false,
                    characters_refresh: false,
                    projects_refresh: false,
                    lexical_refresh: false,
                    divisions_refresh: false,
                    focus_refresh: false,
                    air_wing_refresh: false,
                    air_stats_refresh: false,
                    naval_refresh: false,
                    players_refresh: false,
                    session_refresh: false,
                    production_rest_refresh: false,
                    construction_rest_refresh: false,
                    economy_rest_refresh: false,
                    air_bases_refresh: false,
                    air_history_refresh: false,
                    supremacy_refresh: false,
                    sunk_refresh: false,
                    radar_refresh: false,
                    intel_refresh: false,
                    weather_refresh: false,
                    capitulation_refresh: false,
                    force_full: false,
                    game_elapsed: 1,
                    relations_payload: None,
                    characters_payload: None,
                    projects_payload: None,
                    lexical_payload: None,
                    divisions_payload: None,
                    focus_payload: None,
                    air_wing_payload: None,
                    air_stats_payload: None,
                    naval_payload: None,
                    players_payload: None,
                    session_payload: None,
                    production_rest_payload: None,
                    construction_rest_payload: None,
                    economy_rest_payload: None,
                    air_bases_payload: None,
                    air_history_payload: None,
                    supremacy_payload: None,
                    sunk_payload: None,
                    radar_payload: None,
                    intel_payload: None,
                    weather_payload: None,
                    capitulation_payload: None,
                })
                .unwrap();
        }
        // The game thread never blocks: a full decode FIFO drops the whole
        // frame through the existing gap path with the existing counters and
        // no new loss reasons.
        let extra = decode::DecodeJob {
            index,
            seq: u64::MAX,
            hour: 0,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 1,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        assert!(shared.decode_queue.push(extra).is_err());
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.clock_after = Some(clock(42, 7));
        let dropped_before = shared.dropped.load(Ordering::Relaxed);
        drop_owned_frame(&shared, frame, index, Some(clock(41, 7)));
        assert_eq!(shared.dropped.load(Ordering::Relaxed), dropped_before + 1);
        assert_eq!(shared.ready.len(), 0);
        assert_eq!(shared.free.len(), INITIAL_BUFFERS);
        let pending = unsafe { &*shared.pending.get() };
        let runs: Vec<_> = pending.iter().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].count),
            (42, 42, 1)
        );
        assert_eq!(runs[0].loaded_hour, Some(41));
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
    fn state_comparison_storage_survives_until_both_state_windows_finish() {
        let mut extended_successful = [CONSISTENCY_TARGET; TABLE_COUNT];
        extended_successful[replay::rows::TableId::StateFlags.index()] = CONSISTENCY_TARGET - 1;
        let mut capacities = INITIAL_CAPACITIES;
        let mut frame = Captured::with_capacities(
            &capacities,
            &MapCapacities::default(),
            [CONSISTENCY_TARGET; CONSISTENCY_TABLES.len()],
            &extended::Capacities::default(),
            extended_successful,
        );
        assert_eq!(frame.states_second.len(), capacities[3]);
        frame.grow(&mut capacities);
        assert_eq!(frame.states_second.len(), capacities[3]);
        frame.extended_successful[replay::rows::TableId::StateFlags.index()] = CONSISTENCY_TARGET;
        frame.grow(&mut capacities);
        assert!(frame.states_second.is_empty());

        frame.consistency_successful[1] = CONSISTENCY_TARGET - 1;
        frame.grow(&mut capacities);
        assert_eq!(frame.states_second.len(), capacities[3]);
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

    fn cadence_clock(hour: i32, signal: i64) -> Clock {
        clock(hour, signal)
    }

    #[test]
    fn cadence_rotation_walks_expected_families_on_hours_h_through_h_plus_7() {
        use replay::cadence::{is_refresh, refresh_slot};
        use replay::rows::TableId;
        for base in [0, 1, 60_820_531] {
            for offset in 0..8 {
                let hour: i32 = base + offset;
                let slot = hour.rem_euclid(4);
                for table in TableId::ALL {
                    let expected = match refresh_slot(table) {
                        None => true,
                        Some(s) => s as i32 == slot,
                    };
                    assert_eq!(is_refresh(table, hour), expected, "{table:?} at {hour}");
                    let walk = replay::cadence::should_walk(table, hour, false);
                    assert_eq!(walk, expected, "{table:?} walk at {hour}");
                    assert!(replay::cadence::should_walk(table, hour, true));
                }
            }
        }
    }

    #[test]
    fn branch_starts_keyframes_and_gaps_force_full_refresh() {
        let mut cadence = CadenceState::default();
        assert!(cadence.force_full(&cadence_clock(100, 1)));
        assert!(!cadence.force_full(&cadence_clock(101, 1)));
        assert!(!cadence.force_full(&cadence_clock(102, 1)));
        assert!(cadence.force_full(&cadence_clock(103, 2)));
        assert!(cadence.force_full(&cadence_clock(500, 2)));
        assert!(!cadence.force_full(&cadence_clock(501, 2)));
        assert!(cadence.force_full(&cadence_clock(500 + 720, 2)));
        let mut fresh = CadenceState::default();
        assert!(fresh.force_full(&cadence_clock(10, 1)));
        let mut changed = cadence_clock(11, 1);
        changed.campaign_id.bytes[0] ^= 1;
        assert!(fresh.force_full(&changed));
    }

    #[test]
    fn consistency_windows_count_refreshes_not_wall_hours() {
        use replay::cadence::should_walk;
        use replay::rows::TableId;
        let mut window = ConsistencyWindow::<TABLE_COUNT>::default();
        let table = TableId::Characters.index();
        let mut successes = 0;
        for hour in 0..16 {
            let walk = should_walk(TableId::Characters, hour, false);
            let mut matched = [false; TABLE_COUNT];
            matched[table] = walk;
            let counts = window.commit(matched, true, true);
            if walk {
                successes += 1;
                assert_eq!(counts[table], successes);
            } else {
                assert_eq!(counts[table], successes);
            }
        }
        assert_eq!(successes, 4);
        assert_eq!(window.successful[table], 4);
    }

    #[test]
    fn wars_table_is_hourly_tier_h_with_no_refresh_gate() {
        // WarRelations keeps its hourly cadence: no stagger slot, every hour
        // walks. This pins the unchanged cadence for typed capture.
        use replay::rows::TableId;
        assert!(!replay::cadence::is_staggered(TableId::WarRelations));
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(replay::cadence::is_refresh(TableId::WarRelations, hour));
        }
    }

    #[test]
    fn wars_typed_walk_preserves_native_rows_and_independent_matrix() {
        use super::wars_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let split = kit::direct(&memory, 64);
        // Spot-checks: the owned walk emits each war once from its first
        // endpoint; the matrix census covers both in slot order.
        assert_eq!(split.owned_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.owned[0].first_country, 1);
        assert_eq!(split.owned[0].second_country, 2);
        assert_eq!(split.owned[0].start_hour, i32::MIN);
        assert_eq!(split.owned[0].first_casualties, i64::MAX);
        assert_eq!(split.owned[1].first_country, 3);
        assert_eq!(split.owned[1].second_country, 1);
        assert_eq!(split.owned[1].first_casualties, 11);
        assert_eq!(split.matrix_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.matrix[0].first_country, 1);
        assert_eq!(split.matrix[1].first_country, 3);
    }

    #[test]
    fn wars_typed_walk_preserves_truncated_censuses() {
        use super::wars_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let split = kit::direct(&memory, 1);
        assert_eq!(
            split.owned_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            split.matrix_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn wars_typed_capacity_matrix_keeps_full_validation_and_exact_unread_counts() {
        use super::wars_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for cap in [0usize, 1, 2, 64] {
            let direct = kit::direct(&memory, cap);
            let expected = if cap >= 2 {
                Outcome::Rows { len: 2 }
            } else {
                Outcome::Truncated {
                    len: cap,
                    unread_rows: (2 - cap) as u32,
                }
            };
            assert_eq!(direct.owned_outcome, expected);
            assert_eq!(direct.matrix_outcome, expected);
        }
    }

    #[test]
    fn wars_typed_owned_and_matrix_fault_scopes_remain_independent() {
        use super::wars_test_kit as kit;
        use engine::{ccountry, cdiplomacy_status, cgame_state, crelation, memory::SliceMemory};
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        // A failure in an owned-only root must not invalidate the independent
        // matrix census, and matrix-only failures must not invalidate owned rows.
        for (fault, matched) in [
            (kit::GAME_STATE + cgame_state::COUNTRIES, [false, false]),
            (kit::GAME_STATE + cgame_state::COUNTRY_LINKS, [false, false]),
            (kit::CTAB + 8, [false, false]),
            (kit::LTAB + 4, [false, false]),
            (kit::C0, [false, false]),
            (kit::C0 + ccountry::ID, [false, false]),
            (kit::C0 + ccountry::DIPLOMACY_STATUS, [false, false]),
            (kit::D0, [false, false]),
            (kit::WAR1 + crelation::TYPE_ID, [false, false]),
            (kit::WAR1 + crelation::START_HOUR, [false, false]),
            (kit::D0 + cdiplomacy_status::OWNED_RELATIONS, [false, true]),
            (kit::O0 + 8, [false, true]),
            (kit::OTHER + crelation::TYPE_ID, [false, true]),
            (kit::RETIRED + crelation::TYPE_ID, [false, true]),
            (kit::D0 + cdiplomacy_status::RELATIONS, [false, false]),
            (kit::M0 + 8, [true, false]),
            (kit::status(1, 2), [true, false]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory, 64);
            assert_eq!(
                [
                    matches!(direct.owned_outcome, Outcome::Rows { .. }),
                    matches!(direct.matrix_outcome, Outcome::Rows { .. })
                ],
                matched,
                "fault={fault:x}",
            );
        }
    }

    #[test]
    fn wars_typed_comparison_blocks_checks_on_changed_native_rows() {
        use super::wars_test_kit as kit;
        let bytes = kit::fixture();
        let first = kit::direct(&kit::memory(&bytes), 64);
        let mut torn = bytes.clone();
        torn[kit::WAR1 + engine::crelation::START_HOUR] ^= 0xff;
        let second = kit::direct(&kit::memory(&torn), 64);
        let mismatch = mismatches(
            first.owned_outcome,
            &first.owned,
            second.owned_outcome,
            &second.owned,
        );
        assert_eq!(mismatch, 1);
        assert!(!full_match(
            first.owned_outcome,
            second.owned_outcome,
            mismatch
        ));
    }

    #[test]
    fn divisions_tables_are_hourly_tier_h_with_no_refresh_gate() {
        // The whole group keeps its hourly cadence: no stagger slot, every
        // hour walks. This pins the no-cadence-change rule for the split.
        for table in crate::decode::DIVISIONS {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(replay::cadence::is_refresh(table, hour));
            }
        }
    }

    #[test]
    fn divisions_sequential_readers_reset_recycled_duplicate_cursors() {
        use engine::{Memory, memory::SliceMemory};

        let mut log = RawLog::<6>::new(3, 16);
        let address = 0x1000;
        for (value, readable) in [(11, 8), (0, 0), (29, 8)] {
            let bytes = [value; 8];
            let memory = SliceMemory {
                base: address,
                bytes: &bytes[..readable],
            };
            assert_eq!(log.take(&memory, address, 8, 1).is_some(), readable != 0);
        }
        // The census and detail stages each start at the first observation,
        // even when the prior reader consumed a fault and the terminal repeat.
        // Dropping each reader makes its index available to the next stage.
        for stage in ["census", "detail"] {
            let recorded = Recorded::build(&log);
            let mut output = [0; 8];
            assert!(recorded.copy(address, &mut output), "{stage}");
            assert_eq!(output, [11; 8], "{stage}");
            assert!(!recorded.copy(address, &mut output), "{stage}");
            assert_eq!(output, [11; 8], "failed read leaves output unchanged");
            assert!(recorded.copy(address, &mut output), "{stage}");
            assert_eq!(output, [29; 8], "{stage}");
            assert!(recorded.copy(address, &mut output), "{stage}");
            assert_eq!(output, [29; 8], "terminal repeat");
            drop(recorded);
        }
    }

    #[test]
    fn divisions_split_copy_matches_direct_walk() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory, [64; 6]);
        let (split, report, _) = kit::split(&memory, [64; 6]);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: null location/template/orders read as zero rows.
        assert_eq!(split.divisions_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.divisions[0].id, 101);
        assert_eq!(split.divisions[0].province, 1);
        assert_eq!(split.divisions[0].hit_points, 100_000);
        assert_eq!(split.divisions[1].province, 0);
        assert_eq!(split.divisions[2].template, 0);
        assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.equipment[1].amount_raw, -50);
        assert_eq!(split.groups_outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.groups[0].priority, 3);
        assert_eq!(split.requests_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.deliveries_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.deliveries[0].progress_raw, Some(300));
        assert_eq!(split.produced_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.produced[0].amount_raw, 9);
    }

    #[test]
    fn divisions_split_copy_matches_direct_walk_under_truncation() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory, [1; 6]);
        let (split, report, _) = kit::split(&memory, [1; 6]);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        kit::assert_matches_direct(&split, &direct);
        // A truncated divisions walk makes every dependent census
        // incomplete on both paths with the identical reason.
        assert_eq!(
            split.divisions_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        for outcome in [
            split.equipment_outcome,
            split.groups_outcome,
            split.requests_outcome,
            split.deliveries_outcome,
            split.produced_outcome,
        ] {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: "source census incomplete"
                }
            );
        }
    }

    #[test]
    fn divisions_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::divisions_test_kit as kit;
        use engine::{
            carmy_upgrades_requests, cequipment_variant_pool, cgame_state, cprovince, cunit,
            cupgrade_delivery, memory::SliceMemory,
        };
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        // (fault address, expected raw-compare verdicts in group order).
        // Outcomes including reason strings must match the typed walks
        // exactly; verdicts follow the fault-attribution masks, with the
        // shared census blocking every slot.
        let group = 0x60070usize;
        for (fault, matched) in [
            (kit::GAME_STATE + cgame_state::COUNTRIES, [false; 6]),
            (kit::CTAB + 8, [false; 6]),
            (kit::C0, [false; 6]),
            (kit::C0 + engine::ccountry::ID, [false; 6]),
            (kit::C0 + engine::ccountry::DIVISIONS, [false; 6]),
            (0x23000, [false; 6]),
            (kit::A1, [false; 6]),
            (kit::A1 + cunit::HANDLE, [false; 6]),
            (kit::A1 + cunit::ROSTER_OWNER, [false; 6]),
            (kit::A1 + cunit::LOCATION, [false; 6]),
            (kit::P1, [false; 6]),
            (kit::P1 + cprovince::ID, [false; 6]),
            (kit::A1 + carmy::STANDARD_TEMPLATE, [false; 6]),
            (0x40000, [false; 6]),
            (kit::A1 + carmy::STRENGTH, [false; 6]),
            (kit::A1 + carmy::ORDERS_GROUP, [false; 6]),
            (0x41000, [false; 6]),
            (
                kit::A1 + carmy::EQUIPMENT_POOL,
                [true, false, true, true, true, true],
            ),
            (
                kit::A1 + carmy::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
                [true, false, true, true, true, true],
            ),
            (0x42000, [true, false, true, true, true, true]),
            // V1 backs equipment entries and the first request row.
            (0x50000, [true, false, true, false, true, true]),
            (
                kit::A1 + carmy::REQUESTS,
                [true, true, false, false, false, false],
            ),
            (group, [true, true, false, false, false, false]),
            (
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
                [true, true, false, false, false, false],
            ),
            (
                group + carmy_upgrades_requests::REQUESTS_DATA,
                [true, true, true, false, true, true],
            ),
            (group + 0x200, [true, true, true, false, true, true]),
            (
                group + carmy_upgrades_requests::DELIVERIES_DATA,
                [true, true, true, true, false, false],
            ),
            (group + 0x300, [true, true, false, false, false, false]),
            (0x64000, [true, true, true, true, false, false]),
            (
                0x64000 + cupgrade_delivery::STATUS,
                [true, true, true, true, false, false],
            ),
            (
                0x64000 + cupgrade_delivery::PROGRESS,
                [true, true, true, true, false, true],
            ),
            (
                0x64000 + cupgrade_delivery::TOTAL_PROGRESS,
                [true, true, true, true, false, true],
            ),
            (
                0x64020 + cequipment_variant_pool::ALLOW_ZERO,
                [true, true, true, true, false, true],
            ),
            (
                0x64020 + cequipment_variant_pool::ITEMS,
                [true, true, true, true, true, false],
            ),
            (0x64100, [true, true, true, true, true, false]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let direct = kit::direct(&memory, [64; 6]);
            let (split, report, _) = kit::split(&memory, [64; 6]);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn divisions_double_copy_compare_blocks_checks_on_tearing() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        // Same-hour tearing between the passes changes the log.
        let mut torn = bytes.clone();
        torn[kit::A1 + engine::carmy::STRENGTH] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut after = DivisionsJob::new();
        let mut after_torn = DivisionsJob::new();
        let no_second: Option<&mut DivisionsJob> = None;
        let clean_report = copy_divisions_refresh(&memory, ctx, &mut after, no_second);
        assert!(clean_report.raw_ok);
        let torn_report = copy_divisions_refresh(&torn_memory, ctx, &mut after_torn, None);
        assert!(torn_report.raw_ok);
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    fn divisions_presentation_memory() -> impl engine::Memory {
        use super::divisions_test_kit as kit;
        use engine::csub_unit_definition as stats;
        struct Memory(Vec<u8>);
        impl engine::Memory for Memory {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == carmy::COUNTER_ATTRITION_THRESHOLD_RVA && output.len() == 8 {
                    output.copy_from_slice(&500_i64.to_le_bytes());
                    return true;
                }
                kit::memory(&self.0).copy(address, output)
            }
        }
        let mut bytes = kit::fixture();
        // Null map entities and disabled training need no render/global reads.
        // Per-division effective stats are the only extra heap input.
        let stats = 0x65000_usize;
        bytes[stats..stats + 8].copy_from_slice(&(stats::VTABLE_RVA as u64).to_le_bytes());
        bytes[stats + stats::MAXIMUM_STRENGTH..stats + stats::MAXIMUM_STRENGTH + 8]
            .copy_from_slice(&500_000_i64.to_le_bytes());
        bytes[stats + stats::MAXIMUM_ORGANISATION..stats + stats::MAXIMUM_ORGANISATION + 8]
            .copy_from_slice(&700_000_i64.to_le_bytes());
        for army in [kit::A1, kit::A2, kit::A3] {
            bytes[army + cunit::STAT_BLOCK..army + cunit::STAT_BLOCK + 8]
                .copy_from_slice(&(stats as u64).to_le_bytes());
        }
        Memory(bytes)
    }

    #[test]
    fn divisions_raw_success_supplies_both_live_companions_before_decode_without_erasing_them() {
        use super::divisions_test_kit as kit;
        use replay::frame::TableView;
        use replay::rows::{TableId, TableRows};
        let memory = divisions_presentation_memory();
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(first.source_outcome(64), Outcome::Rows { len: 3 });
        assert_eq!(first.sources, second.sources);
        assert_eq!(first.sources[0].division, 101);
        assert_eq!(first.sources[0].template, 0x3455_0000_0034);
        assert_eq!(first.sources[0].country, 1);
        let mut frame = Captured::new();
        first.supply_sources(&mut frame.extended, frame.divisions.len(), false);
        second.supply_sources(&mut frame.extended, frame.divisions_second.len(), true);
        frame.extended.set_divisions_raw_mode(true);
        let mut needed = [false; TABLE_COUNT];
        needed[TableId::DivisionMovement.index()] = true;
        needed[TableId::DivisionStatus.index()] = true;
        let (matched, ticks) =
            frame
                .extended
                .capture(&memory, 0, kit::GAME_STATE, 0, &needed, false);
        for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
            assert!(
                matched[table.index()],
                "{table:?}: {:?}",
                frame.extended.outcomes()[table.index()]
            );
            assert!(ticks[table.index()] > 0);
            assert_eq!(
                frame.extended.outcomes()[table.index()],
                Outcome::Rows { len: 3 }
            );
        }
        let before = frame.extended.test_division_presentation_tables();
        let [
            TableView::Complete(TableRows::DivisionMovement(movement)),
            TableView::Complete(TableRows::DivisionStatus(status)),
        ] = &before
        else {
            panic!("complete presentation tables");
        };
        assert_eq!(
            movement.iter().map(|r| r.division).collect::<Vec<_>>(),
            [101, 102, 103]
        );
        assert!(
            status
                .iter()
                .all(|r| r.maximum_hit_points == 500_000 && r.maximum_organisation == 700_000)
        );
        // This is the actual worker path. Sources must reset before the worker
        // visitor, but these previously acquired live tables must survive.
        frame.extended.set_divisions_raw_mode(true);
        decode_divisions_payload(&mut frame, 0, &first, &mut [0; 6]);
        assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
        assert_eq!(
            frame
                .extended
                .test_division_detail()
                .division_equipment
                .outcome,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(frame.extended.test_division_presentation_tables(), before);
    }

    #[test]
    fn divisions_copied_sources_distinguish_semantic_faults_capacity_and_empty_census() {
        use super::divisions_test_kit as kit;
        let ctx = DivisionsCopyContext {
            base: 0,
            game_state: kit::GAME_STATE,
        };
        for fault in [
            kit::A2,
            kit::A2 + carmy::STANDARD_TEMPLATE,
            0x41000,
            0x41008,
        ] {
            let mut bytes = kit::fixture();
            bytes[fault..fault + 8].fill(0xff);
            let mut first = DivisionsJob::new();
            let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
            assert!(report.raw_ok); // No overflow is not a successful census.
            assert!(matches!(
                first.source_outcome(64),
                Outcome::Unavailable { .. }
            ));
            if fault == kit::A2 || fault == kit::A2 + carmy::STANDARD_TEMPLATE {
                assert_eq!(
                    first.source_outcome(1),
                    Outcome::Truncated {
                        len: 1,
                        unread_rows: 2
                    }
                );
            }
        }
        let mut bytes = kit::fixture();
        for address in [kit::CTAB + 8, kit::CTAB + 16, kit::CTAB + 24] {
            bytes[address..address + 8].fill(0);
        }
        let mut first = DivisionsJob::new();
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(matches!(
            first.source_outcome(0),
            Outcome::Unavailable { .. }
        ));
        assert!(!first.census_complete);
        let mut bytes = kit::fixture();
        for country in [kit::C0, kit::C1] {
            bytes[country + ccountry::DIVISIONS..country + ccountry::DIVISIONS + 16].fill(0);
        }
        copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
        assert_eq!(first.source_outcome(0), Outcome::Rows { len: 0 });
        assert!(first.sources.is_empty()); // Recycled rows were cleared.
        first.sources = Vec::new();
        let bytes = kit::fixture();
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
        assert!(!report.raw_ok && first.log.overflowed);
        assert!(matches!(
            first.source_outcome(64),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn divisions_live_companion_checks_do_not_accept_missing_or_failed_second_sources() {
        use super::divisions_test_kit as kit;
        use replay::rows::TableId;
        let memory = divisions_presentation_memory();
        let ctx = DivisionsCopyContext {
            base: 0,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        copy_divisions_refresh(&memory, ctx, &mut first, None);
        let mut needed = [false; TABLE_COUNT];
        needed[TableId::DivisionMovement.index()] = true;
        needed[TableId::DivisionStatus.index()] = true;
        for failed in [false, true] {
            let mut frame = Captured::new();
            first.supply_sources(&mut frame.extended, frame.divisions.len(), false);
            if failed {
                DivisionsJob::default().supply_sources(
                    &mut frame.extended,
                    frame.divisions_second.len(),
                    true,
                );
            }
            let (matched, _) =
                frame
                    .extended
                    .capture(&memory, 0, kit::GAME_STATE, 0, &needed, false);
            for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
                assert!(!matched[table.index()]);
                assert!(matches!(
                    frame.extended.outcomes()[table.index()],
                    Outcome::Unavailable { .. }
                ));
            }
        }
        // A retired parent window still has a second raw job when only a new
        // companion needs its independent check; no core check is fabricated.
        for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
            let mut child_only = [false; TABLE_COUNT];
            child_only[table.index()] = true;
            assert!(extended::Captured::needs_division_check(&child_only));
            let mut second = DivisionsJob::new();
            copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
            let mut frame = Captured::new();
            first.supply_sources(&mut frame.extended, frame.divisions.len(), false);
            second.supply_sources(&mut frame.extended, frame.divisions_second.len(), true);
            let (matched, _) =
                frame
                    .extended
                    .capture(&memory, 0, kit::GAME_STATE, 0, &child_only, false);
            assert!(matched[table.index()]);
        }
    }

    #[test]
    fn divisions_partition_equal_isolates_equipment_private_bytes() {
        use super::divisions_test_kit as kit;
        // Tear exactly one table's private bytes: equipment spans carry
        // EQUIPMENT_MASK (0x02) only, so only slot 1 flips.
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        let equip_idx = second
            .log
            .spans
            .iter()
            .position(|span| span.ok && span.mask == 0x02)
            .expect("equipment-private span");
        let start = second.log.spans[equip_idx].start as usize;
        second.log.bytes[start] ^= 0xff;
        for slot in 0..6 {
            let equal = first.log.partition_equal(&second.log, slot);
            if slot == 1 {
                assert!(!equal, "equipment partition must flip");
            } else {
                // Shared census/row spans (0x3F) still agree; other
                // private tables (groups/requests/deliveries/produced)
                // do not include equipment bytes.
                // Note: divisions slot 0 includes census+rows only, so it
                // stays true when only equipment private tears.
                assert!(equal, "slot {slot} must stay true");
            }
        }
    }

    #[test]
    fn divisions_staging_overflow_falls_back_to_typed_walk() {
        use super::divisions_test_kit as kit;
        use crate::raw::RawLog;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        first.log = RawLog::new(4, 32);
        let mut second = DivisionsJob::new();
        second.log = RawLog::new(4, 32);
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(!report.raw_ok);
        assert!(first.log.grow_for_next());
        // The typed walk still produces the identical rows through the
        // existing path, so the fallback keeps every row.
        let direct = kit::direct(&memory, [64; 6]);
        assert_eq!(direct.divisions_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.groups_outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn divisions_copy_decode_counters_keep_game_thread_cost_separate() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(report.copy_ticks[0] > 0);
        let mut frame = Captured::new();
        frame.table_ns[6] = report.copy_ticks[0];
        let before = frame.table_ns;
        let mut ticks = [0; 6];
        super::decode_divisions_payload(&mut frame, kit::BASE, &first, &mut ticks);
        assert_eq!(frame.table_ns, before);
        assert!(ticks[0] > 0);
        assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
    }

    #[test]
    fn divisions_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero-cap / one-row / exact-fit (3) / ample run typed AND
        // copy-decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // The 3-division fixture makes cap 1 the overflow-by-one arm for
        // divisions (total 3, truncated with dependents unavailable); cap 0
        // the massive/zero-rows arm.
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for cap in [0usize, 1, 3, 64] {
            let caps = [cap; 6];
            let direct = kit::direct(&memory, caps);
            let (split, report, _) = kit::split(&memory, caps);
            assert!(report.raw_ok, "cap={cap}");
            assert_eq!(report.matched, [true; 6], "cap={cap}");
            kit::assert_matches_direct(&split, &direct);
        }
        let (exact, _, _) = kit::split(&memory, [3; 6]);
        assert_eq!(exact.divisions_outcome, Outcome::Rows { len: 3 });
    }
}
