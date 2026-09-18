//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use crate::{
    acquisition_factions, acquisition_jobs as acquisition, acquisition_states, acquisition_wars,
    decode, extended,
    map_capture::{MapCapacities, MapCaptured},
    roster::Roster,
    table_capture::{comparison_rows, grow_capacity},
};
use crossbeam_queue::ArrayQueue;
#[cfg(test)]
use engine::cunit;
use engine::{
    carmy, ccountry, cfaction, cfaction_system,
    cgame_state::{self, Clock},
    country_memory::{Cache, SeedStatus},
    cprovince, cstate, cwar_relation,
    memory::LiveMemory,
    msvc_string::Text,
    save_allocation_counters::{self, AllocationCounters},
    walk::Outcome,
};
use replay::frame::TABLE_COUNT;
use std::{
    cell::UnsafeCell,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, OnceLock,
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
/// Where the collector samples: at the entry of the hooked hourly tick, before
/// the trampoline runs, so a frame labelled with +468 is the state at the end
/// of that hour. Recorded in every session so readers know which instant it is.
pub const SAMPLE_BOUNDARY: replay::identity::SampleBoundary =
    replay::identity::SampleBoundary::EndOfHour;
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
    pub terminal_counters: Option<AllocationCounters>,
    pub terminal_qpc_begin: i64,
    pub terminal_qpc_end: i64,
    pub terminal_elapsed_ns: u64,
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
            terminal_counters: None,
            terminal_qpc_begin: 0,
            terminal_qpc_end: 0,
            terminal_elapsed_ns: 0,
        }
    }

    fn reset(&mut self) {
        self.forced = false;
        self.read = None;
        self.qpc_begin = 0;
        self.qpc_end = 0;
        self.elapsed_ns = 0;
        self.terminal_counters = None;
        self.terminal_qpc_begin = 0;
        self.terminal_qpc_end = 0;
        self.terminal_elapsed_ns = 0;
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
    /// Frame-owned spare inputs preserve capture when an asynchronous job is
    /// unavailable. They grow only while this frame belongs to the spooler.
    division_fallback: [crate::division_census::Census; 2],
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
        let division_fallback = std::array::from_fn(|_| {
            let mut census = crate::division_census::Census::new();
            census.reserve_sources(capacities[4].max(extended_capacities.division_sources));
            census
        });
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
            division_fallback,
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
            + self
                .division_fallback
                .iter()
                .map(|census| census.allocated_bytes() as usize)
                .sum::<usize>()
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + (self.provinces.capacity() + self.provinces_second.capacity())
                * size_of::<cprovince::Row>()
            + (self.states.capacity() + self.states_second.capacity()) * size_of::<cstate::Row>()
            + self.divisions.capacity() * size_of::<carmy::Row>()
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

    pub(crate) fn take_states_request(
        &mut self,
        base: usize,
        game_state: usize,
        needed: bool,
        flags_needed: bool,
    ) -> acquisition_states::Request {
        acquisition_states::Request {
            census: acquisition::GatherRequest {
                base,
                game_state,
                needed,
                buffers: acquisition::GatherBuffers::take(
                    &mut self.states,
                    &mut self.states_second,
                ),
            },
            flags_needed,
            visitor: self.extended.take_state_visitor(),
        }
    }

    pub(crate) fn restore_states_request(
        &mut self,
        mut request: acquisition_states::Request,
        report: acquisition_states::Report,
        failure: Option<&'static str>,
    ) -> acquisition_states::Report {
        let needed = request.census.needed;
        let report = failure.map_or(report, |reason| {
            request.visitor.fail_sources(reason);
            acquisition_states::Report::failed(needed, reason)
        });
        request
            .census
            .buffers
            .restore(&mut self.states, &mut self.states_second);
        self.extended.restore_state_visitor(request.visitor);
        self.states_result = report.census.result;
        self.consistency_checked[1] = needed;
        self.states_mismatches = if needed { report.census.mismatches } else { 0 };
        report
    }

    pub(crate) fn take_wars_request(
        &mut self,
        countries: &Arc<Cache>,
        base: usize,
        game_state: usize,
        needed: bool,
        matrix_needed: bool,
    ) -> acquisition_wars::Request {
        acquisition_wars::Request {
            countries: Arc::clone(countries),
            table: acquisition::GatherRequest {
                base,
                game_state,
                needed,
                buffers: acquisition::GatherBuffers::take(&mut self.wars, &mut self.wars_second),
            },
            snapshot: std::mem::take(&mut self.wars_snapshot),
            snapshot_sampled: false,
            matrix: std::mem::take(&mut self.wars_matrix),
            matrix_needed,
        }
    }

    pub(crate) fn restore_wars_request(
        &mut self,
        request: acquisition_wars::Request,
        report: acquisition_wars::Report,
        failure: Option<&'static str>,
    ) -> acquisition_wars::Report {
        let needed = request.table.needed;
        let report = failure.map_or(report, |reason| {
            acquisition_wars::Report::failed(reason, request.matrix_needed)
        });
        request
            .table
            .buffers
            .restore(&mut self.wars, &mut self.wars_second);
        self.wars_snapshot = request.snapshot;
        self.wars_snapshot_sampled = request.snapshot_sampled;
        self.wars_matrix = request.matrix;
        self.wars_result = report.table.result;
        self.consistency_checked[3] = needed;
        self.wars_mismatches = if needed { report.table.mismatches } else { 0 };
        self.war_matrix_result = report.matrix;
        report
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
        let division_source_capacity = self.extended.division_capture_capacities().0;
        for census in &mut self.division_fallback {
            if census.overflowed {
                census.grow();
            }
            census.reserve_sources(capacities[4].max(division_source_capacity));
            // A later hour may use a regular job or have no readable clock.
            // Retire this attempt's overflow request so idle spares do not
            // repeatedly double on every frame recycle.
            census.clear();
        }
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
    /// Cumulative game-thread block per hour running the acquisition body before
    /// the trampoline: the whole main-thread exposure. Its per-hour percentiles
    /// live in callback_work (the same body wall, recorded off-thread).
    pub collect_wait_ns: AtomicU64,
    pub acquisition_panics: AtomicU64,
    pub acquisition_dispatch_ns: AtomicU64,
    pub acquisition_wait_ns: AtomicU64,
    /// Disjoint game-thread wall phases of the block, summing to the body wall:
    /// setup+country snapshot, the parallel region (dispatch, inline reads and
    /// every non-ETA join), the serial ETA barrier loop, then residual
    /// verification and enqueue. Game thread writer; phase_calls is the divisor.
    pub phase_setup_ns: AtomicU64,
    pub phase_parallel_ns: AtomicU64,
    pub phase_eta_ns: AtomicU64,
    pub phase_residual_ns: AtomicU64,
    pub phase_calls: AtomicU64,
    /// The two halves of the parallel phase: the game thread's own typed walks
    /// (countries/links/provinces/states/wars/factions) versus the wall it
    /// spends joining the already-dispatched non-ETA pool jobs.
    pub phase_inline_walk_ns: AtomicU64,
    pub phase_noneta_join_ns: AtomicU64,
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
    /// The game thread owns this until ready is published; the spool thread
    /// then takes it once, independent of any particular FIFO buffer.
    pub roster: UnsafeCell<Option<Box<Roster>>>,
    pub roster_ready: AtomicBool,
    /// Only the game thread writes this, inside the hook. The spool worker takes
    /// it after the hook is quiescent.
    pub pending: UnsafeCell<LossBatch>,
    /// Only the game thread updates these per-attach counts, inside the hook.
    consistency: UnsafeCell<ConsistencyWindow<{ CONSISTENCY_TABLES.len() }>>,
    extended_consistency: UnsafeCell<ConsistencyWindow<TABLE_COUNT>>,
    cadence: UnsafeCell<CadenceState>,
    /// Fresh country authentication per hour, verified directly before
    /// publication. Acquisition thread only.
    country_cache: UnsafeCell<Arc<Cache>>,
    /// Independent matrix census supplements the owned-vector double read for 200 full walks.
    war_matrix_remaining: AtomicUsize,
}

// Each frame index has exactly one owner: a queue, the game thread inside the
// hook, or the spool thread. The game thread runs the whole acquisition body
// before the trampoline, so its single-writer fields stay single-writer.
unsafe impl Sync for Shared {}

impl Shared {
    pub fn new(base: usize, trampoline: usize, frequency: u64) -> Self {
        let roster = Box::new(Roster::new());
        let country_cache = Arc::new(Cache::with_capacity(8192));
        let pool_bytes = roster.allocated_bytes() + country_cache.allocated_bytes();
        let shared = Self {
            acquisition: OnceLock::new(),
            collect_wait_ns: AtomicU64::new(0),
            acquisition_panics: AtomicU64::new(0),
            phase_setup_ns: AtomicU64::new(0),
            phase_parallel_ns: AtomicU64::new(0),
            phase_eta_ns: AtomicU64::new(0),
            phase_residual_ns: AtomicU64::new(0),
            phase_calls: AtomicU64::new(0),
            phase_inline_walk_ns: AtomicU64::new(0),
            phase_noneta_join_ns: AtomicU64::new(0),
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
    /// The caller must be the sole pool allocator; no other thread calls this.
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

    /// Called only by the game thread inside the hook. A failed clock postpones
    /// this once-only read until the next readable hour, free buffer or not.
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

/// The hour's read, built at the tick's entry. `index` is None when no frame
/// was free: that hour is recorded as a drop, as before.
struct Dispatch {
    index: Option<usize>,
    entry: Option<Clock>,
    entry_ticks: i64,
    dispatched: i64,
}

/// The hooked hourly boundary, CGameState::HourlyUpdate. Its entry is the
/// settled end-of-hour state (every command drained, tick_busy 0, before the
/// CGameDate increment). The game thread runs the acquisition body here on its
/// own hot core, driving the reader pool and helping drain it; the trampoline
/// runs only after every live read has finished, so the recorded frame is that
/// settled end-of-hour state.
pub unsafe extern "system" fn tick(state: usize, commands: usize) {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let shared = unsafe { &*SHARED.load(Ordering::Acquire) };
    let recording = shared.recording.load(Ordering::Relaxed);
    if recording {
        let memory = unsafe { LiveMemory::new() };
        // Read the clock at entry: +468 still the hour just ended, the trampoline
        // has not run. The frame's identity and the settled state the body reads.
        let clock_start = qpc();
        let entry = cgame_state::read_clock(&memory, shared.base);
        let entry_ticks = qpc() - clock_start;
        let index = shared.free.pop();
        if let Some(index) = index {
            shared.completed[index].store(0, Ordering::Relaxed);
            shared.callback_tail[index].store(0, Ordering::Relaxed);
        }
        let dispatched = qpc();
        // The body reads on this hot core; pool workers read in parallel and
        // this thread helps drain them. A Rust panic must never unwind across
        // the extern boundary into the game: contain it and drop the hour.
        let job = Dispatch {
            index,
            entry,
            entry_ticks,
            dispatched,
        };
        if catch_unwind(AssertUnwindSafe(|| unsafe { acquire(shared, job) })).is_err() {
            // The frame stays out of the pool; its hour is simply not recorded.
            shared.acquisition_panics.fetch_add(1, Ordering::Relaxed);
        } else if let Some(index) = index {
            shared.callback_tail[index].store(1, Ordering::Release);
        }
        // The whole game-thread block before the trampoline: the only exposure.
        // Its percentiles are callback_work; this is only the cheap cumulative.
        shared
            .collect_wait_ns
            .fetch_add(shared.ns(qpc() - dispatched), Ordering::Relaxed);
    }
    // Run the trampoline (this hour's tick) only after every live read finished.
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
    }
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

/// Every live read of the hour that just ended, on the game thread inside the
/// hook. The trampoline has not run, so the state is the settled end-of-hour
/// boundary, read exactly as the original collector did.
unsafe fn acquire(shared: &Shared, dispatch: Dispatch) {
    let Dispatch {
        index,
        entry,
        entry_ticks,
        dispatched,
    } = dispatch;
    let memory = unsafe { LiveMemory::new() };

    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.clock_entry = entry;
        frame.qpc = dispatched;
        frame.table_ns = [0; TABLE_COUNT];
        let before = qpc();
        frame.clock_before = cgame_state::read_clock(&memory, shared.base);
        // Passive scalar reads of the same counters the save header emits.
        frame.counters = frame.clock_before.map(|clock| {
            save_allocation_counters::read_counters(&memory, shared.base, clock.game_state as usize)
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
        let mut production_rest_report = extended::production::ProductionRestCopyReport::default();
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
        // Disjoint block phases (game-thread wall); default to dispatched so a
        // clockless frame lands entirely in residual.
        let mut ts_setup = dispatched;
        let mut ts_parallel = dispatched;
        let mut ts_eta = dispatched;
        // The two halves of the parallel phase.
        let mut inline_walk_ns = 0u64;
        let mut noneta_join_ns = 0u64;
        if let Some(clock) = frame.clock_before {
            let force_full = unsafe { (&mut *shared.cadence.get()).force_full(&clock) };
            let state = clock.game_state as usize;
            let cache_begin = qpc();
            let country_snapshot = Cache::prepare_shared(
                unsafe { &mut *shared.country_cache.get() },
                &memory,
                shared.base,
                state,
            )
            .expect("every country snapshot lease returned before the next callback");
            let cached = country_snapshot.view(&memory);
            frame.country_cache_status = cached.seed_status();
            frame.country_cache_entries = cached.cached_entries();
            let cache_prepare_ticks = qpc() - cache_begin;
            // Every accepted request is owned by this callback-scoped draining guard.
            // Rejected submissions retain their buffers and execute exactly once inline.
            let acquisition = shared.acquisition.get().expect("initialized before hook");
            let mut batch = acquisition::Batch::new(acquisition);
            ts_setup = qpc();
            let wait_at_setup = shared.acquisition_wait_ns.load(Ordering::Relaxed);
            macro_rules! queue {
                ($job:expr) => {{
                    // SAFETY: batch drains all accepted work before this callback returns.
                    unsafe { submit_acquisition(shared, &mut batch, $job) }
                }};
            }
            macro_rules! completed {
                ($pending:expr, $variant:ident) => {{
                    // SAFETY: this thread still owns the live acquisition boundary.
                    let completed = unsafe { finish_acquisition(shared, &mut batch, $pending) };
                    let acquisition::Job::$variant(request) = completed.request else {
                        unreachable!("acquisition ticket retains its submitted job type")
                    };
                    let report = completed.report.map(|report| {
                        let acquisition::Report::$variant(report) = report else {
                            unreachable!("acquisition report retains its submitted job type")
                        };
                        report
                    });
                    let failure = report
                        .as_ref()
                        .err()
                        .copied()
                        .map(acquisition_failure_reason);
                    (request, report.unwrap_or_default(), failure)
                }};
            }
            macro_rules! pair {
                ($acquire:path, $recycle:path, $checking:expr, $ready:expr) => {{
                    let mut first = $acquire();
                    let second = ($checking).then($acquire);
                    if second.as_ref().is_some_and(|job| !($ready)(job)) {
                        $recycle(first);
                        first = Default::default();
                    }
                    (first, second)
                }};
            }
            let characters_refresh = decode::characters_refresh_hour(clock.hour, force_full);
            let projects_refresh = decode::projects_refresh_hour(clock.hour, force_full);
            let lexical_refresh = decode::lexical_refresh_hour(clock.hour, force_full);
            let relations_refresh = decode::relations_refresh_hour(clock.hour, force_full);
            let world_request = acquire_world_request(
                &country_snapshot,
                shared.base,
                state,
                &extended_needed,
                frame.extended.focus_pilot().focus_copy_context(
                    shared.base,
                    state,
                    decode::FOCUS.map(|table| extended_needed[table.index()]),
                ),
                frame.extended.economy_rest_pilot(),
            );
            let world_pending = queue!(acquisition::Job::World(world_request));
            // Provinces has no country/division dependency; dispatch its pointer
            // chase to the pool so it overlaps the other readers. Buffers move
            // out here and are restored where the inline walk used to run.
            let provinces_pending =
                queue!(acquisition::Job::Provinces(acquisition::ProvincesRequest {
                    base: shared.base,
                    game_state: state,
                    needed: needed[PROVINCE_CHECK],
                    buffers: acquisition::GatherBuffers::take(
                        &mut frame.provinces,
                        &mut frame.provinces_second,
                    ),
                }));
            // The state visitor travels with its census. Restore it before
            // StateFlags consumes the two source passes later in this callback.
            let states_pending = queue!(acquisition::Job::States(frame.take_states_request(
                shared.base,
                state,
                needed[1],
                extended::Captured::needs_state_check(&extended_needed),
            )));
            // Both faction tables use this callback's country snapshot and
            // retain separate second-pass windows. All six buffers return at
            // the original factions/member collection point.
            let factions_pending =
                queue!(acquisition::Job::Factions(acquisition_factions::Request {
                    countries: Arc::clone(&country_snapshot),
                    factions: acquisition::GatherRequest {
                        base: shared.base,
                        game_state: state,
                        needed: needed[4],
                        buffers: acquisition::GatherBuffers {
                            first: acquisition_factions::Names::take(
                                &mut frame.factions,
                                &mut frame.faction_names,
                            ),
                            second: acquisition_factions::Names::take(
                                &mut frame.factions_second,
                                &mut frame.faction_names_second,
                            ),
                        },
                    },
                    members: acquisition::GatherRequest {
                        base: shared.base,
                        game_state: state,
                        needed: needed[5],
                        buffers: acquisition::GatherBuffers::take(
                            &mut frame.faction_members,
                            &mut frame.faction_members_second,
                        ),
                    },
                }));
            let countries_pending = queue!(acquisition::Job::Countries(
                acquisition::CountriesRequest::take(
                    &country_snapshot,
                    shared.base,
                    state,
                    needed[0],
                    &mut frame.countries,
                    &mut frame.links,
                    &mut frame.links_second,
                )
            ));
            // Both war censuses depend only on the callback's country snapshot.
            // Their independent window state cannot advance until publication.
            let matrix_needed = shared.war_matrix_remaining.load(Ordering::Relaxed) != 0;
            let wars_pending = queue!(acquisition::Job::Wars(frame.take_wars_request(
                &country_snapshot,
                shared.base,
                state,
                needed[3],
                matrix_needed,
            )));
            let province_unit_order_pending = queue!(acquisition::Job::ProvinceUnitOrder(
                frame.extended.take_province_unit_order_request(
                    shared.base,
                    state,
                    &extended_needed
                )
            ));
            // These hourly families have no World or division dependency.
            // Loan their existing buffers while other acquisition is ready.
            let supply_trains_pending = queue!(acquisition::Job::SupplyTrains(
                frame.extended.take_supply_trains_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    &extended_needed
                )
            ));
            let character_portraits_pending = queue!(acquisition::Job::CharacterPortraits(
                frame.extended.take_character_portraits_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    &extended_needed
                )
            ));
            let (trait_rows, trait_bytes) =
                frame.extended.characters_pilot().trait_capture_capacities();
            let characters_ctx = extended::characters::CharactersCopyContext {
                base: shared.base,
                game_state: state,
                trait_rows,
                trait_bytes,
            };
            let characters_pending = if characters_refresh {
                let (first, second) = pair!(
                    decode::acquire_characters_job,
                    decode::recycle_characters_job,
                    decode::CHARACTERS
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::characters::CharactersJob| job.log.spans.capacity() != 0
                );
                Some(queue!(acquisition::Job::Characters(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: characters_ctx,
                        first,
                        second
                    }
                )))
            } else {
                None
            };
            let relations_ctx = extended::intel::RelationsCopyContext {
                base: shared.base,
                game_state: state,
                opinion_cap: frame.extended.relations_pilot().opinion_rows_cap(),
                tuple_cap: frame.extended.relations_pilot().tuple_rows_cap(),
            };
            let relations_pending = if relations_refresh {
                let (first, second) = pair!(
                    decode::acquire_relations_job,
                    decode::recycle_relations_job,
                    decode::RELATIONS
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::intel::RelationsJob| job.countries.capacity() != 0
                );
                Some(queue!(acquisition::Job::Relations(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: relations_ctx,
                        first,
                        second
                    }
                )))
            } else {
                None
            };
            let players_ctx = crate::map_capture::PlayersCopyContext {
                base: shared.base,
                game_state: state,
                needed: [needed[6], needed[7]],
            };
            let players_pending = {
                let (first, second) = pair!(
                    decode::acquire_players_job,
                    decode::recycle_players_job,
                    needed[6] || needed[7],
                    |job: &crate::map_capture::PlayersJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Players(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: players_ctx,
                    first,
                    second
                }))
            };
            let air_stats_ctx = extended::air::AirStatsCopyContext {
                base: shared.base,
                game_state: state,
            };
            let air_stats_pending = {
                let (first, second) = pair!(
                    decode::acquire_air_stats_job,
                    decode::recycle_air_stats_job,
                    decode::AIR_STATS
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::air::AirStatsJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::AirStats(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: air_stats_ctx,
                    first,
                    second
                }))
            };
            let air_bases_ctx = extended::air::AirBasesCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::AIR_BASES.map(|table| extended_needed[table.index()]),
            };
            let air_bases_pending = {
                let (first, second) = pair!(
                    decode::acquire_air_bases_job,
                    decode::recycle_air_bases_job,
                    decode::AIR_BASES
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::air::AirBasesJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::AirBases(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: air_bases_ctx,
                    first,
                    second
                }))
            };
            let air_history_ctx = extended::air::AirHistoryCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::AIR_HISTORY.map(|table| extended_needed[table.index()]),
                history_limit: frame.extended.air_history_pilot().air_history_limits()[0],
                damage_limit: frame.extended.air_history_pilot().air_history_limits()[1],
            };
            let air_history_pending = {
                let first = decode::acquire_air_history_job();
                let second = (decode::AIR_HISTORY
                    .iter()
                    .any(|table| extended_needed[table.index()]))
                .then(decode::acquire_air_history_job);
                queue!(acquisition::Job::AirHistory(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: air_history_ctx,
                    first,
                    second
                }))
            };
            let supremacy_ctx = extended::naval_history::SupremacyCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::SUPREMACY.map(|table| extended_needed[table.index()]),
            };
            let supremacy_pending = {
                let (first, second) = pair!(
                    decode::acquire_supremacy_job,
                    decode::recycle_supremacy_job,
                    decode::SUPREMACY
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::naval_history::SupremacyJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Supremacy(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: supremacy_ctx,
                    first,
                    second
                }))
            };
            let sunk_ctx = extended::naval_history::SunkCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                needed: decode::SUNK.map(|table| extended_needed[table.index()]),
            };
            let sunk_pending = {
                let (first, second) = pair!(
                    decode::acquire_sunk_job,
                    decode::recycle_sunk_job,
                    decode::SUNK
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::naval_history::SunkJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Sunk(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: sunk_ctx,
                    first,
                    second
                }))
            };
            let radar_ctx = frame.extended.radar_pilot().radar_copy_context(
                shared.base,
                state,
                decode::RADAR.map(|table| extended_needed[table.index()]),
            );
            let radar_pending = {
                let (first, second) = pair!(
                    decode::acquire_radar_job,
                    decode::recycle_radar_job,
                    decode::RADAR
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::fog::RadarJob| job.initialized()
                );
                queue!(acquisition::Job::Radar(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: radar_ctx,
                    first,
                    second
                }))
            };
            let weather_ctx = extended::weather::WeatherCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::WEATHER.map(|table| extended_needed[table.index()]),
            };
            let weather_pending = {
                let (first, second) = pair!(
                    decode::acquire_weather_job,
                    decode::recycle_weather_job,
                    decode::WEATHER
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::weather::WeatherJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Weather(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: weather_ctx,
                    first,
                    second
                }))
            };
            let capitulation_ctx = extended::intel::CapitulationCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::CAPITULATION.map(|table| extended_needed[table.index()]),
            };
            let capitulation_pending = {
                let (first, second) = pair!(
                    decode::acquire_capitulation_job,
                    decode::recycle_capitulation_job,
                    decode::CAPITULATION
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::intel::CapitulationJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Capitulation(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: capitulation_ctx,
                    first,
                    second
                }))
            };
            let projects_ctx = extended::projects::ProjectsCopyContext {
                base: shared.base,
                game_state: state,
                needed: extended_needed,
            };
            let projects_pending = if projects_refresh {
                let (first, second) = pair!(
                    decode::acquire_projects_job,
                    decode::recycle_projects_job,
                    decode::PROJECTS
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::projects::ProjectsJob| job.log.spans.capacity() != 0
                );
                Some(queue!(acquisition::Job::Projects(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: projects_ctx,
                        first,
                        second
                    }
                )))
            } else {
                None
            };
            let lexical_ctx = extended::lexical::LexicalCopyContext {
                base: shared.base,
                needed: extended_needed,
            };
            let lexical_pending = if lexical_refresh {
                let (first, second) = pair!(
                    decode::acquire_lexical_job,
                    decode::recycle_lexical_job,
                    decode::LEXICAL
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::lexical::LexicalJob| job.initialized()
                );
                Some(queue!(acquisition::Job::Lexical(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: lexical_ctx,
                    first,
                    second
                })))
            } else {
                None
            };
            let session_ctx = extended::session::SessionCopyContext {
                base: shared.base,
                game_state: state,
                needed: decode::SESSION.map(|table| extended_needed[table.index()]),
            };
            let session_pending = {
                let (first, second) = pair!(
                    decode::acquire_session_job,
                    decode::recycle_session_job,
                    decode::SESSION
                        .iter()
                        .any(|table| extended_needed[table.index()]),
                    |job: &extended::session::SessionJob| job.log.spans.capacity() != 0
                );
                queue!(acquisition::Job::Session(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: session_ctx,
                    first,
                    second
                }))
            };
            // Naval uses this callback's independently bounded wing-membership census.
            let naval_second_needed = decode::NAVAL_ORDER
                .iter()
                .any(|table| extended_needed[table.index()]);
            let (mut naval_first, mut naval_second) = pair!(
                decode::acquire_naval_job,
                decode::recycle_naval_job,
                naval_second_needed,
                |job: &extended::naval_history::NavalJob| job.log.spans.capacity() != 0
            );
            let air_second_needed = needed[14]
                || decode::AIR_WING
                    .iter()
                    .chain(decode::NAVAL_ORDER.iter())
                    .any(|table| extended_needed[table.index()]);
            let air_first = decode::acquire_air_wing_job();
            let air_second = air_second_needed.then(decode::acquire_air_wing_job);
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
            let air_pending = queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: air_ctx,
                first: air_first,
                second: air_second,
            }));
            let mut divisions_first = decode::acquire_divisions_job();
            let division_second_needed = needed[2]
                || extended::Captured::needs_division_check(&extended_needed)
                || extended_needed[replay::rows::TableId::UnitIntel.index()];
            let mut divisions_second = division_second_needed.then(decode::acquire_divisions_job);
            // A consistency pass needs both owned buffers; otherwise use
            // the allocation-free typed double read.
            if divisions_second
                .as_ref()
                .is_some_and(|job| !job.initialized())
            {
                decode::recycle_divisions_job(divisions_first);
                divisions_first = Default::default();
            }
            let (division_source_capacity, detail_capacities) =
                frame.extended.division_capture_capacities();
            let divisions_ctx = DivisionsCopyContext {
                base: shared.base,
                game_state: state,
                row_capacity: frame.divisions.len(),
                source_capacity: division_source_capacity,
                detail_capacities,
            };
            // Seed admission remains on this callback. Every consumer below leases these
            // exact packets; a core completion cannot authorize a new seed acquisition.
            let divisions_owned = copy_divisions_inputs_refresh(
                &cached,
                divisions_ctx,
                &mut divisions_first,
                divisions_second.as_mut(),
            );
            let mut divisions_pending = None;
            if divisions_owned {
                frame.extended.install_unit_inputs(
                    divisions_first.census.unit_inputs().map(Arc::clone),
                    divisions_second
                        .as_ref()
                        .and_then(|second| second.census.unit_inputs())
                        .map(Arc::clone),
                    divisions_second.as_ref().is_none_or(|second| {
                        divisions_first.census.unit_inputs_equal(&second.census)
                    }),
                );
            } else {
                let begin = qpc();
                decode::recycle_divisions_job(divisions_first);
                divisions_first = Default::default();
                if let Some(second) = divisions_second.take() {
                    decode::recycle_divisions_job(second);
                }
                frame.extended.set_divisions_raw_mode(false);
                let capacity = frame.divisions.len().max(division_source_capacity);
                frame.division_fallback[0].capture(&cached, shared.base, state, capacity);
                frame.divisions_result =
                    frame.division_fallback[0].install_rows(&mut frame.divisions);
                supply_division_census_sources(
                    &frame.division_fallback[0],
                    &mut frame.extended,
                    division_source_capacity,
                    false,
                );
                if division_second_needed {
                    frame.division_fallback[1].capture(&cached, shared.base, state, capacity);
                    supply_division_census_sources(
                        &frame.division_fallback[1],
                        &mut frame.extended,
                        division_source_capacity,
                        true,
                    );
                    frame.consistency_checked[2] = needed[2];
                    matched[2] = needed[2]
                        && frame.division_fallback[0]
                            .equal(&frame.division_fallback[1], frame.divisions.len());
                    frame.divisions_mismatches = usize::from(needed[2] && !matched[2]);
                }
                frame.extended.install_unit_inputs(
                    frame.division_fallback[0].unit_inputs().map(Arc::clone),
                    division_second_needed
                        .then(|| frame.division_fallback[1].unit_inputs().map(Arc::clone))
                        .flatten(),
                    !division_second_needed
                        || frame.division_fallback[0]
                            .unit_inputs_equal(&frame.division_fallback[1]),
                );
                frame.table_ns[6] = shared.ns(qpc() - begin);
            }
            let presentation_pending = queue!(acquisition::Job::Presentation(
                frame.extended.take_presentation_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    &extended_needed,
                )
            ));
            let unit_first = if divisions_owned {
                divisions_first.census.unit_inputs().map(Arc::clone)
            } else {
                frame.division_fallback[0].unit_inputs().map(Arc::clone)
            };
            let unit_second = if divisions_owned {
                divisions_second
                    .as_ref()
                    .and_then(|second| second.census.unit_inputs())
                    .map(Arc::clone)
            } else if division_second_needed {
                frame.division_fallback[1].unit_inputs().map(Arc::clone)
            } else {
                None
            };
            let unit_matched = if divisions_owned {
                divisions_second
                    .as_ref()
                    .is_none_or(|second| divisions_first.census.intel_equal(&second.census))
            } else {
                !division_second_needed
                    || frame.division_fallback[0].intel_equal(&frame.division_fallback[1])
            };
            if divisions_owned {
                divisions_pending = Some(queue!(acquisition::Job::Divisions(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: divisions_ctx,
                        first: divisions_first,
                        second: divisions_second.take(),
                    }
                )));
            }
            let intel_ctx = frame.extended.intel_pilot().intel_copy_context(
                shared.base,
                state,
                decode::INTEL.map(|table| extended_needed[table.index()]),
            );
            let intel_pending = {
                let (first, second) = pair!(
                    decode::acquire_intel_job,
                    decode::recycle_intel_job,
                    intel_ctx.needed.iter().any(|&needed| needed),
                    |job: &extended::fog::IntelJob| job.initialized()
                );
                queue!(acquisition::Job::Intel(acquisition::IntelRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: intel_ctx,
                    first,
                    second,
                    divisions_first: unit_first.as_ref().map(Arc::clone),
                    divisions_second: unit_second.as_ref().map(Arc::clone),
                    divisions_matched: unit_matched,
                }))
            };
            let eta_checked = extended_needed[replay::rows::TableId::DivisionEta.index()];
            let mut eta_workspaces: [Option<Box<extended::division_eta::Workspace>>;
                extended::division_eta::MAX_RANGES] = std::array::from_fn(|_| None);
            let mut eta_workspace_count = 0;
            for slot in &mut eta_workspaces {
                let Some(workspace) = decode::acquire_eta_workspace() else {
                    break;
                };
                *slot = Some(workspace);
                eta_workspace_count += 1;
            }
            // Range independence requires complete per-range dependency caches.
            // Decide the serial fallback before any ETA acquisition starts.
            let eta_max_ranges = if eta_workspaces
                .iter()
                .flatten()
                .all(|workspace| workspace.supports_ranges())
            {
                eta_workspace_count
            } else {
                1
            };
            let eta_ranges = if eta_workspace_count == 0 {
                frame
                    .extended
                    .fail_eta_capture(eta_checked, "ETA acquisition workspace unavailable");
                0
            } else {
                frame
                    .extended
                    .prepare_eta_ranges(eta_checked, eta_max_ranges)
            };
            let mut eta_pending: [Option<PendingAcquisition>; extended::division_eta::MAX_RANGES] =
                std::array::from_fn(|_| None);
            for (index, workspace) in eta_workspaces.iter_mut().enumerate() {
                if index < eta_ranges {
                    eta_pending[index] =
                        Some(queue!(acquisition::Job::Eta(acquisition::EtaRequest {
                            countries: Arc::clone(&country_snapshot),
                            base: shared.base,
                            state,
                            payload: frame.extended.take_eta_range(index),
                            workspace: workspace.take().expect("prepared ETA workspace"),
                            first: unit_first.as_ref().map(Arc::clone),
                            second: if eta_checked {
                                unit_second.as_ref().map(Arc::clone)
                            } else {
                                None
                            },
                            phase: acquisition::EtaPhase::Acquire { pass: 0 },
                        })));
                } else if let Some(workspace) = workspace.take() {
                    decode::recycle_eta_workspace(workspace);
                }
            }
            // Focus delegation must be settled before the complete Governance job moves.
            let (world, world_report, world_failure) = completed!(world_pending, World);
            production_rest_report = world_report.production;
            construction_rest_report = world_report.construction;
            economy_rest_report = world_report.economy;
            focus_report = world_report.focus;
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
                terminal || production_rest_report.plan_failed || production_rest_report.raw_ok;
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
            if production_rest_report.raw_ok && !terminal && !production_rest_report.plan_failed {
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
            let governance_pending = queue!(acquisition::Job::Governance(
                frame.extended.take_governance_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    &extended_needed
                )
            ));
            let intel_residual_pending = queue!(acquisition::Job::IntelResidual(
                frame.extended.take_intel_residual_request(
                    &country_snapshot,
                    extended::intel::CaptureContext {
                        base: shared.base,
                        game_state: state,
                        hour: clock.hour,
                        force_full,
                        needed: extended::intel::TABLES.map(|table| extended_needed[table.index()]),
                    },
                )
            ));
            // Keep the original collection span. Accepted pool joins in this
            // span are already charged to noneta_join_ns; subtract those from
            // the inline subtotal, including callback helping inside a join.
            let inline_walk_begin = qpc();
            let inline_wait_begin = shared.acquisition_wait_ns.load(Ordering::Relaxed);
            // Restore both tables at their original collection point. Service
            // ticks remain per table; shared authentication is charged below.
            let (countries_request, countries_report, _countries_failure) =
                completed!(countries_pending, Countries);
            countries_request.restore(
                &mut frame.countries,
                &mut frame.links,
                &mut frame.links_second,
            );
            frame.countries_result = countries_report.countries;
            frame.table_ns[2] = shared.ns(countries_report.countries_ticks);
            frame.links_result = countries_report.links.result;
            if needed[0] {
                frame.consistency_checked[0] = true;
                frame.links_mismatches = countries_report.links.mismatches;
                matched[0] = full_match(
                    frame.links_result,
                    countries_report.links.second,
                    frame.links_mismatches,
                );
            }
            frame.table_ns[3] = shared.ns(countries_report.links_ticks);
            // These independent core tables do not feed the division
            // census or ETA: both authenticate their own native roots.
            // Retain the same reads/checks, now overlapping ETA acquisition.
            let begin = qpc();
            // Collect the pool provinces walk and restore its buffers. table_ns[4]
            // is now the join wall; the read cost is on the pool (Role::Provinces).
            let (provinces_request, provinces_report, _provinces_failure) =
                completed!(provinces_pending, Provinces);
            provinces_request
                .buffers
                .restore(&mut frame.provinces, &mut frame.provinces_second);
            frame.provinces_result = provinces_report.result;
            if needed[PROVINCE_CHECK] {
                frame.consistency_checked[PROVINCE_CHECK] = true;
                frame.provinces_mismatches = provinces_report.mismatches;
                matched[PROVINCE_CHECK] = full_match(
                    provinces_report.result,
                    provinces_report.second,
                    provinces_report.mismatches,
                );
            }
            frame.table_ns[4] = shared.ns(qpc() - begin);
            // Collection stays between Provinces and Wars. Restore the visitor
            // without reading flags here; their original later stage is intact.
            let (states_request, states_report, states_failure) =
                completed!(states_pending, States);
            let states_report =
                frame.restore_states_request(states_request, states_report, states_failure);
            matched[1] = needed[1]
                && full_match(
                    states_report.census.result,
                    states_report.census.second,
                    states_report.census.mismatches,
                );
            frame.table_ns[5] = shared.ns(states_report.ticks);
            // Restore all wars storage at the original wars point. Pool read
            // ticks retain distinct table/matrix costs; collection contributes
            // to the callback's existing non-ETA join accounting.
            let (wars_request, wars_report, wars_failure) = completed!(wars_pending, Wars);
            let wars_report = frame.restore_wars_request(wars_request, wars_report, wars_failure);
            matched[3] = needed[3] && wars_report.table_matched();
            wars_matrix_check = wars_report.matrix_eligible();
            frame.table_ns[7] = shared.ns(wars_report.table_ticks);
            frame.war_matrix_ns = shared.ns(wars_report.matrix_ticks);
            let (factions_request, mut factions_report, factions_failure) =
                completed!(factions_pending, Factions);
            if let Some(reason) = factions_failure {
                factions_report =
                    acquisition_factions::Report::failed([needed[4], needed[5]], reason);
            }
            (frame.faction_names_len, frame.faction_names_required) = factions_request.restore(
                &mut frame.factions,
                &mut frame.factions_second,
                &mut frame.faction_names,
                &mut frame.faction_names_second,
                &mut frame.faction_members,
                &mut frame.faction_members_second,
            );
            frame.factions_result = factions_report.factions.result;
            if needed[4] {
                frame.consistency_checked[4] = true;
                frame.factions_mismatches = factions_report.factions.mismatches;
                matched[4] = full_match(
                    factions_report.factions.result,
                    factions_report.factions.second,
                    factions_report.factions.mismatches,
                );
            }
            frame.table_ns[8] = shared.ns(factions_report.ticks[0] as i64);
            frame.faction_members_result = factions_report.members.result;
            if needed[5] {
                frame.consistency_checked[5] = true;
                frame.faction_members_mismatches = factions_report.members.mismatches;
                matched[5] = full_match(
                    factions_report.members.result,
                    factions_report.members.second,
                    factions_report.members.mismatches,
                );
            }
            frame.table_ns[9] = shared.ns(factions_report.ticks[1] as i64);
            inline_walk_ns = shared.ns(qpc() - inline_walk_begin).saturating_sub(
                shared
                    .acquisition_wait_ns
                    .load(Ordering::Relaxed)
                    .saturating_sub(inline_wait_begin),
            );
            let (air_request, report, air_failure) = completed!(air_pending, AirWing);
            air_wing_report = report;
            let mut air_second = air_request.second;
            if air_wing_report.raw_ok && air_failure.is_none() {
                air_wing_payload = Some(air_request.first);
            } else {
                decode::recycle_air_wing_job(air_request.first);
            }
            let naval_ctx = extended::naval_history::NavalCopyContext {
                base: shared.base,
                game_state: state,
            };
            let mut naval_install_ticks = 0;
            if let Some(wings) = air_wing_payload.as_ref() {
                naval_install_ticks += naval_first.install_wings(wings);
            }
            if let Some(naval) = naval_second.as_mut()
                && let Some(wings) = air_second.as_ref()
                && air_failure.is_none()
            {
                naval_install_ticks += naval.install_wings(wings);
            }
            let naval_pending = queue!(acquisition::Job::Naval(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: naval_ctx,
                first: naval_first,
                second: naval_second,
            }));
            let mut divisions_failure = None;
            if let Some(pending) = divisions_pending {
                let (request, report, failure) = completed!(pending, Divisions);
                divisions_report = report;
                divisions_failure =
                    failure
                        .or((!report.raw_ok)
                            .then_some("division owned acquisition could not complete"));
                let source_started = qpc();
                request
                    .first
                    .supply_sources(&mut frame.extended, division_source_capacity, false);
                if let Some(second) = request.second.as_ref() {
                    second.supply_sources(&mut frame.extended, division_source_capacity, true);
                }
                divisions_report.copy_ticks[0] += elapsed(source_started);
                divisions_payload = Some(request.first);
                divisions_second = request.second;
                frame.extended.set_divisions_raw_mode(true);
                matched[2] = divisions_report.matched[0] && divisions_failure.is_none();
                frame.consistency_checked[2] = needed[2];
                frame.divisions_mismatches = if needed[2] {
                    usize::from(!matched[2])
                } else {
                    0
                };
                let ns = shared.ns(divisions_report.copy_ticks[0] as i64);
                frame.table_ns[6] = ns;
                decode::add_divisions_copy_ns(0, ns);
                decode::add_divisions_raw_bytes(0, divisions_report.raw_bytes[0]);
            }
            let mut characters_failure = None;
            if let Some(pending) = characters_pending {
                let (request, report, failure) = completed!(pending, Characters);
                characters_report = report;
                characters_failure = failure;
                frame.extended.characters_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    characters_payload = Some(request.first);
                } else {
                    decode::recycle_characters_job(request.first);
                }
                if let Some(second) = request.second {
                    decode::recycle_characters_job(second);
                }
            }
            let intel_residual_completed =
                unsafe { finish_acquisition(shared, &mut batch, intel_residual_pending) };
            let acquisition::Job::IntelResidual(intel_residual_request) =
                intel_residual_completed.request
            else {
                unreachable!("IntelResidual ticket type")
            };
            let intel_residual_report = intel_residual_completed.report.map(|report| {
                let acquisition::Report::IntelResidual(report) = report else {
                    unreachable!("IntelResidual report type")
                };
                report
            });
            frame
                .extended
                .restore_intel_residual_request(intel_residual_request, intel_residual_report);
            let mut relations_failure = None;
            if let Some(pending) = relations_pending {
                let (request, report, failure) = completed!(pending, Relations);
                relations_report = report;
                relations_failure = failure;
                frame.extended.relations_pilot().relations_raw_mode =
                    report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    relations_payload = Some(request.first);
                } else {
                    decode::recycle_relations_job(request.first);
                }
                if let Some(second) = request.second {
                    decode::recycle_relations_job(second);
                }
            }
            let (request, report, failure) = completed!(players_pending, Players);
            players_report = report;
            let players_failure = failure;
            frame.map.players_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                players_payload = Some(request.first);
            } else {
                decode::recycle_players_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_players_job(second);
            }
            let (request, report, failure) = completed!(air_stats_pending, AirStats);
            air_stats_report = report;
            let air_stats_failure = failure;
            frame.extended.air_stats_pilot().stats_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                air_stats_payload = Some(request.first);
            } else {
                decode::recycle_air_stats_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_air_stats_job(second);
            }
            let (request, report, failure) = completed!(air_bases_pending, AirBases);
            air_bases_report = report;
            let air_bases_failure = failure;
            frame.extended.air_bases_pilot().bases_raw_mode = report.raw_ok || failure.is_some();
            frame.extended.radar_pilot().registrations_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                air_bases_payload = Some(request.first);
            } else {
                decode::recycle_air_bases_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_air_bases_job(second);
            }
            let (request, report, failure) = completed!(air_history_pending, AirHistory);
            air_history_report = report;
            let air_history_failure = failure;
            frame.extended.air_history_pilot().history_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                air_history_payload = Some(request.first);
            } else {
                decode::recycle_air_history_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_air_history_job(second);
            }
            let (request, report, failure) = completed!(supremacy_pending, Supremacy);
            supremacy_report = report;
            let supremacy_failure = failure;
            frame.extended.supremacy_pilot().supremacy_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                supremacy_payload = Some(request.first);
            } else {
                decode::recycle_supremacy_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_supremacy_job(second);
            }
            let (request, report, failure) = completed!(sunk_pending, Sunk);
            sunk_report = report;
            let sunk_failure = failure;
            frame.extended.sunk_pilot().sunk_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                sunk_payload = Some(request.first);
            } else {
                decode::recycle_sunk_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_sunk_job(second);
            }
            let (request, report, failure) = completed!(radar_pending, Radar);
            radar_report = report;
            let radar_failure = failure;
            frame.extended.radar_pilot().radar_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                radar_payload = Some(request.first);
            } else {
                decode::recycle_radar_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_radar_job(second);
            }
            let (request, report, failure) = completed!(weather_pending, Weather);
            weather_report = report;
            let weather_failure = failure;
            frame.extended.weather_pilot().raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                weather_payload = Some(request.first);
            } else {
                decode::recycle_weather_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_weather_job(second);
            }
            let (request, report, failure) = completed!(capitulation_pending, Capitulation);
            capitulation_report = report;
            let capitulation_failure = failure;
            frame.extended.capitulation_pilot().capitulation_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                capitulation_payload = Some(request.first);
            } else {
                decode::recycle_capitulation_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_capitulation_job(second);
            }
            let mut projects_failure = None;
            if let Some(pending) = projects_pending {
                let (request, report, failure) = completed!(pending, Projects);
                projects_report = report;
                projects_failure = failure;
                frame.extended.projects_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    projects_payload = Some(request.first);
                } else {
                    decode::recycle_projects_job(request.first);
                }
                if let Some(second) = request.second {
                    decode::recycle_projects_job(second);
                }
            }
            let mut lexical_failure = None;
            if let Some(pending) = lexical_pending {
                let (request, report, failure) = completed!(pending, Lexical);
                lexical_report = report;
                lexical_failure = failure;
                frame.extended.lexical_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    lexical_payload = Some(request.first);
                } else {
                    decode::recycle_lexical_job(request.first);
                }
                if let Some(second) = request.second {
                    decode::recycle_lexical_job(second);
                }
            }
            let (request, report, failure) = completed!(session_pending, Session);
            session_report = report;
            let session_failure = failure;
            frame.extended.session_pilot().raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                session_payload = Some(request.first);
            } else {
                decode::recycle_session_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_session_job(second);
            }
            let (request, report, failure) = completed!(intel_pending, Intel);
            intel_report = report;
            let intel_failure = failure;
            frame.extended.intel_pilot().intel_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                intel_payload = Some(request.first);
            } else {
                decode::recycle_intel_job(request.first);
            }
            if let Some(second) = request.second {
                decode::recycle_intel_job(second);
            }
            // Release the shared seed leases before either Census can be
            // cleared on the decode worker during payload retirement.
            drop(request.divisions_first);
            drop(request.divisions_second);
            let (naval_request, report, naval_failure) = completed!(naval_pending, Naval);
            naval_report = report;
            if naval_report.raw_ok {
                naval_report.copy_ticks[0] += naval_install_ticks;
            }
            if let Some(wings) = air_wing_payload.as_ref() {
                naval_report.copy_ticks[0] += wings.membership_ticks;
                if let Some(second) = air_second.as_ref() {
                    naval_report.copy_ticks[0] += second.membership_ticks;
                }
                air_wing_report.copy_ticks[0] += frame.map.install_owned_air(wings);
                if !naval_report.raw_ok && naval_failure.is_none() {
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
            frame.extended.naval_pilot().raw_mode = naval_report.raw_ok || naval_failure.is_some();
            if naval_report.raw_ok && naval_failure.is_none() {
                naval_payload = Some(naval_request.first);
            } else {
                decode::recycle_naval_job(naval_request.first);
            }
            if let Some(second) = naval_request.second {
                decode::recycle_naval_job(second);
            }
            if let Some(second) = air_second.take() {
                decode::recycle_air_wing_job(second);
            }
            if let Some(reason) = air_failure {
                frame.map.fail_air_capture(needed[14], reason);
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
                    air_wing_report.raw_ok || air_failure.is_some(),
                    naval_report.raw_ok || naval_failure.is_some(),
                ),
            );
            // ShipModels depends on the exact first/second native ship
            // census just completed above. Loan those preallocated seeds to
            // the flat pool without walking Ships again or copying model
            // data on this thread.
            let ship_models_pending = queue!(acquisition::Job::ShipModels(
                frame.extended.take_ship_models_request(
                    &country_snapshot,
                    shared.base,
                    &extended_needed,
                )
            ));
            matched[6..PROVINCE_CHECK].copy_from_slice(&map_matched);
            frame.consistency_checked[6..PROVINCE_CHECK]
                .copy_from_slice(&needed[6..PROVINCE_CHECK]);
            for (total, ticks) in frame.table_ns[10..].iter_mut().zip(map_ticks) {
                *total = shared.ns(ticks as i64);
            }
            decode_hour = clock.hour;
            decode_force_full = force_full;
            if let Some(reason) = players_failure {
                frame.map.fail_players_capture(players_ctx.needed, reason);
                matched[6] = false;
                matched[7] = false;
            } else if players_report.raw_ok {
                frame
                    .map
                    .apply_players_report(players_ctx.needed, &players_report);
            }
            let governance_completed =
                unsafe { finish_acquisition(shared, &mut batch, governance_pending) };
            let acquisition::Job::Governance(governance_request) = governance_completed.request
            else {
                unreachable!("Governance ticket type")
            };
            let governance_report = governance_completed.report.map(|report| {
                let acquisition::Report::Governance(report) = report else {
                    unreachable!("Governance report type")
                };
                report
            });
            frame
                .extended
                .restore_governance_request(governance_request, governance_report);
            let presentation_completed =
                unsafe { finish_acquisition(shared, &mut batch, presentation_pending) };
            let acquisition::Job::Presentation(presentation_request) =
                presentation_completed.request
            else {
                unreachable!("Presentation ticket type")
            };
            let presentation_report = presentation_completed.report.map(|report| {
                let acquisition::Report::Presentation(report) = report else {
                    unreachable!("Presentation report type")
                };
                report
            });
            frame
                .extended
                .restore_presentation_request(presentation_request, presentation_report);
            let supply_completed =
                unsafe { finish_acquisition(shared, &mut batch, supply_trains_pending) };
            let acquisition::Job::SupplyTrains(supply_request) = supply_completed.request else {
                unreachable!("SupplyTrains ticket type")
            };
            let supply_report = supply_completed.report.map(|report| {
                let acquisition::Report::SupplyTrains(report) = report else {
                    unreachable!("SupplyTrains report type")
                };
                report
            });
            frame
                .extended
                .restore_supply_trains_request(supply_request, supply_report);
            let portraits_completed =
                unsafe { finish_acquisition(shared, &mut batch, character_portraits_pending) };
            let acquisition::Job::CharacterPortraits(portraits_request) =
                portraits_completed.request
            else {
                unreachable!("CharacterPortraits ticket type")
            };
            let portraits_report = portraits_completed.report.map(|report| {
                let acquisition::Report::CharacterPortraits(report) = report else {
                    unreachable!("CharacterPortraits report type")
                };
                report
            });
            frame
                .extended
                .restore_character_portraits_request(portraits_request, portraits_report);
            let province_order_completed =
                unsafe { finish_acquisition(shared, &mut batch, province_unit_order_pending) };
            let acquisition::Job::ProvinceUnitOrder(province_order_request) =
                province_order_completed.request
            else {
                unreachable!("ProvinceUnitOrder ticket type")
            };
            let province_order_report = province_order_completed.report.map(|report| {
                let acquisition::Report::ProvinceUnitOrder(report) = report else {
                    unreachable!("ProvinceUnitOrder report type")
                };
                report
            });
            frame
                .extended
                .restore_province_unit_order_request(province_order_request, province_order_report);
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
            (checks[visibility], ticks[visibility]) = frame.extended.capture_province_visibility(
                &memory,
                shared.base,
                state,
                &frame.provinces,
                visibility_source,
                extended_needed[visibility],
            );
            if production_rest_report.plan_failed {
                frame
                    .extended
                    .production_rest_pilot()
                    .fail_production_rest_capture(
                        decode::PRODUCTION_REST.map(|table| extended_needed[table.index()]),
                        "production acquisition plan could not complete safely",
                    );
                for (table, cost) in decode::PRODUCTION_REST
                    .into_iter()
                    .zip(production_rest_report.copy_ticks)
                {
                    checks[table.index()] = false;
                    ticks[table.index()] = cost;
                }
            }
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
                    .zip(world_report.production.copy_ticks)
                    .chain(
                        decode::CONSTRUCTION_REST
                            .into_iter()
                            .zip(world_report.construction.copy_ticks),
                    )
                    .chain(
                        decode::ECONOMY_REST
                            .into_iter()
                            .zip(world_report.economy.copy_ticks),
                    )
                    .chain(decode::FOCUS.into_iter().zip(world_report.focus.copy_ticks))
                {
                    checks[table.index()] = false;
                    ticks[table.index()] = cost;
                }
            }
            if characters_refresh {
                if let Some(reason) = characters_failure {
                    frame
                        .extended
                        .characters_pilot()
                        .fail_characters_capture(&extended_needed, reason);
                } else if characters_report.raw_ok {
                    frame
                        .extended
                        .characters_pilot()
                        .apply_characters_report(&extended_needed, &characters_report);
                }
            }
            if relations_refresh {
                if let Some(reason) = relations_failure {
                    frame.extended.relations_pilot().fail_relations_capture(
                        decode::RELATIONS.map(|table| extended_needed[table.index()]),
                        reason,
                    );
                } else if relations_report.raw_ok {
                    frame.extended.relations_pilot().apply_relations_report(
                        decode::RELATIONS.map(|table| extended_needed[table.index()]),
                        &relations_report,
                    );
                }
            }
            if let Some(reason) = air_bases_failure {
                frame
                    .extended
                    .air_bases_pilot()
                    .fail_air_bases_capture(air_bases_ctx.needed, reason);
            } else if air_bases_report.raw_ok {
                frame
                    .extended
                    .air_bases_pilot()
                    .apply_air_bases_report(air_bases_ctx.needed, &air_bases_report);
            }
            if let Some(reason) = air_history_failure {
                frame
                    .extended
                    .air_history_pilot()
                    .fail_air_history_capture(air_history_ctx.needed, reason);
            } else if air_history_report.raw_ok {
                frame
                    .extended
                    .air_history_pilot()
                    .apply_air_history_report(air_history_ctx.needed, &air_history_report);
            }
            if let Some(reason) = supremacy_failure {
                frame
                    .extended
                    .supremacy_pilot()
                    .fail_supremacy_capture(supremacy_ctx.needed, reason);
            } else if supremacy_report.raw_ok {
                frame
                    .extended
                    .supremacy_pilot()
                    .apply_supremacy_report(supremacy_ctx.needed, &supremacy_report);
            }
            if let Some(reason) = sunk_failure {
                frame
                    .extended
                    .sunk_pilot()
                    .fail_sunk_capture(sunk_ctx.needed, reason);
            } else if sunk_report.raw_ok {
                frame
                    .extended
                    .sunk_pilot()
                    .apply_sunk_report(sunk_ctx.needed, &sunk_report);
            }
            if let Some(reason) = radar_failure {
                frame
                    .extended
                    .radar_pilot()
                    .fail_radar_capture(radar_ctx.needed, reason);
            } else if radar_report.raw_ok {
                frame
                    .extended
                    .radar_pilot()
                    .apply_radar_report(radar_ctx.needed, &radar_report);
            }
            if let Some(reason) = intel_failure {
                frame
                    .extended
                    .intel_pilot()
                    .fail_intel_capture(intel_ctx.needed, reason);
            } else if intel_report.raw_ok {
                frame
                    .extended
                    .intel_pilot()
                    .apply_intel_report(intel_ctx.needed, &intel_report);
            }
            if let Some(reason) = capitulation_failure {
                frame
                    .extended
                    .capitulation_pilot()
                    .fail_capitulation_capture(capitulation_ctx.needed, reason);
            } else if capitulation_report.raw_ok {
                frame
                    .extended
                    .capitulation_pilot()
                    .apply_capitulation_report(capitulation_ctx.needed, &capitulation_report);
            }
            if let Some(reason) = weather_failure {
                frame
                    .extended
                    .weather_pilot()
                    .fail_weather_capture(weather_ctx, reason);
            } else if weather_report.raw_ok {
                frame
                    .extended
                    .weather_pilot()
                    .apply_weather_report(weather_ctx, &weather_report);
            }
            if projects_refresh {
                if let Some(reason) = projects_failure {
                    frame
                        .extended
                        .projects_pilot()
                        .fail_projects_capture(projects_ctx, reason);
                } else if projects_report.raw_ok {
                    frame
                        .extended
                        .projects_pilot()
                        .apply_projects_report(projects_ctx, &projects_report);
                }
            }
            if lexical_refresh {
                if let Some(reason) = lexical_failure {
                    frame
                        .extended
                        .lexical_pilot()
                        .fail_lexical_capture(lexical_ctx, reason);
                } else if lexical_report.raw_ok {
                    frame
                        .extended
                        .lexical_pilot()
                        .apply_lexical_report(lexical_ctx, &lexical_report);
                }
            }
            if let Some(reason) = session_failure {
                frame
                    .extended
                    .session_pilot()
                    .fail_session_capture(session_ctx, reason);
            } else if session_report.raw_ok {
                frame
                    .extended
                    .session_pilot()
                    .apply_session_report(session_ctx, &session_report);
            }
            if let Some(reason) = air_bases_failure {
                frame
                    .extended
                    .radar_pilot()
                    .fail_registrations_capture(air_bases_ctx.needed[1], reason);
            } else if air_bases_report.raw_ok {
                frame
                    .extended
                    .radar_pilot()
                    .note_registrations_copy(air_bases_ctx.needed[1], air_bases_report.matched[1]);
            }
            if let Some(reason) = air_failure {
                frame
                    .extended
                    .air_wing_pilot()
                    .fail_air_wing_capture(air_ctx.needed, reason);
            }
            if let Some(reason) = air_stats_failure {
                frame.extended.air_stats_pilot().fail_air_stats_capture(
                    decode::AIR_STATS.map(|table| extended_needed[table.index()]),
                    reason,
                );
            }
            if let Some(reason) = naval_failure {
                frame.extended.naval_pilot().fail_naval_capture(
                    decode::NAVAL_ORDER.map(|table| extended_needed[table.index()]),
                    reason,
                );
            }
            if let Some(reason) = divisions_failure {
                frame.divisions_result = Outcome::Unavailable { reason };
                for pass in 0..=usize::from(division_second_needed) {
                    frame
                        .extended
                        .reject_division_detail_source(pass, &extended_needed, reason);
                }
            }
            // ShipModels can run through every independent pre-ETA callback
            // stage. Join it only at the structural barrier, then restore
            // and perform its fresh modifier terminal verification before
            // any ETA phase or later shared-source verification can begin.
            let ship_models_completed =
                unsafe { finish_acquisition(shared, &mut batch, ship_models_pending) };
            let acquisition::Job::ShipModels(ship_models_request) = ship_models_completed.request
            else {
                unreachable!("ShipModels ticket type")
            };
            let ship_models_report = ship_models_completed.report.map(|report| {
                let acquisition::Report::ShipModels(report) = report else {
                    unreachable!("ShipModels report type")
                };
                report
            });
            frame
                .extended
                .restore_ship_models_request(ship_models_request, ship_models_report);
            let ships = replay::rows::TableId::ShipModels.index();
            (checks[ships], ticks[ships]) = frame.extended.capture_ship_models(
                &cached,
                shared.base,
                state,
                clock.hour,
                extended_needed[ships],
                force_full,
            );
            ts_parallel = qpc();
            // Join wall spent on the non-ETA pool jobs during the parallel phase.
            // acquisition_wait_ns is written only by this thread's joins.
            noneta_join_ns = shared
                .acquisition_wait_ns
                .load(Ordering::Relaxed)
                .saturating_sub(wait_at_setup);
            // A structural verification cannot overlap a still-running acquisition pass.
            // All ranges cross each barrier together, including the independent cold pass.
            let mut eta_requests: [Option<acquisition::EtaRequest>;
                extended::division_eta::MAX_RANGES] = std::array::from_fn(|_| None);
            let mut eta_failed = [false; extended::division_eta::MAX_RANGES];
            let phases = [
                acquisition::EtaPhase::Acquire { pass: 0 },
                acquisition::EtaPhase::Verify { pass: 0 },
                acquisition::EtaPhase::Acquire { pass: 1 },
                acquisition::EtaPhase::Verify { pass: 1 },
            ];
            // Open window: two observations, each acquire+verify (the running
            // proof). Closed: a single acquire; the block is quiescent so the
            // verify re-read would be byte-identical (owner ruling).
            let eta_phase_count = if eta_checked { 4 } else { 1 };
            for (phase_index, phase) in phases.into_iter().take(eta_phase_count).enumerate() {
                if phase_index != 0 {
                    for index in 0..eta_ranges {
                        let mut request = eta_requests[index]
                            .take()
                            .expect("previous ETA phase returned");
                        request.phase = phase;
                        if eta_failed[index] {
                            let (pass, acquisition) = match phase {
                                acquisition::EtaPhase::Acquire { pass } => (pass, true),
                                acquisition::EtaPhase::Verify { pass } => (pass, false),
                            };
                            request.payload.fail(
                                "ETA acquisition did not complete",
                                pass,
                                acquisition,
                            );
                            eta_requests[index] = Some(request);
                        } else {
                            eta_pending[index] = Some(queue!(acquisition::Job::Eta(request)));
                        }
                    }
                }
                for index in 0..eta_ranges {
                    if let Some(pending) = eta_pending[index].take() {
                        let (mut request, _report, failure) = completed!(pending, Eta);
                        if let Some(reason) = failure {
                            let (pass, acquisition) = match phase {
                                acquisition::EtaPhase::Acquire { pass } => (pass, true),
                                acquisition::EtaPhase::Verify { pass } => (pass, false),
                            };
                            request.payload.fail(reason, pass, acquisition);
                            eta_failed[index] = true;
                        }
                        eta_requests[index] = Some(request);
                    }
                }
            }
            for (index, request) in eta_requests.into_iter().enumerate().take(eta_ranges) {
                let request = request.expect("all ETA ranges returned");
                frame.extended.restore_eta_range(index, request.payload);
                decode::recycle_eta_workspace(request.workspace);
            }
            if eta_ranges != 0 {
                frame.extended.finish_eta_ranges();
            }
            acquisition.observe_eta_diagnostics(frame.extended.eta_diagnostics());
            let eta = replay::rows::TableId::DivisionEta.index();
            (checks[eta], ticks[eta]) =
                frame
                    .extended
                    .capture_eta(&cached, shared.base, state, extended_needed[eta]);
            ts_eta = qpc();
            // Explicitly drain the flat lease before shared-field or country verification.
            drop(batch);
            drop(unit_first);
            drop(unit_second);
            let unit_verify_started = qpc();
            for pass in 0..=usize::from(division_second_needed) {
                let census = if divisions_owned {
                    if pass == 0 {
                        &mut divisions_payload
                            .as_mut()
                            .expect("owned division packet")
                            .census
                    } else {
                        &mut divisions_second
                            .as_mut()
                            .expect("owned division check packet")
                            .census
                    }
                } else {
                    &mut frame.division_fallback[pass]
                };
                let bytes_before = census.payload_bytes();
                let consumed = frame.extended.consumed_unit_seed_masks(pass);
                let changed = census.verify_unit_inputs(&cached, &consumed);
                let valid = frame.extended.reject_changed_unit_inputs(pass, changed);
                for (table, valid) in [
                    replay::rows::TableId::DivisionMovement,
                    replay::rows::TableId::DivisionStatus,
                    replay::rows::TableId::DivisionEta,
                ]
                .into_iter()
                .zip(valid)
                {
                    checks[table.index()] &= valid;
                }
                let core = census.outcome(frame.divisions.len());
                if needed[2] && !matches!(core, Outcome::Rows { .. }) {
                    matched[2] = false;
                    divisions_report.matched[0] = false;
                    divisions_report.mismatches[0] = 1;
                    frame.divisions_mismatches = 1;
                }
                if !divisions_owned && pass == 0 {
                    frame.divisions_result = core;
                }
                if let Outcome::Unavailable { reason } = census.outcome(division_source_capacity) {
                    frame
                        .extended
                        .reject_division_detail_source(pass, &extended_needed, reason);
                    for (slot, table) in decode::DIVISIONS.iter().enumerate().skip(1) {
                        if pass == 0 || extended_needed[table.index()] {
                            checks[table.index()] = false;
                            divisions_report.matched[slot] = false;
                            divisions_report.mismatches[slot] = 1;
                        }
                    }
                }
                if divisions_report.raw_ok && pass == 0 {
                    decode::add_divisions_raw_bytes(
                        0,
                        census.payload_bytes().saturating_sub(bytes_before),
                    );
                }
            }
            // Drop every worker/frame lease before a census can be recycled
            // or cleared. Deferred projection owns its operands separately.
            frame.extended.release_unit_inputs();
            if let Some(second) = divisions_second.take() {
                decode::recycle_divisions_job(second);
            }
            let unit_verify_ns = shared.ns(qpc() - unit_verify_started);
            frame.table_ns[6] += unit_verify_ns;
            if divisions_report.raw_ok {
                decode::add_divisions_copy_ns(0, unit_verify_ns);
            }
            if divisions_failure.is_some() {
                if let Some(job) = divisions_payload.take() {
                    decode::recycle_divisions_job(job);
                }
                divisions_report.raw_ok = false;
            }
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
                        extended_needed[index] && divisions_report.mismatches[slot] != 0;
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
                    // Governance retains the shared country traversal on this anchor.
                    frame.table_ns[index] = frame.table_ns[index].saturating_add(ns);
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
                    decode::add_economy_rest_raw_bytes(slot, economy_rest_report.raw_bytes[slot]);
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
                        extended_needed[index] && radar_report.mismatches[slot] != 0;
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
                        extended_needed[index] && intel_report.mismatches[slot] != 0;
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
                    decode::add_capitulation_raw_bytes(slot, capitulation_report.raw_bytes[slot]);
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
                frame.order_counters.elapsed_ns = shared.ns(frame.order_counters.qpc_end - begin);
            }
            let cache_verify_begin = qpc();
            frame.country_cache_error = cached.verify().err();
            frame.country_cache_ns = shared.ns(cache_prepare_ticks + qpc() - cache_verify_begin);
            // Authentication is shared by country-dependent tables. This
            // accounting slot includes the seed and final direct check once.
            frame.table_ns[2] += frame.country_cache_ns;
            if frame.country_cache_error.is_some() {
                matched.fill(false);
                extended_matched.fill(false);
                wars_matrix_check = false;
            }
            if force_full {
                // Diagnostic only: the original replay witness is the entry
                // sample. This second sample follows acquisition and all
                // terminal dependency checks, before the tick releases.
                let begin = qpc();
                frame.order_counters.terminal_qpc_begin = begin;
                frame.order_counters.terminal_counters = Some(
                    save_allocation_counters::read_counters(&memory, shared.base, state),
                );
                let end = qpc();
                frame.order_counters.terminal_qpc_end = end;
                frame.order_counters.terminal_elapsed_ns = shared.ns(end - begin);
                clock_ticks += end - begin;
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
        // The frame's cost is this acquisition body's wall on the game thread.
        let game_elapsed = (qpc() - dispatched).max(1);
        // Disjoint block phases: setup+snapshot, parallel (dispatch/inline/joins),
        // the serial ETA barriers, then residual verification and enqueue.
        shared
            .phase_setup_ns
            .fetch_add(shared.ns(ts_setup - dispatched), Ordering::Relaxed);
        shared
            .phase_parallel_ns
            .fetch_add(shared.ns(ts_parallel - ts_setup), Ordering::Relaxed);
        shared
            .phase_eta_ns
            .fetch_add(shared.ns(ts_eta - ts_parallel), Ordering::Relaxed);
        shared.phase_residual_ns.fetch_add(
            shared.ns(dispatched + game_elapsed - ts_eta),
            Ordering::Relaxed,
        );
        shared
            .phase_inline_walk_ns
            .fetch_add(inline_walk_ns, Ordering::Relaxed);
        shared
            .phase_noneta_join_ns
            .fetch_add(noneta_join_ns, Ordering::Relaxed);
        shared.phase_calls.fetch_add(1, Ordering::Relaxed);
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

struct PendingAcquisition {
    ticket: Option<acquisition::Ticket>,
    retained: Option<acquisition::Job>,
}

/// Caller retains the game boundary until this guard and all returned jobs finish.
unsafe fn submit_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    request: acquisition::Job,
) -> PendingAcquisition {
    let begin = qpc();
    let mut retained = Some(request);
    // SAFETY: the caller's batch owns this callback's complete live lease.
    let ticket = unsafe { batch.submit(&mut retained) }.ok();
    shared
        .acquisition_dispatch_ns
        .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
    PendingAcquisition { ticket, retained }
}

/// Accepted work is recovered once; only rejected submissions may execute inline.
unsafe fn finish_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    mut pending: PendingAcquisition,
) -> acquisition::Completion<acquisition::Job, acquisition::Report> {
    if let Some(ticket) = pending.ticket {
        let begin = qpc();
        let result = batch.finish(ticket).expect("submitted acquisition job");
        shared
            .acquisition_wait_ns
            .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
        result
    } else {
        // SAFETY: this request was rejected before acquiring any live state.
        unsafe { batch.run_inline(pending.retained.take().expect("rejected request retained")) }
    }
}

fn acquisition_failure_reason(failure: acquisition::Failure) -> &'static str {
    match failure {
        acquisition::Failure::Panicked => "acquisition worker panicked",
        acquisition::Failure::Stopped => "acquisition worker stopped before completion",
    }
}

fn acquire_world_request(
    countries: &Arc<Cache>,
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
        countries: Arc::clone(countries),
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

/// One independently bounded owned division census and its five detail tables.
/// No address/span replay or live memory access remains on the decode worker.
#[derive(Debug, Default)]
pub(crate) struct DivisionsJob {
    pub(crate) census: crate::division_census::Census,
    details: crate::division_detail::OwnedDetails,
    context: DivisionsCopyContext,
    acquisition_ticks: [u64; 6],
    inputs_captured: bool,
    core_finished: bool,
    overflowed: bool,
    capacity_charge: crate::raw_memory::Charge,
}
impl DivisionsJob {
    pub fn new() -> Self {
        let mut job = Self {
            census: crate::division_census::Census::new(),
            details: crate::division_detail::OwnedDetails::new(),
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }
    pub fn clear(&mut self) {
        self.census.clear();
        self.details.clear();
        self.overflowed = false;
        self.acquisition_ticks = [0; 6];
        self.inputs_captured = false;
        self.core_finished = false;
    }
    pub fn initialized(&self) -> bool {
        self.census.initialized()
    }
    pub fn grow_for_next(&self) -> bool {
        self.overflowed || self.census.overflowed
    }
    pub fn grow(&mut self) {
        self.census.grow();
        self.details.grow();
        self.update_capacity_charge();
    }
    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: self.census.allocated_bytes()
                + self.details.allocated_bytes() as u64,
            ..Default::default()
        });
    }
    pub fn payload_bytes(&self) -> [u64; 6] {
        let mut bytes = [0; 6];
        bytes[0] = self.census.payload_bytes();
        bytes[1..].copy_from_slice(&self.details.acquired_bytes());
        bytes
    }
    #[cfg(test)]
    fn source_outcome(&self, capacity: usize) -> Outcome {
        self.census.outcome(capacity)
    }
    fn supply_sources(&self, captured: &mut extended::Captured, capacity: usize, second: bool) {
        supply_division_census_sources(&self.census, captured, capacity, second);
    }
}

fn supply_division_census_sources(
    census: &crate::division_census::Census,
    captured: &mut extended::Captured,
    capacity: usize,
    second: bool,
) {
    let (sources, outcome) = census.sources(capacity);
    for &source in sources {
        captured.visit_division_source(source, second);
    }
    captured.finish_divisions(outcome, second);
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DivisionsCopyReport {
    /// Ready owned payload. A core failure after seed leasing is terminal;
    /// only an input-stage shortage before leasing may use a fresh fallback.
    pub raw_ok: bool,
    pub matched: [bool; 6],
    pub mismatches: [usize; 6],
    pub copy_ticks: [u64; 6],
    pub raw_bytes: [u64; 6],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DivisionsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub row_capacity: usize,
    pub source_capacity: usize,
    pub detail_capacities: [usize; 5],
}
impl Default for DivisionsCopyContext {
    fn default() -> Self {
        let c = extended::production::Capacities::default().rows;
        Self {
            base: 0,
            game_state: 0,
            row_capacity: INITIAL_CAPACITIES[4],
            source_capacity: 8192,
            detail_capacities: [c[2], c[7], c[8], c[9], c[10]],
        }
    }
}
/// Initial source acquisition only. No input lease may outlive an earlier use
/// of this job. A native failure remains attached to this exact input attempt.
fn copy_divisions_inputs_pass(
    job: &mut DivisionsJob,
    memory: &impl engine::Memory,
    context: DivisionsCopyContext,
) {
    job.clear();
    job.context = context;
    let started = qpc();
    job.census
        .capture_inputs(memory, context.base, context.game_state);
    job.acquisition_ticks[0] = elapsed(started);
    job.inputs_captured = true;
    // This is metadata-only capacity admission. capture_core may still discover
    // a private row/header capacity shortage, which cannot authorize a retry
    // after consumers receive a lease.
    job.overflowed = !job.details.ready_for(context.detail_capacities) || job.census.overflowed;
}

/// Acquire and retain the two independent seed packets before any core task.
/// False permits a fresh fallback only while neither packet has been leased.
pub(crate) fn copy_divisions_inputs_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    first: &mut DivisionsJob,
    second: Option<&mut DivisionsJob>,
) -> bool {
    copy_divisions_inputs_pass(first, memory, ctx);
    let mut ready = !first.overflowed;
    if let Some(second) = second {
        copy_divisions_inputs_pass(second, memory, ctx);
        ready &= !second.overflowed;
    }
    ready
}

/// Consume only the retained input lineage. This stage never clears a Census,
/// reacquires its input packet, or retries an earlier core/detail acquisition.
fn copy_divisions_core_pass(
    job: &mut DivisionsJob,
    memory: &impl engine::Memory,
    context: DivisionsCopyContext,
) {
    // Context mismatch is a terminal orchestration failure. Keep the original
    // input owner intact so its other consumers can finish and be verified.
    if !job.inputs_captured || job.context != context {
        job.overflowed = true;
        job.core_finished = true;
        return;
    }
    if job.core_finished {
        return;
    }
    job.core_finished = true;
    let started = qpc();
    job.census
        .capture_core(memory, context.row_capacity.max(context.source_capacity));
    job.acquisition_ticks[0] += elapsed(started);
    job.overflowed = !job.details.ready_for(context.detail_capacities) || job.census.overflowed;
    if job.overflowed {
        return;
    }
    let (sources, outcome) = job.census.sources(context.source_capacity);
    let detail_ticks = job.details.capture(
        memory,
        context.base,
        sources,
        outcome,
        context.detail_capacities,
    );
    for (total, extra) in job.acquisition_ticks[1..].iter_mut().zip(detail_ticks) {
        *total += extra;
    }
}

/// Compare only the owned results; no source read or input reauthentication.
fn divisions_copy_report(
    ctx: DivisionsCopyContext,
    first: &DivisionsJob,
    second: Option<&DivisionsJob>,
) -> DivisionsCopyReport {
    let mut ticks = first.acquisition_ticks;
    let mut matched = [true; 6];
    let mut mismatches = [0; 6];
    let mut raw_ok = !first.overflowed;
    if let Some(second) = second {
        for (total, extra) in ticks.iter_mut().zip(second.acquisition_ticks) {
            *total += extra;
        }
        raw_ok &= !second.overflowed;
        matched[0] = raw_ok && first.census.equal(&second.census, ctx.row_capacity);
        mismatches[0] = usize::from(!first.census.same(&second.census, ctx.row_capacity));
        let source_match = first.census.equal(&second.census, ctx.source_capacity);
        let source_same = first.census.same(&second.census, ctx.source_capacity);
        mismatches[1..].copy_from_slice(&first.details.differences(&second.details));
        for mismatch in &mut mismatches[1..] {
            *mismatch += usize::from(!source_same);
        }
        for (slot, equal) in matched[1..]
            .iter_mut()
            .zip(first.details.equal(&second.details))
        {
            *slot = raw_ok && source_match && equal;
        }
    }
    DivisionsCopyReport {
        raw_ok,
        matched,
        mismatches,
        copy_ticks: ticks,
        raw_bytes: first.payload_bytes(),
    }
}

/// Finish already-seeded jobs. Once their inputs have been leased, raw_ok=false
/// is a terminal result, never authorization for the composed/fallback reader.
pub(crate) fn copy_divisions_core_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    first: &mut DivisionsJob,
    mut second: Option<&mut DivisionsJob>,
) -> DivisionsCopyReport {
    copy_divisions_core_pass(first, memory, ctx);
    if let Some(second) = second.as_deref_mut() {
        copy_divisions_core_pass(second, memory, ctx);
    }
    divisions_copy_report(ctx, first, second.as_deref())
}

/// Original composed path for synthetic callers and pre-lease fallback. Keep
/// every first-pass observation before the second input census is acquired.
#[cfg(test)]
pub(crate) fn copy_divisions_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    first: &mut DivisionsJob,
    mut second: Option<&mut DivisionsJob>,
) -> DivisionsCopyReport {
    copy_divisions_inputs_pass(first, memory, ctx);
    copy_divisions_core_pass(first, memory, ctx);
    if let Some(second) = second.as_deref_mut() {
        copy_divisions_inputs_pass(second, memory, ctx);
        copy_divisions_core_pass(second, memory, ctx);
    }
    divisions_copy_report(ctx, first, second.as_deref())
}

/// Worker installation copies compact core rows and transfers owned detail
/// buffers. Callback presentation rows and its independent source passes survive.
pub(crate) fn decode_divisions_payload(
    frame: &mut Captured,
    _base: usize,
    job: &mut DivisionsJob,
    decode_ticks: &mut [u64; 6],
) {
    let started = qpc();
    frame.divisions_result = job.census.install_rows(&mut frame.divisions);
    decode_ticks[0] += elapsed(started);
    let started = qpc();
    frame.extended.install_division_detail(&mut job.details);
    job.update_capacity_charge();
    decode_ticks[1] += elapsed(started);
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

    /// Owned capture: independent passes compared, then the first payload
    /// installed into a pooled frame with the same buffer lengths.
    pub(crate) fn split(
        memory: &impl Memory,
        lens: [usize; 6],
    ) -> (Split, DivisionsCopyReport, DivisionsJob) {
        let ctx = DivisionsCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            row_capacity: lens[0],
            source_capacity: lens[0],
            detail_capacities: lens[1..].try_into().unwrap(),
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = super::copy_divisions_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut frame = Captured::new();
        frame.divisions = vec![carmy::Row::default(); lens[0]];
        frame.extended.test_resize_division_buffers(lens);
        let mut ticks = [0u64; 6];
        super::decode_divisions_payload(&mut frame, BASE, &mut first, &mut ticks);
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
        use crate::raw_memory::{Capacity, Charge, Tracker};
        static LOCAL: Tracker = Tracker::new();
        let mut job = DivisionsJob {
            capacity_charge: Charge::empty_on(&LOCAL),
            ..Default::default()
        };
        job.update_capacity_charge();
        let expected = |job: &DivisionsJob| {
            job.census.allocated_bytes() + job.details.allocated_bytes() as u64
        };
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
                    assert_eq!(frame.divisions.len(), capacities[4]);
                    assert_eq!(frame.faction_names.len(), capacities[8]);
                    assert_eq!(frame.consistency_successful, successful);
                    let second = comparison_capacities(frame);
                    assert_eq!(second, [0; 7]);
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

    /// At the end-of-hour boundary a dropped callback reads both clocks before
    /// the original tick, so a lost first hour after a load is the loaded hour.
    #[test]
    fn end_of_hour_losses_record_the_entry_hour_as_the_lost_and_loaded_hour() {
        assert_eq!(SAMPLE_BOUNDARY, replay::identity::SampleBoundary::EndOfHour);
        let mut batch = LossBatch::default();
        batch.push(Some(clock(20, 3)), Some(clock(20, 3)));
        batch.push(Some(clock(21, 3)), Some(clock(21, 3)));
        let runs: Vec<_> = batch.iter().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].loaded_hour),
            (20, 21, Some(20))
        );
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
            0, // Core division comparison rows now live in the owned census.
            frame.wars_second.capacity(),
            frame.factions_second.capacity(),
            frame.faction_members_second.capacity(),
            frame.faction_names_second.capacity(),
        ]
    }

    #[test]
    fn fallback_census_growth_consumes_the_failed_attempt_once() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let mut frame = Captured::new();
        frame.division_fallback[0] = crate::division_census::Census::default();
        frame.division_fallback[0].capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert!(frame.division_fallback[0].overflowed);
        let mut capacities = INITIAL_CAPACITIES;
        frame.grow(&mut capacities);
        assert!(frame.division_fallback[0].initialized());
        assert!(!frame.division_fallback[0].overflowed);
        let grown = frame.division_fallback[0].allocated_bytes();
        assert!(grown > 0);

        // A subsequent frame that never used the fallback carries no new
        // overflow request and must retain, rather than double, its storage.
        frame.grow(&mut capacities);
        assert_eq!(frame.division_fallback[0].allocated_bytes(), grown);

        frame.division_fallback[0].capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(
            frame.division_fallback[0].outcome(64),
            Outcome::Rows { len: 3 },
        );
        let lease = Arc::clone(frame.division_fallback[0].unit_inputs().unwrap());
        frame
            .extended
            .install_unit_inputs(Some(Arc::clone(&lease)), None, true);
        assert_eq!(Arc::strong_count(&lease), 3);
        frame.extended.release_unit_inputs();
        assert_eq!(Arc::strong_count(&lease), 2);
        drop(lease);
        frame.grow(&mut capacities);
        assert_eq!(frame.division_fallback[0].allocated_bytes(), grown);
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
            if index == 2 {
                // Fallback census storage is retained independently of the core window.
                assert_eq!(frame.allocated_bytes(), before);
            } else {
                assert!(frame.allocated_bytes() < before);
            }
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
        assert_eq!(pending.divisions.len(), 8192);
        assert_eq!(pending.wars_second.len(), 2048);
        assert_eq!(pending.factions_second.len(), 128);
        assert_eq!(pending.faction_names_second.len(), 32768);
        assert!(
            comparison_capacities(&pending)
                .into_iter()
                .enumerate()
                .all(|(index, capacity)| index == 2 || capacity != 0)
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
    fn wars_acquisition_restores_storage_and_denies_failed_completion_credit() {
        use crate::acquisition::Failure;
        use wars_test_kit as kit;
        for failure in [None, Some(Failure::Panicked), Some(Failure::Stopped)] {
            let bytes = kit::fixture();
            let memory = kit::memory(&bytes);
            let mut owner = Arc::new(Cache::with_capacity(1024));
            let countries =
                Cache::prepare_shared(&mut owner, &memory, kit::BASE, kit::GAME_STATE).unwrap();
            let mut frame = Captured::new();
            let expected_bytes = frame.allocated_bytes();
            let pointers = [
                frame.wars.as_ptr(),
                frame.wars_second.as_ptr(),
                frame.wars_matrix.as_ptr(),
            ];
            let snapshot_capacity = frame.wars_snapshot.capacity();
            let mut request =
                frame.take_wars_request(&countries, kit::BASE, kit::GAME_STATE, true, true);
            assert!(frame.wars.is_empty());
            assert!(frame.wars_second.is_empty());
            assert!(frame.wars_matrix.is_empty());
            assert_eq!(
                frame.wars_snapshot.capacity(),
                cwar_relation::OwnedSnapshotCapacity::default()
            );
            let row_bytes = (request.table.buffers.first.capacity()
                + request.table.buffers.second.capacity()
                + request.matrix.capacity())
                * size_of::<cwar_relation::Row>();
            assert_eq!(
                frame.allocated_bytes() + snapshot_capacity.allocated_bytes + row_bytes,
                expected_bytes
            );
            let report = if failure == Some(Failure::Stopped) {
                acquisition_wars::Report::default()
            } else {
                request.run(&memory)
            };
            // A completion failure overrides even fully populated, matching
            // buffers. They are recovered, but cannot publish a valid report.
            let report = frame.restore_wars_request(
                request,
                report,
                failure.map(acquisition_failure_reason),
            );
            assert_eq!(
                [
                    frame.wars.as_ptr(),
                    frame.wars_second.as_ptr(),
                    frame.wars_matrix.as_ptr()
                ],
                pointers
            );
            assert_eq!(frame.wars_snapshot.capacity(), snapshot_capacity);
            assert_eq!(frame.allocated_bytes(), expected_bytes);
            assert_eq!(
                frame.wars_snapshot_sampled,
                failure != Some(Failure::Stopped)
            );
            assert!(frame.consistency_checked[3]);
            assert_eq!(Arc::strong_count(&countries), 2);
            drop(countries);
            assert!(Arc::get_mut(&mut owner).is_some());
            let mut window = ConsistencyWindow::<17>::default();
            let mut matched = [false; 17];
            matched[3] = report.table_matched();
            assert_eq!(
                window.commit(matched, true, true)[3],
                u16::from(failure.is_none())
            );
            let matrix_remaining = AtomicUsize::new(1);
            if report.matrix_eligible() {
                complete_matrix_check(&matrix_remaining);
            }
            assert_eq!(
                matrix_remaining.load(Ordering::Relaxed),
                usize::from(failure.is_some())
            );
            if let Some(failure) = failure {
                let unavailable = Outcome::Unavailable {
                    reason: acquisition_failure_reason(failure),
                };
                assert_eq!(frame.wars_result, unavailable);
                assert_eq!(frame.war_matrix_result, Some(unavailable));
                assert_eq!(report.table_ticks, 0);
                assert_eq!(report.matrix_ticks, 0);
            }
        }
    }

    #[test]
    fn wars_acquisition_uses_empty_default_and_preserves_closed_outputs() {
        use wars_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let mut cache = Cache::with_capacity(1024);
        let _ = cache.prepare(&memory, kit::BASE, kit::GAME_STATE);
        let countries = Arc::new(cache);
        let mut frame = Captured::new();
        let bytes = frame.allocated_bytes();
        for (needed, matrix_needed) in [(true, true), (false, true), (true, false), (false, false)]
        {
            let second = frame.wars_second.clone();
            let matrix = frame.wars_matrix.clone();
            let mut request = frame.take_wars_request(
                &countries,
                kit::BASE,
                kit::GAME_STATE,
                needed,
                matrix_needed,
            );
            assert_eq!(
                frame.wars_snapshot.capacity(),
                cwar_relation::OwnedSnapshotCapacity::default()
            );
            let report = request.run(&memory);
            let report = frame.restore_wars_request(request, report, None);
            assert_eq!(frame.consistency_checked[3], needed);
            assert_eq!(frame.wars_mismatches, 0);
            assert_eq!(frame.war_matrix_result.is_some(), matrix_needed);
            assert_eq!(report.table_matched(), needed);
            assert_eq!(report.matrix_eligible(), matrix_needed);
            assert_eq!(frame.allocated_bytes(), bytes);
            if !needed {
                assert_eq!(frame.wars_second, second);
            }
            if !matrix_needed {
                assert_eq!(frame.wars_matrix, matrix);
            }
        }
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
        assert_eq!(report.matched, [false; 6]);
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
        // (fault address, expected independent table verdicts in group order).
        // Outcomes including reason strings must match the typed walks
        // exactly; only equal complete tables can advance their cold windows.
        // Shared census failures block every dependent table.
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
            (group + 0x300, [true, true, true, true, false, false]),
            (0x64000, [true, true, true, true, false, false]),
            (
                0x64000 + cupgrade_delivery::STATUS,
                // The native delivery header independently authenticates its
                // produced pool before attempting this optional status read.
                [true, true, true, true, false, true],
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
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        // Same-hour tearing between independent passes changes the owned rows.
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
        assert!(!after.census.equal(&after_torn.census, ctx.row_capacity));
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
    fn divisions_owned_success_supplies_both_live_companions_before_decode_without_erasing_them() {
        use super::divisions_test_kit as kit;
        use replay::frame::TableView;
        use replay::rows::{TableId, TableRows};
        let memory = divisions_presentation_memory();
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(first.source_outcome(64), Outcome::Rows { len: 3 });
        let (first_sources, _) = first.census.sources(64);
        assert_eq!(first_sources, second.census.sources(64).0);
        assert_eq!(first_sources[0].division, 101);
        assert_eq!(first_sources[0].template, 0x3455_0000_0034);
        assert_eq!(first_sources[0].country, 1);
        let mut frame = Captured::new();
        first.supply_sources(&mut frame.extended, frame.divisions.len(), false);
        second.supply_sources(&mut frame.extended, frame.divisions.len(), true);
        frame.extended.install_unit_inputs(
            first.census.unit_inputs().map(Arc::clone),
            second.census.unit_inputs().map(Arc::clone),
            first.census.unit_inputs_equal(&second.census),
        );
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
        for (pass, job) in [&mut first, &mut second].into_iter().enumerate() {
            let consumed = frame.extended.consumed_unit_seed_masks(pass);
            let changed = job.census.verify_unit_inputs(&memory, &consumed);
            assert_eq!(
                frame.extended.reject_changed_unit_inputs(pass, changed),
                [true; 3]
            );
        }
        frame.extended.release_unit_inputs();
        frame.extended.decode_division_presentation_inputs();
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
        // The actual worker installation preserves acquired live tables and
        // installs the independently captured native detail rows.
        frame.extended.set_divisions_raw_mode(true);
        decode_divisions_payload(&mut frame, 0, &mut first, &mut [0; 6]);
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
            ..Default::default()
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
        // The pointer census is empty and complete for independent consumers;
        // core division sources still refuse the missing-country world above.
        assert_eq!(first.census.census_outcome(), Outcome::Rows { len: 0 });
        let mut bytes = kit::fixture();
        for country in [kit::C0, kit::C1] {
            bytes[country + ccountry::DIVISIONS..country + ccountry::DIVISIONS + 16].fill(0);
        }
        copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
        assert_eq!(first.source_outcome(0), Outcome::Rows { len: 0 });
        assert!(first.census.sources(64).0.is_empty()); // Recycled rows were cleared.
        first.census = crate::division_census::Census::default();
        let bytes = kit::fixture();
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first, None);
        assert!(!report.raw_ok && first.census.overflowed);
        assert!(matches!(
            first.source_outcome(64),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn divisions_owned_census_survives_a_private_core_field_fault() {
        use super::divisions_test_kit as kit;
        struct Fault<'a>(&'a [u8]);
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let fault = kit::A1 + carmy::STRENGTH;
                if address <= fault && address.saturating_add(output.len()) > fault {
                    return false;
                }
                kit::memory(self.0).copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let mut census = crate::division_census::Census::new();
        census.capture(&Fault(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(census.census_outcome(), Outcome::Rows { len: 3 });
        assert_eq!(
            census
                .entries()
                .iter()
                .map(|entry| entry.pointer)
                .collect::<Vec<_>>(),
            [kit::A1, kit::A2, kit::A3]
        );
        assert_eq!(
            census.rows(64).1,
            Outcome::Unavailable {
                reason: "division hit points unreadable"
            }
        );
        assert!(!census.equal(&census, 64));
    }

    #[test]
    fn divisions_shared_empty_country_census_preserves_independent_fog_success() {
        use super::divisions_test_kit as kit;
        let mut bytes = kit::fixture();
        for address in [kit::CTAB + 8, kit::CTAB + 16, kit::CTAB + 24] {
            bytes[address..address + 8].fill(0);
        }
        let memory = kit::memory(&bytes);
        let mut census = crate::division_census::Census::new();
        census.capture(&memory, kit::BASE, kit::GAME_STATE, 64);
        let direct = engine::unit_viewer_entry::walk(&memory, kit::BASE, kit::GAME_STATE, &mut []);
        assert_eq!(direct, Outcome::Rows { len: 0 });
        let shared = engine::unit_viewer_entry::walk_with_divisions(
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            &mut [],
            Some(census.intel_sources()),
        );
        assert_eq!(shared, direct);
        assert!(census.intel_equal(&census));
        assert_eq!(
            census.rows(0).1,
            Outcome::Unavailable {
                reason: "country table has no readable countries"
            }
        );
    }

    #[test]
    fn divisions_owned_batch_does_not_retry_a_failed_field_into_success() {
        use super::divisions_test_kit as kit;
        struct OnceFault<'a> {
            bytes: &'a [u8],
            pending: std::cell::Cell<bool>,
        }
        impl engine::Memory for OnceFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let fault = kit::A1 + carmy::STRENGTH;
                if address <= fault
                    && address.saturating_add(output.len()) > fault
                    && self.pending.replace(false)
                {
                    return false;
                }
                kit::memory(self.bytes).copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = OnceFault {
            bytes: &bytes,
            pending: std::cell::Cell::new(true),
        };
        let direct = kit::direct(&memory, [64; 6]);
        assert_eq!(
            direct.divisions_outcome,
            Outcome::Unavailable {
                reason: "division hit points unreadable"
            }
        );
        memory.pending.set(true);
        let mut census = crate::division_census::Census::new();
        census.capture(&memory, kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(census.rows(64).1, direct.divisions_outcome);
        assert!(!census.equal(&census, 64));
    }

    #[test]
    fn divisions_owned_consistency_rejects_replaced_source_with_equal_rows() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let mut replaced = bytes.clone();
        let pointer = 0x66000_usize;
        replaced[pointer..pointer + 0x1000].copy_from_slice(&bytes[kit::A1..kit::A1 + 0x1000]);
        replaced[0x23000..0x23008].copy_from_slice(&(pointer as u64).to_le_bytes());
        let mut first = crate::division_census::Census::new();
        let mut second = crate::division_census::Census::new();
        first.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        second.capture(&kit::memory(&replaced), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(first.rows(64), second.rows(64));
        assert_eq!(first.rows(64).1, Outcome::Rows { len: 3 });
        assert_ne!(first.entries(), second.entries());
        assert!(!first.equal(&second, 64));
    }

    #[test]
    fn divisions_live_companion_checks_do_not_accept_missing_or_failed_second_sources() {
        use super::divisions_test_kit as kit;
        use replay::rows::TableId;
        let memory = divisions_presentation_memory();
        let ctx = DivisionsCopyContext {
            base: 0,
            game_state: kit::GAME_STATE,
            ..Default::default()
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
                    frame.divisions.len(),
                    true,
                );
            }
            frame.extended.install_unit_inputs(
                first.census.unit_inputs().map(Arc::clone),
                failed.then(|| {
                    Arc::new(crate::division_census::UnitInputs::fixture(
                        Vec::new(),
                        Outcome::Unavailable {
                            reason: "synthetic failed second census",
                        },
                    ))
                }),
                false,
            );
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
        // A retired parent window still has a second owned job when only a new
        // companion needs its independent check; no core check is fabricated.
        for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
            let mut child_only = [false; TABLE_COUNT];
            child_only[table.index()] = true;
            assert!(extended::Captured::needs_division_check(&child_only));
            let mut second = DivisionsJob::new();
            copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
            let mut frame = Captured::new();
            first.supply_sources(&mut frame.extended, frame.divisions.len(), false);
            second.supply_sources(&mut frame.extended, frame.divisions.len(), true);
            frame.extended.install_unit_inputs(
                first.census.unit_inputs().map(Arc::clone),
                second.census.unit_inputs().map(Arc::clone),
                first.census.unit_inputs_equal(&second.census),
            );
            let (matched, _) =
                frame
                    .extended
                    .capture(&memory, 0, kit::GAME_STATE, 0, &child_only, false);
            assert!(matched[table.index()]);
            frame.extended.release_unit_inputs();
        }
    }

    #[test]
    fn divisions_owned_comparison_isolates_equipment_private_values() {
        use super::divisions_test_kit as kit;
        // Change one bank amount while keeping its shared variant identity and
        // the entire division census stable. Only equipment must differ.
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        let mut changed = bytes.clone();
        changed[0x42008] ^= 0xff;
        copy_divisions_refresh(&kit::memory(&changed), ctx, &mut second, None);
        assert!(first.census.equal(&second.census, ctx.row_capacity));
        assert!(first.census.equal(&second.census, ctx.source_capacity));
        assert_eq!(
            first.details.equal(&second.details),
            [false, true, true, true, true]
        );
    }

    #[test]
    fn divisions_seeded_worker_preserves_owned_results_and_does_not_repeat_core() {
        use super::divisions_test_kit as kit;
        struct NoRead;
        impl engine::Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("completed core must not read again");
            }
        }
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        assert!(copy_divisions_inputs_refresh(
            &memory,
            context,
            &mut first,
            Some(&mut second)
        ));
        let lease = Arc::clone(first.census.unit_inputs().unwrap());
        let report = copy_divisions_core_refresh(&memory, context, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert!(Arc::ptr_eq(&lease, first.census.unit_inputs().unwrap()));
        let repeated = copy_divisions_core_refresh(&NoRead, context, &mut first, Some(&mut second));
        assert_eq!(repeated.raw_bytes, report.raw_bytes);
        assert_eq!(repeated.copy_ticks, report.copy_ticks);
        assert_eq!(repeated.matched, report.matched);
        let mut composed_first = DivisionsJob::new();
        let mut composed_second = DivisionsJob::new();
        let composed = copy_divisions_refresh(
            &memory,
            context,
            &mut composed_first,
            Some(&mut composed_second),
        );
        assert_eq!(report.raw_bytes, composed.raw_bytes);
        assert_eq!(report.matched, composed.matched);
        assert_eq!(report.mismatches, composed.mismatches);
        assert!(
            first
                .census
                .same(&composed_first.census, context.row_capacity)
        );
        assert_eq!(first.details.differences(&composed_first.details), [0; 5]);
    }

    #[test]
    fn divisions_seeded_worker_rejects_changed_context_without_reacquiring_inputs() {
        use super::divisions_test_kit as kit;
        struct NoRead;
        impl engine::Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("invalid worker context must not acquire live state");
            }
        }
        let bytes = kit::fixture();
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut job = DivisionsJob::new();
        assert!(copy_divisions_inputs_refresh(
            &kit::memory(&bytes),
            context,
            &mut job,
            None
        ));
        let lease = Arc::clone(job.census.unit_inputs().unwrap());
        let before = job.payload_bytes();
        let changed = DivisionsCopyContext {
            row_capacity: 0,
            ..context
        };
        assert!(!copy_divisions_core_refresh(&NoRead, changed, &mut job, None).raw_ok);
        assert_eq!(job.payload_bytes(), before);
        assert!(Arc::ptr_eq(&lease, job.census.unit_inputs().unwrap()));
        assert!(!copy_divisions_core_refresh(&NoRead, context, &mut job, None).raw_ok);
    }

    #[test]
    fn divisions_staging_overflow_falls_back_to_typed_walk() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        first.census = crate::division_census::Census::default();
        let mut second = DivisionsJob::new();
        second.census = crate::division_census::Census::default();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(!report.raw_ok);
        assert!(first.grow_for_next());
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
            ..Default::default()
        };
        let mut first = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(report.copy_ticks[0] > 0);
        let mut frame = Captured::new();
        frame.table_ns[6] = report.copy_ticks[0];
        let before = frame.table_ns;
        let mut ticks = [0; 6];
        super::decode_divisions_payload(&mut frame, kit::BASE, &mut first, &mut ticks);
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
            assert_eq!(report.matched, [cap >= 3; 6], "cap={cap}");
            kit::assert_matches_direct(&split, &direct);
        }
        let (exact, _, _) = kit::split(&memory, [3; 6]);
        assert_eq!(exact.divisions_outcome, Outcome::Rows { len: 3 });
    }
}
