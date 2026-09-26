//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use crate::{
    acquisition_factions, acquisition_jobs as acquisition, acquisition_states, acquisition_wars,
    decode, extended,
    map_capture::{MapCapacities, MapCaptured, ShipCensus},
    roster::Roster,
    table_capture::grow_capacity,
};
use crossbeam_queue::ArrayQueue;
#[cfg(test)]
use engine::cunit;
use engine::{
    carmy, ccountry, cfaction, cfaction_system,
    cgame_state::{self, Clock},
    country_memory::{Cache, SeedStatus},
    cprovince, cstate, cwar_relation,
    memory::{CountedMemory, LiveMemory, ReadCounts},
    msvc_string::Text,
    save_allocation_counters::{self, AllocationCounters},
    walk::Outcome,
};
use replay::{frame::TABLE_COUNT, rows::TableId};
use std::{
    cell::UnsafeCell,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

// Reuse a bounded producer/consumer pool; a burst waits rather than dropping an hour.
pub const BUFFERS: usize = 4;
/// One DivisionEtaInputs job per contiguous seed range.
const ETA_INPUT_RANGES: usize = extended::division_eta_inputs::MAX_RANGES;
const SUPPLY_PARTS: usize = extended::supply_trains::PARTS;
const CHARACTER_PARTS: usize = extended::characters::PARTS;
const PORTRAIT_PARTS: usize = extended::character_portraits::PARTS;
pub const SEGMENTS: usize = 15;
pub const SEGMENTS_SCOPE: &str = "game-thread wall of the parallel phase split at fixed marks: 0 first dispatch burst (ShipModels census, World, Research, off-slot Governance first); 1 division inputs copy + divisions/intel dispatch + character parts, relations, ordinary, slow and supply parts; 2 ShipModels census join + detail part dispatch, World join + Focus-slot governance/presentation/ETA dispatch; 3 unused; 4 AirWing join + wing install; 5 mid joins (divisions..intel) + Naval dispatch; 6 character roster join + portrait/Gated dispatch + Naval join; 7 map sample; 8 Orders join + Late dispatch + core joins; 9 supply/portraits/order/relations/ordinary/slow/gated/governance/research/character role joins; 10 capture_tables; 11 presentation + ETA joins; 12 Late join + capture_tables_late; 13 visibility + failure handling; 14 ShipModels detail part joins";
pub const INITIAL_CAPACITIES: [usize; 9] = [512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384];

/// Countries has one row per canonical slot and CountryLinks one per raw
/// link id; FactionMembers holds each country at most once. Order counters and
/// the war snapshot follow the Countries capacity when the frame grows.
pub fn raise_for_countries(core: &mut [usize; 9], slots: usize) {
    let rows = crate::country_scale::rows(slots, 1);
    for index in [0, 2, 7] {
        crate::country_scale::raise(&mut core[index], rows);
    }
}
/// Where the collector samples: at the entry of the hooked hourly tick, before
/// the trampoline runs, so a frame labelled with +468 is the state at the end
/// of that hour. Recorded in every session so readers know which instant it is.
pub const SAMPLE_BOUNDARY: replay::identity::SampleBoundary =
    replay::identity::SampleBoundary::EndOfHour;
const LOSS_RUNS: usize = 64;
pub static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);

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
    fn continuous(&self, clock: &Clock) -> bool {
        self.initialized
            && self.prev_hour.checked_add(1) == Some(clock.hour)
            && self.prev_signal == clock.play_session_started_at
            && self.prev_campaign == clock.campaign_id
    }
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

    fn push(&mut self, clock: Option<Clock>) {
        let Some(clock) = clock else {
            self.unknown_count += 1;
            return;
        };
        if let Some(last) = self.items[..self.len].last_mut()
            && last.campaign_id == clock.campaign_id
            && last.load_signal == Some(clock.play_session_started_at)
            && last.last_hour.checked_add(1) == Some(clock.hour)
        {
            last.last_hour = clock.hour;
            last.count += 1;
            return;
        }
        if self.len == LOSS_RUNS {
            self.unknown_count += 1;
            return;
        }
        self.items[self.len] = LostObservation {
            campaign_id: clock.campaign_id,
            first_hour: clock.hour,
            last_hour: clock.hour,
            loaded_hour: Some(clock.hour),
            load_signal: Some(clock.play_session_started_at),
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
    pub country_cache_status: SeedStatus,
    pub country_cache_entries: usize,
    pub country_cache_ns: u64,
    /// The sole clock observation, taken at hook entry before acquisition.
    pub clock: Option<Clock>,
    pub counters: Option<AllocationCounters>,
    pub countries: Vec<ccountry::Row>,
    pub provinces: Vec<cprovince::Row>,
    pub states: Vec<cstate::Row>,
    pub divisions: Vec<carmy::Row>,
    /// A frame-owned spare for an input-stage capacity miss before any lease.
    division_fallback: crate::division_census::Census,
    pub wars: Vec<cwar_relation::Row>,
    pub(crate) wars_snapshot: cwar_relation::OwnedSnapshotScratch,
    pub(crate) wars_snapshot_sampled: bool,
    pub factions: Vec<cfaction::Row>,
    pub faction_names: Vec<u8>,
    pub faction_names_len: usize,
    pub faction_names_required: usize,
    pub faction_members: Vec<cfaction_system::MemberRow>,
    pub links: Vec<ccountry::LinkRow>,
    pub countries_result: Outcome,
    pub provinces_result: Outcome,
    pub states_result: Outcome,
    pub divisions_result: Outcome,
    pub wars_result: Outcome,
    pub factions_result: Outcome,
    pub faction_members_result: Outcome,
    pub links_result: Outcome,
    pub table_ns: [u64; TABLE_COUNT],
    pub table_reads: [ReadCounts; TABLE_COUNT],
    pub total_ns: u64,
    pub qpc: i64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
    pub dropped_before: LossBatch,
}

impl Captured {
    /// Vanilla-sized frame for synthetic tests; production frames come from
    /// `Shared::with_sizing`.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_capacities(
            &INITIAL_CAPACITIES,
            &MapCapacities::default(),
            &extended::Capacities::default(),
        )
    }

    pub(crate) fn with_capacities(
        capacities: &[usize; 9],
        map_capacities: &MapCapacities,
        extended_capacities: &extended::Capacities,
    ) -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        let mut division_fallback = crate::division_census::Census::new();
        division_fallback.reserve_sources(capacities[4].max(extended_capacities.division_sources));
        Self {
            capture_seq: 0,
            order_counters: OrderCounterCapture::with_capacity(capacities[0]),
            map: MapCaptured::with_capacities(map_capacities),
            extended: extended::Captured::with_capacities(extended_capacities),
            country_cache_status: SeedStatus::Unavailable {
                reason: "clock unavailable",
            },
            country_cache_entries: 0,
            country_cache_ns: 0,
            clock: None,
            counters: None,
            countries: vec![ccountry::Row::default(); capacities[0]],
            provinces: vec![cprovince::Row::default(); capacities[1]],
            states: vec![cstate::Row::default(); capacities[3]],
            divisions: vec![carmy::Row::default(); capacities[4]],
            division_fallback,
            wars: vec![cwar_relation::Row::default(); capacities[5]],
            wars_snapshot: cwar_relation::OwnedSnapshotScratch::with_capacity(capacities[0]),
            wars_snapshot_sampled: false,
            factions: vec![cfaction::Row::default(); capacities[6]],
            faction_names: vec![0; capacities[8]],
            faction_names_len: 0,
            faction_names_required: 0,
            faction_members: vec![cfaction_system::MemberRow::default(); capacities[7]],
            links: vec![ccountry::LinkRow::default(); capacities[2]],
            countries_result: unavailable,
            provinces_result: unavailable,
            states_result: unavailable,
            divisions_result: unavailable,
            wars_result: unavailable,
            factions_result: unavailable,
            faction_members_result: unavailable,
            links_result: unavailable,
            table_ns: [0; TABLE_COUNT],
            table_reads: crate::read_accounting::ZERO,
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
            + self.division_fallback.allocated_bytes() as usize
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + self.provinces.capacity() * size_of::<cprovince::Row>()
            + self.states.capacity() * size_of::<cstate::Row>()
            + self.divisions.capacity() * size_of::<carmy::Row>()
            + self.wars.capacity() * size_of::<cwar_relation::Row>()
            + self.factions.capacity() * size_of::<cfaction::Row>()
            + self.faction_names.capacity()
            + self.faction_members.capacity() * size_of::<cfaction_system::MemberRow>()
            + self.links.capacity() * size_of::<ccountry::LinkRow>()
    }

    pub(crate) fn take_states_request(
        &mut self,
        base: usize,
        game_state: usize,
    ) -> acquisition_states::Request {
        acquisition_states::Request {
            census: acquisition::GatherRequest::take(base, game_state, &mut self.states),
            visitor: self.extended.take_state_visitor(),
        }
    }

    pub(crate) fn restore_states_request(
        &mut self,
        mut request: acquisition_states::Request,
        report: acquisition_states::Report,
        failure: Option<&'static str>,
    ) -> acquisition_states::Report {
        let report = failure.map_or(report, |reason| {
            request.visitor.fail_sources(reason);
            acquisition_states::Report::failed(reason)
        });
        request.census.restore(&mut self.states);
        self.extended.restore_state_visitor(request.visitor);
        self.states_result = report.census.result;
        report
    }

    pub(crate) fn take_wars_request(
        &mut self,
        countries: &Arc<Cache>,
        base: usize,
        game_state: usize,
    ) -> acquisition_wars::Request {
        acquisition_wars::Request {
            countries: Arc::clone(countries),
            table: acquisition::GatherRequest::take(base, game_state, &mut self.wars),
            snapshot: std::mem::take(&mut self.wars_snapshot),
            snapshot_sampled: false,
        }
    }

    pub(crate) fn restore_wars_request(
        &mut self,
        request: acquisition_wars::Request,
        report: acquisition_wars::Report,
        failure: Option<&'static str>,
    ) -> acquisition_wars::Report {
        let report = failure.map_or(report, acquisition_wars::Report::failed);
        request.table.restore(&mut self.wars);
        self.wars_snapshot = request.snapshot;
        self.wars_snapshot_sampled = request.snapshot_sampled;
        self.wars_result = report.table.result;
        report
    }

    /// Only the spool thread grows a frame while it owns that frame's queue index.
    pub fn grow(&mut self, capacities: &mut [usize; 9]) {
        fn resize<T: Default + Clone>(rows: &mut Vec<T>, outcome: Outcome, capacity: &mut usize) {
            if let Outcome::Truncated { len, unread_rows } = outcome {
                grow_capacity(capacity, len.saturating_add(unread_rows as usize));
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
        self.order_counters.grow(capacities[0]);
        self.wars_snapshot.grow(capacities[0]);
        resize(&mut self.states, self.states_result, &mut capacities[3]);
        resize(
            &mut self.divisions,
            self.divisions_result,
            &mut capacities[4],
        );
        let census = &mut self.division_fallback;
        if census.overflowed {
            census.grow();
        }
        census.reserve_sources(capacities[4].max(self.extended.division_capture_capacities().0));
        census.clear();
        resize(&mut self.wars, self.wars_result, &mut capacities[5]);
        resize(&mut self.factions, self.factions_result, &mut capacities[6]);
        resize(
            &mut self.faction_members,
            self.faction_members_result,
            &mut capacities[7],
        );
        grow_capacity(&mut capacities[8], self.faction_names_required);
        self.faction_names.resize(capacities[8], 0);
    }
}

/// Cumulative DivisionEtaInputs attribution. Game thread writer; `hours` is the
/// divisor. `walks` counts single acquisitions; spans are wall time inside the
/// owning worker rather than the whole game-thread block.
#[derive(Default)]
pub struct EtaInputStats {
    pub hours: AtomicU64,
    pub walks: AtomicU64,
    pub divisions: AtomicU64,
    pub legs: AtomicU64,
    pub graph_ns: AtomicU64,
    pub scan_ns: AtomicU64,
    pub other_ns: AtomicU64,
    pub geometry_hits: AtomicU64,
    pub geometry_misses: AtomicU64,
    pub static_hits: AtomicU64,
    pub static_misses: AtomicU64,
    pub static_cleared: AtomicU64,
    pub state_modifier_requests: AtomicU64,
    pub state_modifier_hits: AtomicU64,
    pub air_defense_requests: AtomicU64,
    pub air_defense_hits: AtomicU64,
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
    /// every join), then residual work and enqueue. Game thread writer;
    /// phase_calls is the divisor.
    pub phase_setup_ns: AtomicU64,
    pub phase_parallel_ns: AtomicU64,
    pub phase_residual_ns: AtomicU64,
    pub phase_calls: AtomicU64,
    /// The two halves of the parallel phase: the game thread's own typed walks
    /// (countries/links/wars/factions) versus the wall it spends joining the
    /// already-dispatched pool jobs.
    pub phase_inline_walk_ns: AtomicU64,
    pub phase_join_ns: AtomicU64,
    /// Game-thread wall between successive marks of the parallel phase; see
    /// `block_segments_scope` in status.json for what each segment spans.
    pub phase_segments: [AtomicU64; SEGMENTS],
    /// Game-thread wall spent joining each pool role, indexed by
    /// `acquisition::ROLES`.
    pub join_wait: [AtomicU64; acquisition::ROLE_COUNT],
    /// Existing QPC brackets around the original native tick, not collector
    /// acquisition. These diagnostic counters also include ticks with no free
    /// recording buffer and exclude game-loop work outside the hooked function.
    pub native_tick_ns: AtomicU64,
    pub native_tick_calls: AtomicU64,
    pub base: usize,
    pub trampoline: usize,
    pub frequency: u64,
    /// Every slot is allocated before hook installation and never moves.
    pub frames: [AtomicPtr<Captured>; BUFFERS],
    pub completed: [AtomicI64; BUFFERS],
    pub callback_tail: [AtomicI64; BUFFERS],
    frame_return: Mutex<()>,
    frame_available: Condvar,
    pub cancel_buffer_wait: AtomicBool,
    pub buffer_wait_ns: AtomicU64,
    pub buffer_wait_calls: AtomicU64,
    pub buffer_wait_max_ns: AtomicU64,
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
    cadence: UnsafeCell<CadenceState>,
    /// Research assignment fingerprints; loaned to the Research job each hour.
    /// Only the game thread takes and returns it.
    /// One store per sliced family (`slicing::FAMILIES`), loaned to that
    /// family's job each hour. Only the game thread takes and returns them.
    slice_fingerprints: [UnsafeCell<Option<Box<crate::slicing::SliceFingerprints>>>;
        crate::slicing::FAMILIES.len()],
    /// Fresh country authentication for this quiescent callback. Game thread only.
    country_cache: UnsafeCell<Arc<Cache>>,
    pub eta_inputs: EtaInputStats,
    /// Set by the spool when a staggered table truncated or a static table
    /// failed: the next hour walks everything again, with grown buffers.
    pub force_next: AtomicBool,
    /// The lexer's maximum token id at the last dictionary walk; a change
    /// means tokens were appended and the static dictionary is walked again.
    pub lexical_max: AtomicI32,
    /// A larger country authentication cache prepared off the game thread.
    /// The hook swaps it in with a non-blocking `try_lock` and parks the old
    /// one in `retired` for the spool thread to free: no allocation or
    /// deallocation happens on the game thread.
    pub country_cache_swap: Mutex<CountryCacheSwap>,
    /// Cells of the installed country cache, published by the game thread.
    pub country_cache_cells: AtomicUsize,
    /// The one CountryIntel buffer every frame borrows; counted once in
    /// `pool_bytes`.
    pub country_intel: crate::extended::daily_intel::Home,
}

#[derive(Default)]
pub struct CountryCacheSwap {
    pub next: Option<Arc<Cache>>,
    pub retired: Option<Arc<Cache>>,
}

// Each frame index has exactly one owner: a queue, the game thread inside the
// hook, or the spool thread. The game thread runs the whole acquisition body
// before the trampoline, so its single-writer fields stay single-writer.
unsafe impl Sync for Shared {}

impl Shared {
    /// Registry-free sizing for synthetic tests; the loader uses `with_sizing`.
    #[cfg(test)]
    pub fn new(base: usize, trampoline: usize, frequency: u64) -> Self {
        Self::with_sizing(
            base,
            trampoline,
            frequency,
            &crate::country_scale::Sizing::for_slots(0),
        )
    }

    /// Every frame and the country cache are allocated from `sizing`, which
    /// the loader derives from the live country registry before hooking.
    pub fn with_sizing(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
    ) -> Self {
        let roster = Box::new(Roster::with_counts(sizing.content));
        let country_cache = Arc::new(Cache::with_capacity(sizing.country_cells));
        let country_cells = country_cache.capacity();
        // Frame 0 allocates the CountryIntel buffer; the others borrow it.
        let first = Captured::with_capacities(&sizing.core, &sizing.map, &sizing.extended);
        let country_intel = first
            .extended
            .country_intel_home()
            .expect("a new frame holds its intel pack");
        let pool_bytes = roster.allocated_bytes()
            + country_cache.allocated_bytes()
            + country_intel.allocated_bytes();
        let shared = Self {
            acquisition: OnceLock::new(),
            collect_wait_ns: AtomicU64::new(0),
            acquisition_panics: AtomicU64::new(0),
            phase_setup_ns: AtomicU64::new(0),
            phase_parallel_ns: AtomicU64::new(0),
            phase_residual_ns: AtomicU64::new(0),
            phase_calls: AtomicU64::new(0),
            phase_inline_walk_ns: AtomicU64::new(0),
            phase_join_ns: AtomicU64::new(0),
            phase_segments: std::array::from_fn(|_| AtomicU64::new(0)),
            join_wait: std::array::from_fn(|_| AtomicU64::new(0)),
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
            frame_return: Mutex::new(()),
            frame_available: Condvar::new(),
            cancel_buffer_wait: AtomicBool::new(false),
            buffer_wait_ns: AtomicU64::new(0),
            buffer_wait_calls: AtomicU64::new(0),
            buffer_wait_max_ns: AtomicU64::new(0),
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
            cadence: UnsafeCell::new(CadenceState::default()),
            slice_fingerprints: std::array::from_fn(|_| {
                UnsafeCell::new(Some(Box::new(crate::slicing::SliceFingerprints::new())))
            }),
            country_cache: UnsafeCell::new(country_cache),
            eta_inputs: EtaInputStats::default(),
            force_next: AtomicBool::new(false),
            lexical_max: AtomicI32::new(-1),
            country_cache_swap: Mutex::new(CountryCacheSwap::default()),
            country_cache_cells: AtomicUsize::new(country_cells),
            country_intel,
        };
        let mut borrowed = sizing.extended;
        borrowed.intel.rows[15] = 0;
        borrowed.intel.intel_scratch = 0;
        // No callback or spool thread can observe Shared during construction.
        unsafe { shared.publish_frame(0, Box::new(first)) };
        for index in 1..BUFFERS {
            let mut frame = Captured::with_capacities(&sizing.core, &sizing.map, &borrowed);
            frame.extended.share_country_intel(&shared.country_intel);
            unsafe { shared.publish_frame(index, Box::new(frame)) };
        }
        shared
    }

    /// Spool thread: free a retired cache and offer a larger one when the
    /// live registry or a capacity-limited census asks for more cells.
    pub fn offer_country_cache(&self, cells: usize) {
        let Ok(mut swap) = self.country_cache_swap.lock() else {
            return;
        };
        if let Some(retired) = swap.retired.take() {
            self.pool_bytes
                .fetch_sub(retired.allocated_bytes() as u64, Ordering::Relaxed);
            drop(retired);
        }
        let installed = self.country_cache_cells.load(Ordering::Acquire);
        let offered = swap.next.as_ref().map_or(0, |next| next.capacity());
        if cells > installed && cells > offered {
            if let Some(previous) = swap.next.take() {
                self.pool_bytes
                    .fetch_sub(previous.allocated_bytes() as u64, Ordering::Relaxed);
            }
            let next = Arc::new(Cache::with_capacity(cells));
            self.pool_bytes
                .fetch_add(next.allocated_bytes() as u64, Ordering::Relaxed);
            swap.next = Some(next);
        }
    }

    /// Game thread, before preparing the hour's country snapshot: install an
    /// offered cache without blocking, allocating or freeing.
    pub(crate) fn install_offered_country_cache(&self) {
        let Ok(mut swap) = self.country_cache_swap.try_lock() else {
            return;
        };
        if swap.retired.is_some() {
            return;
        }
        let Some(next) = swap.next.take() else {
            return;
        };
        let cells = next.capacity();
        // Every previous lease was returned before this callback began.
        let current = unsafe { &mut *self.country_cache.get() };
        swap.retired = Some(std::mem::replace(current, next));
        self.country_cache_cells.store(cells, Ordering::Release);
    }

    /// Initialize one slot before Shared can be observed by any worker or hook.
    unsafe fn publish_frame(&self, index: usize, frame: Box<Captured>) {
        debug_assert!(self.frames[index].load(Ordering::Relaxed).is_null());
        let bytes = frame.allocated_bytes();
        // Acquire after popping a free index makes the entire allocation visible.
        self.frames[index].store(Box::into_raw(frame), Ordering::Release);
        self.pool_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        // A newly allocated slot has no index in either queue. Both queues have
        // room for every allocated slot, including ones currently held by a thread.
        let published = self.free.push(index);
        debug_assert!(published.is_ok());
    }

    /// The callback retains the hourly boundary while an existing frame drains.
    /// The common path is only a queue pop; no native reads or allocations are
    /// repeated while waiting. Loader cancellation prevents shutdown deadlocks.
    pub fn take_frame(&self) -> Option<usize> {
        if self.acquisition_failed() {
            return None;
        }
        if let Some(index) = self.free.pop() {
            return Some(index);
        }
        let started = qpc();
        self.buffer_wait_calls.fetch_add(1, Ordering::Relaxed);
        let mut returned = self.frame_return.lock().unwrap_or_else(|e| e.into_inner());
        let index = loop {
            if let Some(index) = self.free.pop() {
                break Some(index);
            }
            if self.cancel_buffer_wait.load(Ordering::Acquire) {
                break None;
            }
            // Cancellation must work even if a failed consumer cannot notify us.
            returned = self
                .frame_available
                .wait_timeout(returned, std::time::Duration::from_millis(5))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        };
        drop(returned);
        let elapsed = self.ns(qpc() - started);
        self.buffer_wait_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.buffer_wait_max_ns
            .fetch_max(elapsed, Ordering::Relaxed);
        index
    }

    /// At most one reusable capture buffer is free: the next callback may wait
    /// in `take_frame` for the decode and spool threads to return one.
    pub fn buffers_backlogged(&self) -> bool {
        self.free.len() <= 1
    }

    pub fn acquisition_failed(&self) -> bool {
        self.acquisition_panics.load(Ordering::Acquire) != 0
    }

    /// Publish all completed writes before waking a callback waiting for reuse.
    pub fn recycle_frame(&self, index: usize) {
        let _returned = self.frame_return.lock().unwrap_or_else(|e| e.into_inner());
        let published = self.free.push(index);
        debug_assert!(published.is_ok());
        self.frame_available.notify_one();
    }

    /// Fold one hour's ETA input attribution into the cumulative counters.
    fn record_eta_inputs(&self, counters: engine::carmy::EtaInputCounters) {
        let add = |slot: &AtomicU64, value: u64| {
            slot.fetch_add(value, Ordering::Relaxed);
        };
        let stats = &self.eta_inputs;
        add(&stats.hours, 1);
        add(&stats.walks, counters.walks);
        add(&stats.divisions, counters.divisions);
        add(&stats.legs, counters.legs);
        add(&stats.graph_ns, self.ns(counters.graph_ticks as i64));
        add(&stats.scan_ns, self.ns(counters.scan_ticks as i64));
        add(&stats.other_ns, self.ns(counters.other_ticks as i64));
        add(&stats.geometry_hits, counters.geometry_hits);
        add(&stats.geometry_misses, counters.geometry_misses);
        add(&stats.static_hits, counters.static_hits);
        add(&stats.static_misses, counters.static_misses);
        add(&stats.static_cleared, counters.static_cleared);
        add(
            &stats.state_modifier_requests,
            counters.state_modifier_requests,
        );
        add(&stats.state_modifier_hits, counters.state_modifier_hits);
        add(&stats.air_defense_requests, counters.air_defense_requests);
        add(&stats.air_defense_hits, counters.air_defense_hits);
    }

    pub fn ns(&self, ticks: i64) -> u64 {
        ((ticks.max(0) as u128 * 1_000_000_000) / self.frequency as u128) as u64
    }

    /// Called only by the game thread inside the hook. A failed clock postpones
    /// this once-only read until the next readable hour, free buffer or not.
    unsafe fn sample_roster(&self, memory: &impl engine::Memory, clock: Option<Clock>) {
        if self.roster_ready.load(Ordering::Acquire) {
            return;
        }
        if let Some(clock) = clock {
            let _reads = crate::read_accounting::table(replay::rows::TableId::Players);
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

/// Restore an owned frame's absorbed witnesses, record its lost hour and return
/// the buffer. Shared by every queue-full branch; no new loss reasons.
fn drop_owned_frame(shared: &Shared, frame: &mut Captured, index: usize) {
    // The dropped hour's sliced reads are never published: read them again.
    // A clockless frame read nothing, and its selections belong to an older hour.
    let read = frame.clock.is_some();
    for (family, fingerprints) in shared
        .slice_fingerprints
        .iter()
        .enumerate()
        .filter(|_| read)
    {
        // SAFETY: game thread only; every store was returned by this callback.
        if let Some(fingerprints) = unsafe { &mut *fingerprints.get() }.as_mut() {
            frame
                .extended
                .family_selection(family)
                .invalidate(fingerprints);
        }
    }
    unsafe {
        *shared.pending.get() = frame.dropped_before;
        (&mut *shared.pending.get()).push(frame.clock);
    }
    shared.dropped.fetch_add(1, Ordering::Relaxed);
    shared.recycle_frame(index);
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
/// could be obtained before shutdown cancellation: retain an explicit loss witness.
struct Dispatch {
    index: Option<usize>,
    entry: Option<Clock>,
    entry_ticks: i64,
    dispatched: i64,
}

#[cfg(feature = "callback_profile")]
fn callback_profile_counters(shared: &Shared) -> crate::callback_profile::Sample {
    let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
    crate::callback_profile::Sample {
        buffer_wait_ns: load(&shared.buffer_wait_ns),
        setup_ns: load(&shared.phase_setup_ns),
        parallel_ns: load(&shared.phase_parallel_ns),
        residual_ns: load(&shared.phase_residual_ns),
        dispatch_ns: load(&shared.acquisition_dispatch_ns),
        phase_calls: load(&shared.phase_calls),
        segments_ns: shared.phase_segments.each_ref().map(load),
        join_wait_ns: shared.join_wait.each_ref().map(load),
        ..Default::default()
    }
}

/// Contain callback failures without reusing partially restored capture storage.
/// Once a callback panics, subsequent hours record losses until the loader
/// restores the hook; they must not consume more buffers or wait for the lost one.
///
/// # Safety
/// Only the sole game-thread callback may mutate the pending loss batch.
unsafe fn guarded_acquisition(
    shared: &Shared,
    entry: Option<Clock>,
    acquire: impl FnOnce(),
) -> bool {
    if !shared.acquisition_failed() {
        match catch_unwind(AssertUnwindSafe(acquire)) {
            Ok(()) => return true,
            Err(payload) => {
                // A panic payload can itself panic on Drop. At most one is
                // retained because this attachment now stops acquisition.
                std::mem::forget(payload);
                shared.cancel_buffer_wait.store(true, Ordering::Release);
                shared.acquisition_panics.fetch_add(1, Ordering::Release);
            }
        }
    }
    unsafe { (&mut *shared.pending.get()).push(entry) };
    shared.dropped.fetch_add(1, Ordering::Relaxed);
    false
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
        let _reads = crate::read_accounting::Capture::new();
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        // Read the clock at entry: +468 still the hour just ended, the trampoline
        // has not run. The frame's identity and the settled state the body reads.
        let clock_start = qpc();
        let entry = cgame_state::read_clock(&memory, shared.base);
        let entry_ticks = qpc() - clock_start;
        #[cfg(feature = "callback_profile")]
        let profile_before = callback_profile_counters(shared);
        #[cfg(feature = "callback_profile")]
        crate::callback_profile::reset_join();
        #[cfg(feature = "callback_profile")]
        let profile_stamp = crate::callback_profile::Stamp::start();
        let dispatched = qpc();
        let index = shared.take_frame();
        if let Some(index) = index {
            shared.completed[index].store(0, Ordering::Relaxed);
            shared.callback_tail[index].store(0, Ordering::Relaxed);
        }
        // The body reads on this hot core; pool workers read in parallel and
        // this thread helps drain them. A Rust panic must never unwind across
        // the extern boundary into the game: contain it and drop the hour.
        let job = Dispatch {
            index,
            entry,
            entry_ticks,
            dispatched,
        };
        if unsafe { guarded_acquisition(shared, entry, || acquire(shared, job)) }
            && let Some(index) = index
        {
            shared.callback_tail[index].store(1, Ordering::Release);
        }
        // Include buffer backpressure in the whole game-thread block before the trampoline.
        // Its percentiles are callback_work; this is only the cheap cumulative.
        shared
            .collect_wait_ns
            .fetch_add(shared.ns(qpc() - dispatched), Ordering::Relaxed);
        #[cfg(feature = "callback_profile")]
        {
            let mut sample = profile_stamp.finish(shared.frequency);
            let after = callback_profile_counters(shared);
            sample.hour = entry.map(|clock| clock.hour);
            sample.buffer_wait_ns = after
                .buffer_wait_ns
                .saturating_sub(profile_before.buffer_wait_ns);
            sample.setup_ns = after.setup_ns.saturating_sub(profile_before.setup_ns);
            sample.parallel_ns = after.parallel_ns.saturating_sub(profile_before.parallel_ns);
            sample.residual_ns = after.residual_ns.saturating_sub(profile_before.residual_ns);
            sample.dispatch_ns = after.dispatch_ns.saturating_sub(profile_before.dispatch_ns);
            sample.phase_calls = after.phase_calls.saturating_sub(profile_before.phase_calls);
            sample.segments_ns = std::array::from_fn(|i| {
                after.segments_ns[i].saturating_sub(profile_before.segments_ns[i])
            });
            sample.join_wait_ns = std::array::from_fn(|i| {
                after.join_wait_ns[i].saturating_sub(profile_before.join_wait_ns[i])
            });
            sample.join = crate::callback_profile::take_join();
            crate::callback_profile::record(sample);
        }
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
    let memory = CountedMemory::new(unsafe { LiveMemory::new() });

    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.clock = entry;
        frame.qpc = dispatched;
        frame.table_ns = [0; TABLE_COUNT];
        frame.table_reads = crate::read_accounting::ZERO;
        let before = qpc();
        // Passive scalar reads of the same counters the save header emits.
        frame.counters = frame.clock.map(|clock| {
            save_allocation_counters::read_counters(&memory, shared.base, clock.game_state as usize)
        });
        let clock_ticks = entry_ticks + qpc() - before;
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        frame.countries_result = unavailable;
        frame.provinces_result = unavailable;
        frame.states_result = unavailable;
        frame.divisions_result = unavailable;
        frame.wars_result = unavailable;
        frame.wars_snapshot_sampled = false;
        frame.factions_result = unavailable;
        frame.faction_names_len = 0;
        frame.faction_names_required = 0;
        frame.faction_members_result = unavailable;
        frame.links_result = unavailable;
        frame.map.reset();
        frame.extended.reset();
        frame.country_cache_status = SeedStatus::Unavailable {
            reason: "clock unavailable",
        };
        frame.country_cache_entries = 0;
        frame.country_cache_ns = 0;
        frame.order_counters.reset();
        // Relations handoff state: same FIFO, same ordering guarantees.
        let mut relations_payload: Option<extended::intel::RelationsJob> = None;
        let mut relations_report = extended::intel::RelationsCopyReport::default();
        // Characters handoff state: same FIFO, same ordering guarantees.
        let mut characters_payload: [Option<extended::characters::CharactersJob>; CHARACTER_PARTS] =
            [const { None }; CHARACTER_PARTS];
        let mut characters_report =
            [extended::characters::CharactersCopyReport::default(); CHARACTER_PARTS];
        // Projects handoff state: same FIFO, same ordering guarantees.
        let mut projects_payload: Option<extended::projects::ProjectsJob> = None;
        let mut projects_report = extended::projects::ProjectsCopyReport::default();
        // Lexical handoff state: same FIFO, same ordering guarantees.
        let mut lexical_payload: Option<extended::lexical::LexicalJob> = None;
        let mut lexical_report = extended::lexical::LexicalCopyReport::default();
        // Divisions handoff state: same FIFO, same ordering guarantees.
        // The hourly payload carries the divisions walk and any scheduled
        // equipment/upgrade detail walks sharing its census.
        let mut divisions_payload: Option<DivisionsJob> = None;
        let mut divisions_report = DivisionsCopyReport::default();
        // Focus handoff state: same FIFO, same ordering guarantees. The
        // scheduled payload carries the summary walk plus the completed
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
        // Players handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the independent roster
        // and controller walks with its own root refs and no
        // cross-family decoded roots. Staging overflow falls back to the
        // typed walks with no payload enqueued.
        let mut players_payload: Option<crate::map_capture::PlayersJob> = None;
        let mut players_report = crate::map_capture::PlayersCopyReport::default();
        // Session family handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries the manager, sources
        // and aggregates with its own root refs and no cross-family
        // decoded roots. Staging overflow falls back to the typed walks
        // with no payload enqueued.
        let mut session_payload: Option<extended::session::SessionJob> = None;
        let mut session_report = extended::session::SessionCopyReport::default();
        // Production rest handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries lines, stockpiles,
        // licences, the deployment queue and licence records with its own
        // census and no cross-family decoded roots. Staging overflow
        // falls back to the typed walks with no payload enqueued.
        let mut production_rest_payload: Option<extended::production::ProductionRestJob> = None;
        let mut production_rest_report = extended::production::ProductionRestCopyReport::default();
        // Construction rest handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries resources, the
        // construction queue, occupation, resistance and collaboration
        // with its own census and no cross-family decoded roots. Staging
        // overflow falls back to the typed walks with no payload enqueued.
        let mut construction_rest_payload: Option<extended::construction::ConstructionRestJob> =
            None;
        let mut construction_rest_report =
            extended::construction::ConstructionRestCopyReport::default();
        // Economy hourly rest handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries province supply plus scheduled
        // fuel, experience, hubs, disrupted, foreign, definitions, resources,
        // exchanges and purchases with its own census and no
        // cross-family decoded roots. Staging overflow falls back to the
        // typed walks with no payload enqueued.
        let mut economy_rest_payload: Option<extended::economy::EconomyRestJob> = None;
        let mut economy_rest_report = extended::economy::EconomyRestCopyReport::default();
        // Shared hourly bases handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the wing/manager census
        // once with bases, registrations and scheduled aces detail; the
        // decodes read the shared spans. Staging overflow falls back to
        // the typed walks with no payload enqueued.
        let mut air_bases_payload: Option<extended::air::AirBasesJob> = None;
        let mut air_bases_report = extended::air::AirBasesCopyReport::default();
        // Shared history handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries the regional walk once
        // with history and damage detail; both decodes read the shared
        // spans. Staging overflow falls back to the typed walk with no
        // payload enqueued.
        let mut air_history_payload: Option<extended::air::AirHistoryJob> = None;
        let mut air_history_report = extended::air::AirHistoryCopyReport::default();
        // Scheduled supremacy handoff state: same FIFO, same ordering
        // guarantees. Staging overflow falls back to the typed walk with
        // no payload enqueued.
        let mut supremacy_payload: Option<extended::naval_history::SupremacyJob> = None;
        let mut supremacy_report = extended::naval_history::SupremacyCopyReport::default();
        // Scheduled sunk-ships handoff state: same FIFO, same ordering
        // guarantees. Staging overflow falls back to the typed walk with
        // no payload enqueued.
        let mut sunk_payload: Option<extended::naval_history::SunkJob> = None;
        let mut sunk_report = extended::naval_history::SunkCopyReport::default();
        // Scheduled radar handoff state: same FIFO, same ordering
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
        // The two halves of the parallel phase.
        let mut inline_walk_ns = 0u64;
        let mut join_ns = 0u64;
        if let Some(clock) = frame.clock {
            let cadence = unsafe { &mut *shared.cadence.get() };
            let continuous = cadence.continuous(&clock);
            let recovery = shared.force_next.swap(false, Ordering::AcqRel);
            let force_full = cadence.force_full(&clock) | recovery;
            // A periodic encoding keyframe does not interrupt native mutation
            // observation. Its freshly certified owned rows remain complete.
            if !continuous || recovery {
                crate::portrait_changes::invalidate();
                crate::advisor_changes::invalidate();
            }
            frame.extended.prepare_hour(clock.hour, force_full);
            let state = clock.game_state as usize;
            let cache_begin = qpc();
            shared.install_offered_country_cache();
            let country_snapshot =
                crate::read_accounting::with_table(replay::rows::TableId::Countries, || {
                    Cache::prepare_shared(
                        unsafe { &mut *shared.country_cache.get() },
                        &memory,
                        shared.base,
                        state,
                    )
                })
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
            let mut segment_prev = ts_setup;
            macro_rules! segment {
                ($index:expr) => {{
                    let now = qpc();
                    shared.phase_segments[$index]
                        .fetch_add(shared.ns(now - segment_prev), Ordering::Relaxed);
                    segment_prev = now;
                }};
            }
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
            let characters_refresh = decode::characters_refresh_hour(clock.hour, force_full);
            let projects_refresh = decode::projects_refresh_hour(clock.hour, force_full);
            let lexical_max =
                engine::read::<i32>(&memory, shared.base + engine::cpdx_lexer::MAX_TOKEN_ID_RVA)
                    .unwrap_or(-1);
            let lexical_refresh = decode::lexical_refresh_hour(clock.hour, force_full)
                || lexical_max != shared.lexical_max.load(Ordering::Relaxed);
            if lexical_refresh {
                shared.lexical_max.store(lexical_max, Ordering::Relaxed);
            }
            let relations_refresh = decode::relations_refresh_hour(clock.hour, force_full);
            // ShipModels is the longest single job and walks its own ship census:
            // dispatch it first so it overlaps everything else.
            let ship_census = std::mem::replace(&mut frame.map.naval, ShipCensus::vacant());
            let ship_models_pending = queue!(acquisition::Job::ShipModels(
                frame.extended.take_ship_models_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    ship_census,
                )
            ));
            let world_request = acquire_world_request(
                &country_snapshot,
                shared.base,
                state,
                frame.extended.focus_pilot().focus_copy_context(
                    shared.base,
                    state,
                    replay::cadence::should_walk(TableId::Focus, clock.hour, force_full),
                ),
                frame.extended.economy_rest_pilot(),
                clock.hour,
                force_full,
            );
            let world_pending = queue!(acquisition::Job::World(world_request));
            // The longest hourly jobs go out first: the block ends when they do.
            // The research tables are loaned before the family that owns them.
            // SAFETY: only this callback takes and returns the store.
            let fingerprints =
                unsafe { &mut *shared.slice_fingerprints[crate::slicing::RESEARCH].get() }
                    .take()
                    .expect("research fingerprints returned by the previous callback");
            let research_pending = queue!(acquisition::Job::Research(
                frame.extended.take_research_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    fingerprints,
                )
            ));
            // Governance waits for the World join only to learn whether World
            // delegated this hour's Focus walk. Off the Focus slot there is
            // nothing to delegate, so it leaves in the first burst instead.
            let focus_hour = replay::cadence::should_walk(TableId::Focus, clock.hour, force_full);
            let early_governance = (!focus_hour).then(|| {
                // Off the Focus slot World delegates nothing: the loaned family
                // must not carry an older delegation into this capture.
                frame.extended.focus_pilot().focus_raw_mode = false;
                let request = frame.extended.take_governance_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                );
                let bytes = request.allocated_bytes();
                (queue!(acquisition::Job::Governance(request)), bytes)
            });
            let orders_request = frame
                .extended
                .take_orders_request(&mut frame.map.orders, shared.base);
            let orders_bytes = orders_request.allocated_bytes();
            let orders_pending = queue!(acquisition::Job::Orders(orders_request));
            // Naval uses this callback's independently bounded wing-membership census.
            let mut naval_job = decode::acquire_naval_job();
            let wing_limits = frame.extended.air_wing_pilot().wing_limits();
            let air_ctx = extended::air::AirWingCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
                limits: [
                    frame.map.air_wings.rows.len(),
                    wing_limits[0],
                    wing_limits[1],
                ],
                naval_limit: naval_job.wing_limit(),
            };
            let air_pending = queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: air_ctx,
                payload: decode::acquire_air_wing_job(),
            }));
            let air_bases_ctx = extended::air::AirBasesCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let air_bases_pending = {
                let payload = decode::acquire_air_bases_job();
                queue!(acquisition::Job::AirBases(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: air_bases_ctx,
                    payload,
                }))
            };
            let intel_residual_pending = queue!(acquisition::Job::IntelResidual(
                frame.extended.take_intel_residual_request(
                    &country_snapshot,
                    extended::intel::CaptureContext {
                        base: shared.base,
                        game_state: state,
                        hour: clock.hour,
                        force_full,
                    },
                )
            ));
            let provinces_pending = queue!(acquisition::Job::Provinces(
                acquisition::ProvincesRequest::take(shared.base, state, &mut frame.provinces)
            ));
            let states_pending = queue!(acquisition::Job::States(
                frame.take_states_request(shared.base, state)
            ));
            let factions_pending =
                queue!(acquisition::Job::Factions(acquisition_factions::Request {
                    countries: Arc::clone(&country_snapshot),
                    factions: acquisition::GatherRequest {
                        base: shared.base,
                        game_state: state,
                        payload: acquisition_factions::Names::take(
                            &mut frame.factions,
                            &mut frame.faction_names
                        ),
                    },
                    members: acquisition::GatherRequest::take(
                        shared.base,
                        state,
                        &mut frame.faction_members
                    ),
                }));
            let countries_pending = queue!(acquisition::Job::Countries(
                acquisition::CountriesRequest::take(
                    &country_snapshot,
                    shared.base,
                    state,
                    &mut frame.countries,
                    &mut frame.links
                )
            ));
            let wars_pending = queue!(acquisition::Job::Wars(frame.take_wars_request(
                &country_snapshot,
                shared.base,
                state
            )));
            let province_unit_order_pending = queue!(acquisition::Job::ProvinceUnitOrder(
                frame
                    .extended
                    .take_province_unit_order_request(shared.base, state,)
            ));
            let players_ctx = crate::map_capture::PlayersCopyContext {
                base: shared.base,
                game_state: state,
            };
            let players_pending = {
                let payload = decode::acquire_players_job();
                queue!(acquisition::Job::Players(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: players_ctx,
                    payload,
                }))
            };
            let air_stats_ctx = extended::air::AirStatsCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let air_stats_pending =
                if replay::cadence::should_walk(TableId::AirStatistics, clock.hour, force_full) {
                    let payload = decode::acquire_air_stats_job();
                    Some(queue!(acquisition::Job::AirStats(
                        acquisition::RawRequest {
                            countries: Arc::clone(&country_snapshot),
                            context: air_stats_ctx,
                            payload,
                        }
                    )))
                } else {
                    frame.extended.air_stats_pilot().stats_raw_mode = false;
                    None
                };
            let air_history_ctx = extended::air::AirHistoryCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
                history_limit: frame.extended.air_history_pilot().air_history_limits()[0],
                damage_limit: frame.extended.air_history_pilot().air_history_limits()[1],
            };
            let air_history_pending = if replay::cadence::should_walk(
                TableId::AirRegionHistory,
                clock.hour,
                force_full,
            ) {
                let payload = decode::acquire_air_history_job();
                Some(queue!(acquisition::Job::AirHistory(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: air_history_ctx,
                        payload,
                    }
                )))
            } else {
                frame.extended.air_history_pilot().history_raw_mode = false;
                None
            };
            let supremacy_ctx = extended::naval_history::SupremacyCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let supremacy_pending =
                if replay::cadence::should_walk(TableId::NavalSupremacy, clock.hour, force_full) {
                    let payload = decode::acquire_supremacy_job();
                    Some(queue!(acquisition::Job::Supremacy(
                        acquisition::RawRequest {
                            countries: Arc::clone(&country_snapshot),
                            context: supremacy_ctx,
                            payload,
                        }
                    )))
                } else {
                    frame.extended.supremacy_pilot().supremacy_raw_mode = false;
                    None
                };
            let sunk_ctx = extended::naval_history::SunkCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let sunk_pending =
                if replay::cadence::should_walk(TableId::SunkShips, clock.hour, force_full) {
                    let payload = decode::acquire_sunk_job();
                    Some(queue!(acquisition::Job::Sunk(acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: sunk_ctx,
                        payload,
                    })))
                } else {
                    frame.extended.sunk_pilot().sunk_raw_mode = false;
                    None
                };
            let radar_ctx = frame.extended.radar_pilot().radar_copy_context(
                shared.base,
                state,
                clock.hour,
                force_full,
            );
            let radar_pending =
                if replay::cadence::should_walk(TableId::RadarCoverage, clock.hour, force_full) {
                    let payload = decode::acquire_radar_job();
                    Some(queue!(acquisition::Job::Radar(acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: radar_ctx,
                        payload,
                    })))
                } else {
                    frame.extended.radar_pilot().radar_raw_mode = false;
                    None
                };
            let weather_ctx = extended::weather::WeatherCopyContext { game_state: state };
            let weather_pending = {
                let payload = decode::acquire_weather_job();
                queue!(acquisition::Job::Weather(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: weather_ctx,
                    payload,
                }))
            };
            let capitulation_ctx = extended::intel::CapitulationCopyContext {
                base: shared.base,
                game_state: state,
            };
            let capitulation_pending = {
                let payload = decode::acquire_capitulation_job();
                queue!(acquisition::Job::Capitulation(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: capitulation_ctx,
                    payload,
                }))
            };
            let projects_ctx = extended::projects::ProjectsCopyContext {
                base: shared.base,
                game_state: state,
                active: projects_refresh,
            };
            let projects_pending = if projects_refresh {
                let payload = decode::acquire_projects_job();
                Some(queue!(acquisition::Job::Projects(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: projects_ctx,
                        payload,
                    }
                )))
            } else {
                frame.extended.projects_pilot().raw_mode = false;
                None
            };
            let lexical_ctx = extended::lexical::LexicalCopyContext { base: shared.base };
            let lexical_pending = if lexical_refresh {
                let payload = decode::acquire_lexical_job();
                Some(queue!(acquisition::Job::Lexical(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: lexical_ctx,
                    payload,
                })))
            } else {
                None
            };
            let session_ctx = extended::session::SessionCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let session_pending =
                if replay::cadence::should_walk(TableId::WorldTension, clock.hour, force_full) {
                    let payload = decode::acquire_session_job();
                    Some(queue!(acquisition::Job::Session(acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: session_ctx,
                        payload,
                    })))
                } else {
                    frame.extended.session_pilot().raw_mode = false;
                    None
                };
            segment!(0);
            let mut divisions_job = Some(decode::acquire_divisions_job());
            let (division_source_capacity, detail_capacities) =
                frame.extended.division_capture_capacities();
            let divisions_ctx = DivisionsCopyContext {
                base: shared.base,
                game_state: state,
                row_capacity: frame.divisions.len(),
                source_capacity: division_source_capacity,
                detail_capacities,
                details_active: replay::cadence::should_walk(
                    TableId::DivisionEquipment,
                    clock.hour,
                    force_full,
                ),
            };
            // Input-stage admission precedes every consumer's lease. A later
            // core completion cannot authorize another source acquisition.
            let divisions_owned = copy_divisions_inputs_refresh(
                &cached,
                divisions_ctx,
                divisions_job.as_mut().expect("owned input job"),
            );
            let inputs = if divisions_owned {
                divisions_job
                    .as_ref()
                    .unwrap()
                    .census
                    .unit_inputs()
                    .map(Arc::clone)
            } else {
                let begin = qpc();
                decode::recycle_divisions_job(divisions_job.take().unwrap());
                let _reads = crate::read_accounting::table(replay::rows::TableId::Divisions);
                frame.extended.set_divisions_raw_mode(false);
                let capacity = frame.divisions.len().max(division_source_capacity);
                frame
                    .division_fallback
                    .capture(&cached, shared.base, state, capacity);
                frame.divisions_result = frame.division_fallback.install_rows(&mut frame.divisions);
                supply_division_census_sources(
                    &frame.division_fallback,
                    &mut frame.extended,
                    division_source_capacity,
                );
                frame.table_ns[6] = shared.ns(qpc() - begin);
                frame.division_fallback.unit_inputs().map(Arc::clone)
            };
            frame
                .extended
                .install_unit_inputs(inputs.as_ref().map(Arc::clone));
            let divisions_pending = divisions_job.take().map(|payload| {
                queue!(acquisition::Job::Divisions(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: divisions_ctx,
                    payload
                }))
            });
            let intel_ctx = frame
                .extended
                .intel_pilot()
                .intel_copy_context(shared.base, state);
            let intel_pending = queue!(acquisition::Job::Intel(acquisition::IntelRequest {
                countries: Arc::clone(&country_snapshot),
                context: intel_ctx,
                payload: decode::acquire_intel_job(),
                inputs: inputs.as_ref().map(Arc::clone),
            }));
            // The fat late-joined jobs go out after Divisions and Intel so those
            // find a free worker; the burst's short jobs have freed some by now.
            let (trait_rows, trait_bytes) =
                frame.extended.characters_pilot().trait_capture_capacities();
            let characters_ctx = extended::characters::CharactersCopyContext {
                base: shared.base,
                game_state: state,
                trait_rows,
                trait_bytes,
                active: decode::CHARACTERS
                    .map(|table| replay::cadence::should_walk(table, clock.hour, force_full)),
            };
            frame
                .extended
                .characters_pilot()
                .set_character_activity(characters_ctx.active);
            let mut characters_pending: [Option<PendingAcquisition>; CHARACTER_PARTS] =
                [const { None }; CHARACTER_PARTS];
            for (part, pending) in characters_pending.iter_mut().enumerate() {
                let active = std::array::from_fn(|index| {
                    characters_ctx.active[index] && extended::characters::PART_MASKS[part][index]
                });
                if characters_refresh && active.iter().any(|&value| value) {
                    *pending = Some(queue!(acquisition::Job::Characters(
                        acquisition::CharactersRequest {
                            context: extended::characters::CharactersCopyContext {
                                active,
                                ..characters_ctx
                            },
                            payload: decode::acquire_characters_job(part),
                        }
                    )));
                }
            }
            let relations_ctx = extended::intel::RelationsCopyContext {
                base: shared.base,
                game_state: state,
                opinion_cap: frame.extended.relations_pilot().opinion_rows_cap(),
                tuple_cap: frame.extended.relations_pilot().tuple_rows_cap(),
                active: decode::RELATIONS
                    .map(|table| replay::cadence::should_walk(table, clock.hour, force_full)),
            };
            frame
                .extended
                .relations_pilot()
                .set_relation_activity(relations_ctx.active);
            let relations_pending = if relations_refresh {
                let payload = decode::acquire_relations_job();
                Some(queue!(acquisition::Job::Relations(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: relations_ctx,
                        payload,
                    }
                )))
            } else {
                None
            };
            let ordinary_request =
                frame
                    .extended
                    .take_ordinary_request(&country_snapshot, shared.base, state);
            let ordinary_bytes = ordinary_request.allocated_bytes();
            let ordinary_pending = queue!(acquisition::Job::Ordinary(ordinary_request));
            let slow_pending = queue!(acquisition::Job::Slow(frame.extended.take_slow_request(
                &country_snapshot,
                shared.base,
                state,
                clock.hour,
                force_full
            )));
            let mut supply_trains_pending: [Option<PendingAcquisition>; SUPPLY_PARTS] =
                [const { None }; SUPPLY_PARTS];
            for (part, pending) in supply_trains_pending.iter_mut().enumerate() {
                *pending = Some(queue!(acquisition::Job::SupplyTrains(
                    frame.extended.take_supply_trains_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        part
                    )
                )));
            }
            // Focus delegation must be settled before the complete Governance job moves.
            segment!(1);
            // The ship census is short and left first: join it here and send
            // its detail parts, which claim first, so the details (mostly the
            // recomputed maximum organisation) overlap the rest of the hour.
            let ship_models_completed =
                unsafe { finish_acquisition(shared, &mut batch, ship_models_pending) };
            let acquisition::Job::ShipModels(ship_models_request) = ship_models_completed.request
            else {
                unreachable!("ShipModels ticket type")
            };
            let ship_models_report = ship_models_completed.report.map(|report| {
                let acquisition::Report::ShipModels = report else {
                    unreachable!("ShipModels report type")
                };
            });
            frame.map.naval = frame
                .extended
                .restore_ship_models_request(ship_models_request, ship_models_report);
            // Written after the map-tick copy below, which covers these indices.
            let naval_ns = frame.map.naval.ticks.map(|cost| shared.ns(cost as i64));
            let ship_detail_pending = frame
                .extended
                .take_ship_model_parts(&country_snapshot, shared.base)
                .map(|request| {
                    request.map(|request| queue!(acquisition::Job::ShipModelDetails(request)))
                });
            let (world, world_report, world_failure) = completed!(world_pending, World);
            let production_active = world.production.is_some();
            let construction_active = world.construction.is_some();
            let focus_active = world.focus.as_ref().is_some_and(|part| part.context.active);
            production_rest_report = world_report.production;
            construction_rest_report = world_report.construction;
            economy_rest_report = world_report.economy;
            focus_report = world_report.focus;
            // An interrupted accepted job is final for this callback. Capacity
            // admission failures retain each family's existing typed fallback.
            let terminal = world_failure.is_some();
            frame.extended.production_rest_pilot().rest_raw_mode = production_active
                && (terminal
                    || production_rest_report.plan_failed
                    || production_rest_report.raw_ok);
            frame.extended.construction_rest_pilot().rest_raw_mode =
                construction_active && (terminal || construction_rest_report.raw_ok);
            frame.extended.economy_rest_pilot().rest_raw_mode =
                terminal || economy_rest_report.raw_ok;
            // An early-loaned family already holds its (false) delegation; the
            // pilot here would be its vacant placeholder.
            if early_governance.is_none() {
                frame.extended.focus_pilot().focus_raw_mode =
                    focus_active && (terminal || focus_report.raw_ok);
            }
            if let Some(part) = world.production {
                if production_rest_report.raw_ok && !terminal && !production_rest_report.plan_failed
                {
                    production_rest_payload = Some(part.payload);
                } else {
                    decode::recycle_production_rest_job(part.payload);
                }
            }

            if let Some(part) = world.construction {
                if construction_rest_report.raw_ok && !terminal {
                    construction_rest_payload = Some(part.payload);
                } else {
                    decode::recycle_construction_rest_job(part.payload);
                }
            }

            if economy_rest_report.raw_ok && !terminal {
                economy_rest_payload = Some(world.economy.payload);
            } else {
                decode::recycle_economy_rest_job(world.economy.payload);
            }

            if let Some(part) = world.focus {
                if focus_report.raw_ok && !terminal {
                    focus_payload = Some(part.payload);
                } else {
                    decode::recycle_focus_job(part.payload);
                }
            }

            if terminal {
                production_rest_report.raw_ok = false;
                construction_rest_report.raw_ok = false;
                economy_rest_report.raw_ok = false;
                focus_report.raw_ok = false;
            }
            // On the Focus slot the delegation is settled now; the family follows.
            let (governance_pending, governance_bytes) = match early_governance {
                Some(early) => early,
                None => {
                    let request = frame.extended.take_governance_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                    );
                    let bytes = request.allocated_bytes();
                    (queue!(acquisition::Job::Governance(request)), bytes)
                }
            };
            let presentation_request =
                frame
                    .extended
                    .take_presentation_request(&country_snapshot, shared.base, state);
            let presentation_bytes = presentation_request.allocated_bytes();
            let presentation_pending = queue!(acquisition::Job::Presentation(presentation_request));
            let eta_ranges = frame.extended.plan_eta_input_ranges();
            let mut eta_inputs_pending: [Option<PendingAcquisition>; ETA_INPUT_RANGES] =
                [const { None }; ETA_INPUT_RANGES];
            for (range, pending) in eta_inputs_pending.iter_mut().enumerate().take(eta_ranges) {
                *pending = Some(queue!(acquisition::Job::EtaInputs(
                    frame.extended.take_eta_inputs_request(
                        range,
                        &country_snapshot,
                        shared.base,
                        state
                    )
                )));
            }
            // Keep the original collection span. Accepted pool joins in this
            // span are already charged to join_ns; subtract those from
            // the inline subtotal, including callback helping inside a join.
            let inline_walk_begin = qpc();
            let inline_wait_begin = shared.acquisition_wait_ns.load(Ordering::Relaxed);
            segment!(2);
            segment!(3);
            let (air_request, report, air_failure) = completed!(air_pending, AirWing);
            air_wing_report = report;
            if air_wing_report.raw_ok && air_failure.is_none() {
                air_wing_payload = Some(air_request.payload);
            } else {
                decode::recycle_air_wing_job(air_request.payload);
            }
            let naval_ctx = extended::naval_history::NavalCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let mut naval_install_ticks = 0;
            if let Some(wings) = air_wing_payload.as_ref() {
                naval_install_ticks += naval_job.install_wings(wings);
            }
            segment!(4);
            let naval_pending = queue!(acquisition::Job::Naval(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: naval_ctx,
                payload: naval_job,
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
                    .payload
                    .supply_sources(&mut frame.extended, division_source_capacity);
                divisions_report.copy_ticks[0] += elapsed(source_started);
                divisions_payload = Some(request.payload);
                frame.extended.set_divisions_raw_mode(true);
                let ns = shared.ns(divisions_report.copy_ticks[0] as i64);
                frame.table_ns[6] = ns;
                decode::add_divisions_copy_ns(0, ns);
                decode::add_divisions_raw_bytes(0, divisions_report.raw_bytes[0]);
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
            let (request, report, failure) = completed!(players_pending, Players);
            players_report = report;
            let players_failure = failure;
            frame.map.players_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                players_payload = Some(request.payload);
            } else {
                decode::recycle_players_job(request.payload);
            }

            let mut air_stats_failure = None;
            if let Some(pending) = air_stats_pending {
                let (request, report, failure) = completed!(pending, AirStats);
                air_stats_report = report;
                air_stats_failure = failure;
                frame.extended.air_stats_pilot().stats_raw_mode =
                    report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    air_stats_payload = Some(request.payload);
                } else {
                    decode::recycle_air_stats_job(request.payload);
                }
            }

            let (request, report, failure) = completed!(air_bases_pending, AirBases);
            air_bases_report = report;
            let air_bases_failure = failure;
            frame.extended.air_bases_pilot().bases_raw_mode = report.raw_ok || failure.is_some();
            frame.extended.radar_pilot().registrations_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                air_bases_payload = Some(request.payload);
            } else {
                decode::recycle_air_bases_job(request.payload);
            }

            let mut air_history_failure = None;
            if let Some(pending) = air_history_pending {
                let (request, report, failure) = completed!(pending, AirHistory);
                air_history_report = report;
                air_history_failure = failure;
                frame.extended.air_history_pilot().history_raw_mode =
                    report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    air_history_payload = Some(request.payload);
                } else {
                    decode::recycle_air_history_job(request.payload);
                }
            }

            let mut supremacy_failure = None;
            if let Some(pending) = supremacy_pending {
                let (request, report, failure) = completed!(pending, Supremacy);
                supremacy_report = report;
                supremacy_failure = failure;
                frame.extended.supremacy_pilot().supremacy_raw_mode =
                    report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    supremacy_payload = Some(request.payload);
                } else {
                    decode::recycle_supremacy_job(request.payload);
                }
            }

            let mut sunk_failure = None;
            if let Some(pending) = sunk_pending {
                let (request, report, failure) = completed!(pending, Sunk);
                sunk_report = report;
                sunk_failure = failure;
                frame.extended.sunk_pilot().sunk_raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    sunk_payload = Some(request.payload);
                } else {
                    decode::recycle_sunk_job(request.payload);
                }
            }

            let mut radar_failure = None;
            if let Some(pending) = radar_pending {
                let (request, report, failure) = completed!(pending, Radar);
                radar_report = report;
                radar_failure = failure;
                frame.extended.radar_pilot().radar_raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    radar_payload = Some(request.payload);
                } else {
                    decode::recycle_radar_job(request.payload);
                }
            }

            let (request, report, failure) = completed!(weather_pending, Weather);
            weather_report = report;
            let weather_failure = failure;
            frame.extended.weather_pilot().raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                weather_payload = Some(request.payload);
            } else {
                decode::recycle_weather_job(request.payload);
            }

            let (request, report, failure) = completed!(capitulation_pending, Capitulation);
            capitulation_report = report;
            let capitulation_failure = failure;
            frame.extended.capitulation_pilot().capitulation_raw_mode =
                report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                capitulation_payload = Some(request.payload);
            } else {
                decode::recycle_capitulation_job(request.payload);
            }

            let mut projects_failure = None;
            if let Some(pending) = projects_pending {
                let (request, report, failure) = completed!(pending, Projects);
                projects_report = report;
                projects_failure = failure;
                frame.extended.projects_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    projects_payload = Some(request.payload);
                } else {
                    decode::recycle_projects_job(request.payload);
                }
            }
            let mut lexical_failure = None;
            if let Some(pending) = lexical_pending {
                let (request, report, failure) = completed!(pending, Lexical);
                lexical_report = report;
                lexical_failure = failure;
                frame.extended.lexical_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    lexical_payload = Some(request.payload);
                } else {
                    decode::recycle_lexical_job(request.payload);
                }
            }
            let mut session_failure = None;
            if let Some(pending) = session_pending {
                let (request, report, failure) = completed!(pending, Session);
                session_report = report;
                session_failure = failure;
                frame.extended.session_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    session_payload = Some(request.payload);
                } else {
                    decode::recycle_session_job(request.payload);
                }
            }

            let (request, report, failure) = completed!(intel_pending, Intel);
            intel_report = report;
            let intel_failure = failure;
            frame.extended.intel_pilot().intel_raw_mode = report.raw_ok || failure.is_some();
            if report.raw_ok && failure.is_none() {
                intel_payload = Some(request.payload);
            } else {
                decode::recycle_intel_job(request.payload);
            }

            // Release the shared seed leases before the Census can be
            // cleared on the decode worker during payload retirement.
            drop(request.inputs);
            segment!(5);
            // Resolve the character producer before dispatching its consumers.
            // The root owns this dependency; executor jobs never enqueue jobs.
            let mut characters_failure = [None; CHARACTER_PARTS];
            frame.extended.characters_pilot().raw_active = [false; 6];
            macro_rules! join_characters {
                ($part:expr) => {{
                    let part = $part;
                    if let Some(pending) = characters_pending[part].take() {
                        let (request, report, failure) = completed!(pending, Characters);
                        characters_report[part] = report;
                        let failure = failure.or(report
                            .plan_failed
                            .then_some("character prepared acquisition failed"));
                        characters_failure[part] = failure;
                        for (raw, active) in frame
                            .extended
                            .characters_pilot()
                            .raw_active
                            .iter_mut()
                            .zip(request.context.active)
                        {
                            *raw |= active && (report.raw_ok || failure.is_some());
                        }
                        if report.raw_ok && failure.is_none() {
                            characters_payload[part] = Some(request.payload);
                        } else {
                            decode::recycle_characters_job(request.payload);
                        }
                    }
                }};
            }
            join_characters!(0);
            let mut character_portraits_pending: [Option<PendingAcquisition>; PORTRAIT_PARTS] =
                [const { None }; PORTRAIT_PARTS];
            for (part, pending) in character_portraits_pending.iter_mut().enumerate() {
                let mut request =
                    frame
                        .extended
                        .take_character_portraits_request(shared.base, state, part);
                request.census = characters_payload[0]
                    .as_ref()
                    .and_then(|job| job.portrait_census());
                *pending = Some(queue!(acquisition::Job::CharacterPortraits(request)));
            }
            let gated_pending = queue!(acquisition::Job::Gated(frame.extended.take_gated_request(
                shared.base,
                state,
                clock.hour,
                force_full
            )));
            let (naval_request, report, naval_failure) = completed!(naval_pending, Naval);
            naval_report = report;
            if naval_report.raw_ok {
                naval_report.copy_ticks[0] += naval_install_ticks;
            }
            if let Some(wings) = air_wing_payload.as_ref() {
                naval_report.copy_ticks[0] += wings.membership_ticks;

                air_wing_report.copy_ticks[0] += frame.map.install_owned_air(wings);
                if !naval_report.raw_ok && naval_failure.is_none() {
                    frame
                        .extended
                        .naval_pilot()
                        .add_wing_source_ticks(naval_install_ticks);
                    frame.extended.naval_pilot().install_map_wings(wings);
                }
            }
            frame.extended.naval_pilot().raw_mode = naval_report.raw_ok || naval_failure.is_some();
            if naval_report.raw_ok && naval_failure.is_none() {
                naval_payload = Some(naval_request.payload);
            } else {
                decode::recycle_naval_job(naval_request.payload);
            }

            if let Some(reason) = air_failure {
                frame.map.fail_air_capture(reason);
            }
            segment!(6);
            let map_ticks = frame.map.sample(
                &cached,
                shared.base,
                state,
                &mut frame.extended.sources(
                    &cached,
                    shared.base,
                    air_wing_report.raw_ok || air_failure.is_some(),
                    naval_report.raw_ok || naval_failure.is_some(),
                ),
            );
            segment!(7);
            let (orders_request, _, orders_failure) = completed!(orders_pending, Orders);
            let acquired_bytes = orders_request.allocated_bytes();
            if acquired_bytes >= orders_bytes {
                shared
                    .pool_bytes
                    .fetch_add((acquired_bytes - orders_bytes) as u64, Ordering::Relaxed);
            } else {
                shared
                    .pool_bytes
                    .fetch_sub((orders_bytes - acquired_bytes) as u64, Ordering::Relaxed);
            }
            frame.extended.restore_orders_request(
                orders_request,
                orders_failure.is_none(),
                &mut frame.map.orders,
            );
            // Written after the map-tick copy below, which covers this index.
            let orders_ns = shared.ns(frame.map.orders.ticks as i64);
            let late_pending = queue!(acquisition::Job::Late(frame.extended.take_late_request(
                &mut frame.map.orders,
                naval_report.raw_ok || naval_failure.is_some(),
                shared.base,
                state,
                clock.hour,
                force_full
            )));
            let (countries_request, countries_report, _countries_failure) =
                completed!(countries_pending, Countries);
            countries_request.restore(&mut frame.countries, &mut frame.links);
            frame.countries_result = countries_report.countries;
            frame.table_ns[2] = shared.ns(countries_report.countries_ticks);
            frame.links_result = countries_report.links.result;
            frame.table_ns[3] = shared.ns(countries_report.links_ticks);
            let begin = qpc();
            let (provinces_request, provinces_report, _provinces_failure) =
                completed!(provinces_pending, Provinces);
            provinces_request.restore(&mut frame.provinces);
            frame.provinces_result = provinces_report.result;
            frame.table_ns[4] = shared.ns(qpc() - begin);
            let (states_request, states_report, states_failure) =
                completed!(states_pending, States);
            let states_report =
                frame.restore_states_request(states_request, states_report, states_failure);
            frame.table_ns[5] = shared.ns(states_report.ticks);
            let (wars_request, wars_report, wars_failure) = completed!(wars_pending, Wars);
            let wars_report = frame.restore_wars_request(wars_request, wars_report, wars_failure);
            frame.table_ns[7] = shared.ns(wars_report.table_ticks);
            let (factions_request, mut factions_report, factions_failure) =
                completed!(factions_pending, Factions);
            if let Some(reason) = factions_failure {
                factions_report = acquisition_factions::Report::failed(reason);
            }
            (frame.faction_names_len, frame.faction_names_required) = factions_request.restore(
                &mut frame.factions,
                &mut frame.faction_names,
                &mut frame.faction_members,
            );
            frame.factions_result = factions_report.factions.result;
            frame.table_ns[8] = shared.ns(factions_report.ticks[0] as i64);
            frame.faction_members_result = factions_report.members.result;
            frame.table_ns[9] = shared.ns(factions_report.ticks[1] as i64);
            inline_walk_ns = shared.ns(qpc() - inline_walk_begin).saturating_sub(
                shared
                    .acquisition_wait_ns
                    .load(Ordering::Relaxed)
                    .saturating_sub(inline_wait_begin),
            );
            for (total, ticks) in frame.table_ns[10..].iter_mut().zip(map_ticks) {
                *total = shared.ns(ticks as i64);
            }
            // Pool-walked map tables keep their own measured costs.
            for (table, ns) in [TableId::Fleets, TableId::TaskForces, TableId::Ships]
                .into_iter()
                .zip(naval_ns)
            {
                frame.table_ns[table.index()] = ns;
            }
            frame.table_ns[TableId::ArmyOrders.index()] = orders_ns;
            decode_hour = clock.hour;
            decode_force_full = force_full;
            if let Some(reason) = players_failure {
                frame.map.fail_players_capture(reason);
            }
            segment!(8);
            for pending in supply_trains_pending.into_iter().flatten() {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, pending) };
                let acquisition::Job::SupplyTrains(request) = completed.request else {
                    unreachable!("SupplyTrains ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::SupplyTrains(report) = report else {
                        unreachable!("SupplyTrains report type")
                    };
                    report
                });
                frame
                    .extended
                    .restore_supply_trains_request(request, report);
            }
            for pending in character_portraits_pending.into_iter().flatten() {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, pending) };
                let acquisition::Job::CharacterPortraits(request) = completed.request else {
                    unreachable!("CharacterPortraits ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::CharacterPortraits(report) = report else {
                        unreachable!("CharacterPortraits report type")
                    };
                    report
                });
                frame
                    .extended
                    .restore_character_portraits_request(request, report);
            }
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
            let mut relations_failure = None;
            if let Some(pending) = relations_pending {
                let (request, report, failure) = completed!(pending, Relations);
                relations_report = report;
                relations_failure = failure;
                frame.extended.relations_pilot().relations_raw_mode =
                    report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    relations_payload = Some(request.payload);
                } else {
                    decode::recycle_relations_job(request.payload);
                }
            }
            let (ordinary_request, _, ordinary_failure) = completed!(ordinary_pending, Ordinary);
            let acquired_bytes = ordinary_request.allocated_bytes();
            if acquired_bytes >= ordinary_bytes {
                shared
                    .pool_bytes
                    .fetch_add((acquired_bytes - ordinary_bytes) as u64, Ordering::Relaxed);
            } else {
                shared
                    .pool_bytes
                    .fetch_sub((ordinary_bytes - acquired_bytes) as u64, Ordering::Relaxed);
            }
            frame
                .extended
                .restore_ordinary_request(ordinary_request, ordinary_failure);
            let (slow_request, _, slow_failure) = completed!(slow_pending, Slow);
            frame
                .extended
                .restore_slow_request(slow_request, slow_failure.is_none());
            let (gated_request, _, gated_failure) = completed!(gated_pending, Gated);
            frame
                .extended
                .restore_gated_request(gated_request, gated_failure.is_none());
            let governance_completed =
                unsafe { finish_acquisition(shared, &mut batch, governance_pending) };
            let acquisition::Job::Governance(governance_request) = governance_completed.request
            else {
                unreachable!("Governance ticket type")
            };
            let acquired_bytes = governance_request.allocated_bytes();
            if acquired_bytes >= governance_bytes {
                shared.pool_bytes.fetch_add(
                    (acquired_bytes - governance_bytes) as u64,
                    Ordering::Relaxed,
                );
            } else {
                shared.pool_bytes.fetch_sub(
                    (governance_bytes - acquired_bytes) as u64,
                    Ordering::Relaxed,
                );
            }
            let governance_report = governance_completed.report.map(|report| {
                let acquisition::Report::Governance(report) = report else {
                    unreachable!("Governance report type")
                };
                report
            });
            frame
                .extended
                .restore_governance_request(governance_request, governance_report);
            // Research returns into the restored family and charges its costs there.
            let (research_request, research_report) = {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, research_pending) };
                let acquisition::Job::Research(request) = completed.request else {
                    unreachable!("Research ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::Research(report) = report else {
                        unreachable!("Research report type")
                    };
                    report
                });
                (request, report)
            };
            let fingerprints = frame
                .extended
                .restore_research_request(research_request, research_report);
            // SAFETY: only this callback takes and returns the store.
            unsafe {
                *shared.slice_fingerprints[crate::slicing::RESEARCH].get() = Some(fingerprints)
            };
            // Only the core roster feeds portraits. Independent role and
            // trait jobs need their verdicts here, before the typed fallback;
            // they must not delay dispatch of unrelated map/gated/late work.
            for part in 1..CHARACTER_PARTS {
                join_characters!(part);
            }
            segment!(9);
            let mut ticks =
                frame
                    .extended
                    .capture_tables(&cached, shared.base, state, clock.hour, force_full);
            segment!(10);
            let presentation_completed =
                unsafe { finish_acquisition(shared, &mut batch, presentation_pending) };
            let acquisition::Job::Presentation(presentation_request) =
                presentation_completed.request
            else {
                unreachable!("Presentation ticket type")
            };
            let acquired_bytes = presentation_request.allocated_bytes();
            if acquired_bytes >= presentation_bytes {
                shared.pool_bytes.fetch_add(
                    (acquired_bytes - presentation_bytes) as u64,
                    Ordering::Relaxed,
                );
            } else {
                shared.pool_bytes.fetch_sub(
                    (presentation_bytes - acquired_bytes) as u64,
                    Ordering::Relaxed,
                );
            }
            let presentation_report = presentation_completed.report.map(|report| {
                let acquisition::Report::Presentation(report) = report else {
                    unreachable!("Presentation report type")
                };
                report
            });
            frame
                .extended
                .restore_presentation_request(presentation_request, presentation_report);
            for pending in eta_inputs_pending.into_iter().flatten() {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, pending) };
                let acquisition::Job::EtaInputs(request) = completed.request else {
                    unreachable!("EtaInputs ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::EtaInputs(report) = report else {
                        unreachable!("EtaInputs report type")
                    };
                    report
                });
                frame.extended.restore_eta_inputs_request(request, report);
            }
            segment!(11);
            let (late_request, _, late_failure) = completed!(late_pending, Late);
            frame.extended.restore_late_request(
                late_request,
                late_failure.is_none(),
                &mut frame.map.orders,
            );
            frame.extended.capture_tables_late(
                &cached,
                shared.base,
                state,
                clock.hour,
                force_full,
                &mut frame.map.orders,
                &mut ticks,
            );
            segment!(12);
            shared.record_eta_inputs(frame.extended.eta_input_counters());
            if production_rest_report.plan_failed {
                frame
                    .extended
                    .production_rest_pilot()
                    .fail_production_rest_capture(
                        "production acquisition plan could not complete safely",
                    );
                for (table, cost) in decode::PRODUCTION_REST
                    .into_iter()
                    .zip(production_rest_report.copy_ticks)
                {
                    ticks[table.index()] = cost;
                }
            }
            if let Some(reason) = world_failure {
                if production_active {
                    frame
                        .extended
                        .production_rest_pilot()
                        .fail_production_rest_capture(reason);
                }
                if construction_active {
                    frame
                        .extended
                        .construction_rest_pilot()
                        .fail_construction_rest_capture(reason);
                }
                frame
                    .extended
                    .economy_rest_pilot()
                    .fail_economy_rest_capture(reason);
                if focus_active {
                    frame
                        .extended
                        .focus_pilot()
                        .fail_focus_capture(focus_active, reason);
                }
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
                    ticks[table.index()] = cost;
                }
            }
            for (part, failure) in characters_failure.into_iter().enumerate() {
                if let Some(reason) = failure {
                    let active = std::array::from_fn(|index| {
                        characters_ctx.active[index]
                            && extended::characters::PART_MASKS[part][index]
                    });
                    frame
                        .extended
                        .characters_pilot()
                        .fail_character_tables(active, reason);
                }
            }
            if let Some(reason) = relations_failure {
                frame
                    .extended
                    .relations_pilot()
                    .fail_relations_capture(relations_ctx.active, reason);
            }
            if let Some(reason) = air_bases_failure {
                frame
                    .extended
                    .air_bases_pilot()
                    .fail_air_bases_capture(reason);
            }
            if let Some(reason) = air_history_failure {
                frame
                    .extended
                    .air_history_pilot()
                    .fail_air_history_capture(reason);
            }
            if let Some(reason) = supremacy_failure {
                frame
                    .extended
                    .supremacy_pilot()
                    .fail_supremacy_capture(reason);
            }
            if let Some(reason) = sunk_failure {
                frame.extended.sunk_pilot().fail_sunk_capture(reason);
            }
            if let Some(reason) = radar_failure {
                frame.extended.radar_pilot().fail_radar_capture(reason);
            }
            if let Some(reason) = intel_failure {
                frame.extended.intel_pilot().fail_intel_capture(reason);
            }
            if let Some(reason) = capitulation_failure {
                frame
                    .extended
                    .capitulation_pilot()
                    .fail_capitulation_capture(reason);
            }
            if let Some(reason) = weather_failure {
                frame.extended.weather_pilot().fail_weather_capture(reason);
            }
            if let Some(reason) = projects_failure {
                frame
                    .extended
                    .projects_pilot()
                    .fail_projects_capture(projects_ctx.active, reason);
            }
            if let Some(reason) = lexical_failure {
                frame.extended.lexical_pilot().fail_lexical_capture(reason);
            }
            if let Some(reason) = session_failure {
                frame.extended.session_pilot().fail_session_capture(reason);
            }
            if let Some(reason) = air_bases_failure {
                frame
                    .extended
                    .radar_pilot()
                    .fail_registrations_capture(reason);
            }
            if let Some(reason) = air_failure {
                frame
                    .extended
                    .air_wing_pilot()
                    .fail_air_wing_capture(reason);
            }
            if let Some(reason) = air_stats_failure {
                frame
                    .extended
                    .air_stats_pilot()
                    .fail_air_stats_capture(reason);
            }
            if let Some(reason) = naval_failure {
                frame.extended.naval_pilot().fail_naval_capture(reason);
            }
            if let Some(reason) = divisions_failure {
                frame.divisions_result = Outcome::Unavailable { reason };
                if divisions_ctx.details_active {
                    frame.extended.fail_division_details(reason);
                }
            }
            // Restore the final independent request before releasing the lease.
            segment!(13);
            // Join the last-dispatched parts first: any still queued then run
            // inline here instead of parking. Part 0, the family's own capture,
            // is restored first.
            let mut ship_detail_completed: [Option<
                acquisition::Completion<acquisition::Job, acquisition::Report>,
            >; extended::ship_models::MAX_PARTS] =
                [const { None }; extended::ship_models::MAX_PARTS];
            for (pending, completed) in ship_detail_pending
                .into_iter()
                .zip(ship_detail_completed.iter_mut())
                .rev()
            {
                if let Some(pending) = pending {
                    *completed = Some(unsafe { finish_acquisition(shared, &mut batch, pending) });
                }
            }
            for completed in ship_detail_completed.into_iter().flatten() {
                let acquisition::Job::ShipModelDetails(request) = completed.request else {
                    unreachable!("ShipModelDetails ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::ShipModelDetails(report) = report else {
                        unreachable!("ShipModelDetails report type")
                    };
                    report
                });
                frame.extended.restore_ship_model_part(request, report);
            }
            frame.extended.finish_ship_model_parts();
            let ships = replay::rows::TableId::ShipModels.index();
            ticks[ships] = frame.extended.capture_ship_models(&cached, shared.base);
            segment!(14);
            ts_parallel = segment_prev;
            // Join wall spent on the pool jobs during the parallel phase.
            // acquisition_wait_ns is written only by this thread's joins.
            join_ns = shared
                .acquisition_wait_ns
                .load(Ordering::Relaxed)
                .saturating_sub(wait_at_setup);
            // Every consumer has joined. Release all input leases before
            // returning a failed payload to its decode-side pool.
            drop(batch);
            drop(inputs);
            frame.extended.release_unit_inputs();
            if divisions_failure.is_some() {
                if let Some(job) = divisions_payload.take() {
                    decode::recycle_divisions_job(job);
                }
                divisions_report.raw_ok = false;
            }
            for table in extended::TABLES {
                let index = table.index();
                let walk = replay::cadence::should_walk(table, clock.hour, force_full);
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
                    let ns = shared.ns(relations_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                    decode::add_relations_copy_ns(slot, ns);
                    decode::add_relations_raw_bytes(slot, relations_report.raw_bytes[slot]);
                }
            }
            // Independent jobs charge their own anchor; these worker times
            // overlap and must not be interpreted as callback wall time.
            for (part, report) in characters_report.iter().enumerate() {
                if !report.raw_ok {
                    continue;
                }
                for (slot, table) in decode::CHARACTERS.iter().enumerate() {
                    if !extended::characters::PART_MASKS[part][slot] {
                        continue;
                    }
                    let index = table.index();
                    let ns = shared.ns(report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                    decode::add_characters_copy_ns(slot, ns);
                    decode::add_characters_raw_bytes(slot, report.raw_bytes[slot]);
                }
            }
            // Projects tables measured copy-only above; same install.
            if projects_report.raw_ok {
                for (slot, table) in decode::PROJECTS.iter().enumerate() {
                    let index = table.index();
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
                    let ns = shared.ns(air_stats_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                    decode::add_air_stats_copy_ns(slot, ns);
                    decode::add_air_stats_raw_bytes(slot, air_stats_report.raw_bytes[slot]);
                }
            }
            if naval_report.raw_ok {
                for (slot, table) in decode::NAVAL_ORDER.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(naval_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                    decode::add_naval_copy_ns(slot, ns);
                    decode::add_naval_raw_bytes(slot, naval_report.raw_bytes[slot]);
                }
            }
            // Players pair measured copy-only above; same install for the
            // calibration counters. Each table retains its own timing slot.
            if players_report.raw_ok {
                for (slot, table) in decode::PLAYERS.iter().enumerate() {
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
                    let ns = shared.ns(intel_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                    decode::add_intel_copy_ns(slot, ns);
                    decode::add_intel_raw_bytes(slot, intel_report.raw_bytes[slot]);
                }
            }
            // Hourly weather measured copy-only above; same install for
            // the calibration counters. The manager header is
            // anchor-charged to the regions slot, matching the read-group
            // convention.
            if weather_report.raw_ok {
                for (slot, table) in decode::WEATHER.iter().enumerate() {
                    let index = table.index();
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
            frame.country_cache_ns = shared.ns(cache_prepare_ticks);
            // All dependent tables share the one seeded authentication census.
            frame.table_ns[2] += frame.country_cache_ns;
        }
        unsafe {
            shared.sample_roster(&memory, frame.clock);
        }
        frame.table_ns[0] = shared.ns(clock_ticks);
        frame.table_reads = crate::read_accounting::snapshot();
        // Keep the producer's loss witnesses until ownership is published.
        // A contained panic before enqueue must not consume earlier gaps.
        frame.dropped_before = unsafe { *shared.pending.get() };
        frame.pool_bytes = shared.pool_bytes.load(Ordering::Relaxed);
        frame.dropped_frames = shared.dropped.load(Ordering::Relaxed);
        shared.completed[index].store(0, Ordering::Relaxed);
        shared.callback_tail[index].store(0, Ordering::Relaxed);
        // Finish every frame write before publishing its index. The decode
        // worker may acquire exclusive access as soon as enqueue succeeds;
        // its later ready-queue Release cannot protect writes after enqueue.
        // Every frame transits the single decode FIFO in hour order; the
        // worker hands it to the spool ready queue after decoding any
        // refresh payload. Each queue can hold the complete buffer pool,
        // so owning a buffer guarantees space for its index.
        // The frame's cost is this acquisition body's wall on the game thread.
        let game_elapsed = (qpc() - dispatched).max(1);
        // Disjoint block phases: setup+snapshot, parallel (dispatch/inline/joins),
        // then residual work and enqueue.
        shared
            .phase_setup_ns
            .fetch_add(shared.ns(ts_setup - dispatched), Ordering::Relaxed);
        shared
            .phase_parallel_ns
            .fetch_add(shared.ns(ts_parallel - ts_setup), Ordering::Relaxed);
        shared.phase_residual_ns.fetch_add(
            shared.ns(dispatched + game_elapsed - ts_parallel),
            Ordering::Relaxed,
        );
        shared
            .phase_inline_walk_ns
            .fetch_add(inline_walk_ns, Ordering::Relaxed);
        shared.phase_join_ns.fetch_add(join_ns, Ordering::Relaxed);
        shared.phase_calls.fetch_add(1, Ordering::Relaxed);
        frame.capture_seq = decode::next_seq();
        let decode_job = decode::DecodeJob {
            index,
            seq: frame.capture_seq,
            hour: decode_hour,
            relations_refresh: relations_report.raw_ok,
            characters_refresh: characters_report.map(|report| report.raw_ok),
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
            None => unsafe { *shared.pending.get() = LossBatch::default() },
            Some(returned) => {
                if let Some(payload) = returned.relations_payload {
                    decode::recycle_relations_job(payload);
                }
                for payload in returned.characters_payload.into_iter().flatten() {
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
                drop_owned_frame(shared, frame, index);
            }
        }
    } else {
        unsafe {
            shared.sample_roster(&memory, entry);
            (&mut *shared.pending.get()).push(entry);
        }
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

// Keep a rejected request inline: boxing it would allocate on the game thread.
#[allow(clippy::large_enum_variant)]
enum PendingAcquisition {
    Accepted(acquisition::Ticket),
    Rejected(acquisition::Job),
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
    let pending = match unsafe { batch.submit(&mut retained) } {
        Ok(ticket) => PendingAcquisition::Accepted(ticket),
        Err(_) => PendingAcquisition::Rejected(retained.expect("rejected request retained")),
    };
    shared
        .acquisition_dispatch_ns
        .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
    pending
}

/// Accepted work is recovered once; only rejected submissions may execute inline.
unsafe fn finish_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    pending: PendingAcquisition,
) -> acquisition::Completion<acquisition::Job, acquisition::Report> {
    match pending {
        PendingAcquisition::Accepted(ticket) => {
            let begin = qpc();
            let result = batch.finish(ticket).expect("submitted acquisition job");
            let wait = shared.ns(qpc() - begin);
            shared
                .acquisition_wait_ns
                .fetch_add(wait, Ordering::Relaxed);
            shared.join_wait[result.request.role() as usize].fetch_add(wait, Ordering::Relaxed);
            #[cfg(feature = "callback_profile")]
            crate::callback_profile::record_join(
                result.request.role() as usize,
                wait,
                result.profile,
            );
            result
        }
        PendingAcquisition::Rejected(mut request) => {
            if let acquisition::Job::Governance(governance) = &mut request {
                let report = governance.fail("governance worker admission failed");
                return acquisition::Completion {
                    request,
                    report: Ok(acquisition::Report::Governance(report)),
                    table_reads: crate::read_accounting::ZERO,
                    #[cfg(feature = "callback_profile")]
                    profile: Default::default(),
                };
            }
            if let acquisition::Job::Orders(orders) = &mut request {
                let report = orders.fail("army order worker admission failed");
                return acquisition::Completion {
                    request,
                    report: Ok(acquisition::Report::Orders(report)),
                    table_reads: crate::read_accounting::ZERO,
                    #[cfg(feature = "callback_profile")]
                    profile: Default::default(),
                };
            }
            if let acquisition::Job::Ordinary(ordinary) = &mut request {
                let report = ordinary.fail("ordinary worker admission failed");
                return acquisition::Completion {
                    request,
                    report: Ok(acquisition::Report::Ordinary(report)),
                    table_reads: crate::read_accounting::ZERO,
                    #[cfg(feature = "callback_profile")]
                    profile: Default::default(),
                };
            }
            if let acquisition::Job::Presentation(presentation) = &mut request {
                let report = presentation.fail("division presentation worker admission failed");
                return acquisition::Completion {
                    request,
                    report: Ok(acquisition::Report::Presentation(report)),
                    table_reads: crate::read_accounting::ZERO,
                    #[cfg(feature = "callback_profile")]
                    profile: Default::default(),
                };
            }
            // SAFETY: this request was rejected before acquiring any live state.
            unsafe { batch.run_inline(request) }
        }
    }
}

fn acquisition_failure_reason(failure: acquisition::Failure) -> &'static str {
    match failure {
        acquisition::Failure::Panicked => "acquisition worker panicked",
        acquisition::Failure::Stopped => "acquisition worker stopped before completion",
    }
}

pub(crate) fn acquire_world_request(
    countries: &Arc<Cache>,
    base: usize,
    game_state: usize,
    focus: extended::governance::FocusCopyContext,
    economy: &mut extended::economy::Captured,
    hour: i32,
    force_full: bool,
) -> acquisition::WorldRequest {
    acquisition::WorldRequest {
        countries: Arc::clone(countries),
        production: replay::cadence::should_walk(TableId::ProductionLines, hour, force_full).then(
            || acquisition::WorldPart {
                context: extended::production::ProductionRestCopyContext { base, game_state },
                payload: decode::acquire_production_rest_job(),
            },
        ),
        construction: replay::cadence::should_walk(TableId::StateBaseResources, hour, force_full)
            .then(|| acquisition::WorldPart {
                context: extended::construction::ConstructionRestCopyContext { base, game_state },
                payload: decode::acquire_construction_rest_job(),
            }),
        economy: acquisition::WorldPart {
            context: economy.resource_copy_context(base, game_state, hour, force_full),
            payload: decode::acquire_economy_rest_job(),
        },
        focus: replay::cadence::should_walk(TableId::Focus, hour, force_full).then(|| {
            acquisition::WorldPart {
                context: focus,
                payload: decode::acquire_focus_job(),
            }
        }),
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
        let census = crate::division_census::Census::new();
        let mut job = Self {
            details: crate::division_detail::OwnedDetails::new(census.source_capacity()),
            census,
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
        self.details.grow(self.census.source_capacity());
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
    fn supply_sources(&self, captured: &mut extended::Captured, capacity: usize) {
        supply_division_census_sources(&self.census, captured, capacity);
    }
}

fn supply_division_census_sources(
    census: &crate::division_census::Census,
    captured: &mut extended::Captured,
    capacity: usize,
) {
    let (sources, outcome) = census.sources(capacity);
    for &source in sources {
        captured.visit_division_source(source);
    }
    captured.finish_divisions(outcome);
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DivisionsCopyReport {
    /// Ready owned payload. A core failure after seed leasing is terminal;
    /// only an input-stage shortage before leasing may use a fresh fallback.
    pub raw_ok: bool,
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
    /// Frozen with the input lineage and retained through owned decoding.
    pub details_active: bool,
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
            details_active: true,
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
    let _reads = crate::read_accounting::table(replay::rows::TableId::Divisions);
    job.clear();
    job.context = context;
    if !context.details_active {
        job.details.mark_skipped();
    }
    let started = qpc();
    job.census
        .capture_inputs(memory, context.base, context.game_state);
    job.acquisition_ticks[0] = elapsed(started);
    job.inputs_captured = true;
    // This is metadata-only capacity admission. capture_core may still discover
    // a private row/header capacity shortage, which cannot authorize a retry
    // after consumers receive a lease.
    job.overflowed = (context.details_active && !job.details.ready_for(context.detail_capacities))
        || job.census.overflowed;
}

/// Retain one input packet before any core task leases it. A capacity miss
/// permits fallback only before any consumer has borrowed that packet.
pub(crate) fn copy_divisions_inputs_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> bool {
    copy_divisions_inputs_pass(payload, memory, ctx);
    !payload.overflowed
}

/// Consume only the retained input lineage. This stage never clears a Census,
/// reacquires its input packet, or retries an earlier core/detail acquisition.
fn copy_divisions_core_pass(
    job: &mut DivisionsJob,
    memory: &impl engine::Memory,
    context: DivisionsCopyContext,
) {
    let _reads = crate::read_accounting::table(replay::rows::TableId::Divisions);
    // Context mismatch is a terminal orchestration failure. Keep the original
    // input owner intact so its other consumers can finish.
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
    job.overflowed = (context.details_active && !job.details.ready_for(context.detail_capacities))
        || job.census.overflowed;
    if job.overflowed {
        return;
    }
    if !context.details_active {
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

/// Report the one owned acquisition without reading source memory.
fn divisions_copy_report(payload: &DivisionsJob) -> DivisionsCopyReport {
    DivisionsCopyReport {
        raw_ok: !payload.overflowed,
        copy_ticks: payload.acquisition_ticks,
        raw_bytes: payload.payload_bytes(),
    }
}

/// A failure after input leasing is final for this callback; no source retry.
pub(crate) fn copy_divisions_core_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

/// Single composed acquisition for synthetic callers.
#[cfg(test)]
pub(crate) fn copy_divisions_refresh(
    memory: &impl engine::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_inputs_pass(payload, memory, ctx);
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

/// Worker installation copies compact core rows and transfers owned detail
/// buffers. Callback presentation rows and their source packet survive.
pub(crate) fn decode_divisions_payload(
    frame: &mut Captured,
    _base: usize,
    job: &mut DivisionsJob,
    decode_ticks: &mut [u64; 6],
) {
    let started = qpc();
    frame.divisions_result = job.census.install_rows(&mut frame.divisions);
    decode_ticks[0] += elapsed(started);
    if job.context.details_active {
        let started = qpc();
        frame.extended.install_division_detail(&mut job.details);
        job.update_capacity_charge();
        decode_ticks[1] += elapsed(started);
    } else {
        frame.extended.set_divisions_raw_mode(false);
        job.update_capacity_charge();
    }
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

    pub(crate) struct Capture {
        pub owned_outcome: Outcome,
        pub owned: Vec<cwar_relation::Row>,
    }

    /// Reference primary walk with identical pooled buffers.
    pub(crate) fn direct(memory: &impl Memory, len: usize) -> Capture {
        let mut owned = vec![cwar_relation::Row::default(); len];
        let owned_outcome = cwar_relation::walk(memory, BASE, GAME_STATE, &mut owned);
        Capture {
            owned_outcome,
            owned,
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

    /// One owned acquisition installed into a pooled frame with equal capacities.
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
            details_active: true,
        };
        let mut first = DivisionsJob::new();
        let report = super::copy_divisions_refresh(memory, ctx, &mut first);
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
        assert_eq!(
            &split.groups[..direct.groups.len()],
            direct.groups,
            "groups rows"
        );
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
        assert!(
            job.details
                .ready_for([0, job.census.source_capacity(), 0, 0, 0])
        );
        let mut frame = Captured::new();
        let frame_before = frame.allocated_bytes();
        let payload_before = expected(&job);
        decode_divisions_payload(&mut frame, 0, &mut job, &mut [0; 6]);
        let payload_after = expected(&job);
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, payload_after);
        assert_eq!(
            frame.allocated_bytes() as u64 + payload_after - frame_before as u64 - payload_before,
            (job.census.source_capacity() - extended::production::Capacities::default().rows[7])
                as u64
                * size_of::<engine::carmy_upgrades_requests::Row>() as u64
        );
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, payload_after);
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
    fn decode_queue_bound_matches_pool_invariant() {
        let shared = Shared::new(0, 0, 1);
        assert_eq!(shared.decode_queue.capacity(), BUFFERS);
        assert_eq!(shared.free.capacity(), BUFFERS);
        assert_eq!(shared.ready.capacity(), BUFFERS);
    }

    #[test]
    fn a_dropped_frame_invalidates_its_sliced_reads_and_a_clockless_one_does_not() {
        use crate::slicing::{RESEARCH, SliceSelection};
        let shared = Shared::new(0, 0, 1);
        // SAFETY: test thread only; nothing else holds the store.
        let store = || {
            unsafe { &mut *shared.slice_fingerprints[RESEARCH].get() }
                .as_mut()
                .unwrap()
        };
        let owners = [(7, Some(1)), (9, Some(1))];
        let early_at = |hour: i32| {
            let mut next = SliceSelection::for_test(hour, false, &[]);
            for (owner, value) in owners {
                next.scan(store(), owner, value);
            }
            store().finish_scan();
            [7, 9].map(|owner| next.contains(owner))
        };
        early_at(0);
        assert_eq!(early_at(1), [false, false]);
        for (clock, expected) in [(Some(clock(7, 1)), [true, false]), (None, [false, false])] {
            let index = shared.free.pop().expect("pooled buffer");
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.extended.prepare_hour(7, false);
            frame.clock = clock;
            drop_owned_frame(&shared, frame, index);
            // Hour 7's slice (owner 7) was not published: read it again at hour 8.
            assert_eq!(early_at(8), expected, "clock {:?}", clock.map(|c| c.hour));
        }
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
                    characters_refresh: [false; CHARACTER_PARTS],
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
                    characters_payload: [const { None }; CHARACTER_PARTS],
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
        // A decode FIFO filled with duplicate leases is an invariant violation.
        // Keep its explicit failure path rather than losing an acquired frame silently.
        let extra = decode::DecodeJob {
            index,
            seq: u64::MAX,
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
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
            characters_payload: [const { None }; CHARACTER_PARTS],
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
        frame.clock = Some(clock(42, 7));
        let dropped_before = shared.dropped.load(Ordering::Relaxed);
        drop_owned_frame(&shared, frame, index);
        assert_eq!(shared.dropped.load(Ordering::Relaxed), dropped_before + 1);
        assert_eq!(shared.ready.len(), 0);
        assert_eq!(shared.free.len(), BUFFERS);
        let pending = unsafe { &*shared.pending.get() };
        let runs: Vec<_> = pending.iter().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].count),
            (42, 42, 1)
        );
        assert_eq!(runs[0].loaded_hour, Some(42));
    }

    #[test]
    fn pool_initialization_allocates_all_slots_and_counts_actual_bytes() {
        let shared = Shared::new(0, 0, 1);
        assert_eq!(shared.free.capacity(), BUFFERS);
        assert_eq!(shared.ready.capacity(), BUFFERS);
        assert_eq!(shared.free.len(), BUFFERS);
        let mut bytes = unsafe { (&*shared.roster.get()).as_ref().unwrap() }.allocated_bytes();
        bytes += unsafe { &*shared.country_cache.get() }.allocated_bytes();
        // One CountryIntel buffer for the whole pool.
        bytes += shared.country_intel.allocated_bytes();
        for slot in &shared.frames {
            let frame = slot.load(Ordering::Acquire);
            assert!(!frame.is_null());
            let frame = unsafe { &*frame };
            let home = frame.extended.country_intel_home().unwrap();
            assert!(home.shares(&shared.country_intel));
            bytes += frame.allocated_bytes();
        }
        assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), bytes as u64);
    }

    #[test]
    fn exhausted_pool_reuses_a_returned_frame_without_allocating_or_dropping() {
        use std::{sync::mpsc, time::Duration};
        let shared = Arc::new(Shared::new(0, 0, 1));
        let held: Vec<_> = (0..BUFFERS).map(|_| shared.take_frame().unwrap()).collect();
        let original = shared.frames[held[0]].load(Ordering::Acquire);
        unsafe {
            (*original).qpc = 42;
        }
        let bytes = shared.pool_bytes.load(Ordering::Relaxed);
        let (send, receive) = mpsc::channel();
        let consumer = Arc::clone(&shared);
        let waiter = std::thread::spawn(move || {
            let result = consumer.take_frame();
            send.send(result).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while shared.buffer_wait_calls.load(Ordering::Relaxed) == 0
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        let waited = shared.buffer_wait_calls.load(Ordering::Relaxed);
        let pending = receive.try_recv().is_err();
        shared.recycle_frame(held[0]);
        let result = receive.recv_timeout(Duration::from_secs(2));
        shared.cancel_buffer_wait.store(true, Ordering::Release);
        waiter.join().unwrap();
        assert_eq!(waited, 1);
        assert!(pending);
        assert_eq!(result.unwrap(), Some(held[0]));
        assert_eq!(shared.frames[held[0]].load(Ordering::Acquire), original);
        assert_eq!(unsafe { (*original).qpc }, 42);
        assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), bytes);
        assert_eq!(shared.dropped.load(Ordering::Relaxed), 0);
        for index in held {
            shared.recycle_frame(index);
        }
        assert_eq!(shared.free.len(), BUFFERS);
    }

    #[test]
    fn exhausted_pool_wait_cancels_without_a_live_consumer() {
        use std::{sync::mpsc, time::Duration};
        let shared = Arc::new(Shared::new(0, 0, 1));
        let held: Vec<_> = (0..BUFFERS).map(|_| shared.take_frame().unwrap()).collect();
        let (send, receive) = mpsc::channel();
        let consumer = Arc::clone(&shared);
        let waiter = std::thread::spawn(move || {
            send.send(consumer.take_frame()).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while shared.buffer_wait_calls.load(Ordering::Relaxed) == 0
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        shared.cancel_buffer_wait.store(true, Ordering::Release);
        let result = receive.recv_timeout(Duration::from_secs(2));
        waiter.join().unwrap();
        assert_eq!(result.unwrap(), None);
        for index in held {
            shared.recycle_frame(index);
        }
        // Cancellation does not discard a frame that is already available.
        assert!(shared.take_frame().is_some());
    }

    #[test]
    fn acquisition_panic_stops_admission_and_preserves_losses_without_waiting() {
        use std::sync::atomic::AtomicUsize;

        struct PanicOnDrop;
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("panic payload must not be dropped across the callback boundary");
            }
        }

        let shared = Shared::new(0, 0, 1);
        let index = shared.take_frame().unwrap();
        unsafe { (&mut *shared.pending.get()).push(Some(clock(40, 7))) };
        let acquisitions = AtomicUsize::new(0);
        let completed = unsafe {
            guarded_acquisition(&shared, Some(clock(41, 7)), || {
                acquisitions.fetch_add(1, Ordering::Relaxed);
                std::panic::panic_any(PanicOnDrop);
            })
        };
        assert!(!completed);
        assert!(shared.acquisition_failed());
        assert!(shared.cancel_buffer_wait.load(Ordering::Acquire));
        assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
        assert_eq!(shared.free.len(), BUFFERS - 1);
        assert_eq!(shared.take_frame(), None);
        assert_eq!(shared.buffer_wait_calls.load(Ordering::Relaxed), 0);

        // During a delayed hook restore, record the next hour as a loss without
        // running acquisition again or consuming another capture buffer.
        assert!(!unsafe {
            guarded_acquisition(&shared, Some(clock(42, 7)), || {
                acquisitions.fetch_add(1, Ordering::Relaxed);
            })
        });
        assert_eq!(acquisitions.load(Ordering::Relaxed), 1);
        assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 1);
        assert_eq!(shared.dropped.load(Ordering::Relaxed), 2);
        let losses = unsafe { &*shared.pending.get() };
        let run = losses.iter().next().unwrap();
        assert_eq!((run.first_hour, run.last_hour, run.count), (40, 42, 3));

        // Cancellation also works when no healthy frame remains. Do not
        // reclaim the partially restored frame as if acquisition succeeded.
        while shared.free.pop().is_some() {}
        assert_eq!(shared.take_frame(), None);
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 0);
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
        batch.push(Some(clock(10, 1)));
        batch.push(Some(clock(11, 1)));
        batch.push(Some(clock(5, 2)));
        batch.push(Some(clock(8, 2)));
        let runs: Vec<_> = batch.iter().collect();
        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].count),
            (10, 11, 2)
        );
        assert_eq!(runs[1].loaded_hour, Some(5));
        batch.push(None);
        assert_eq!(batch.unknown_count, 1);
    }

    #[test]
    fn end_of_hour_losses_record_the_entry_hour_as_the_lost_and_loaded_hour() {
        assert_eq!(SAMPLE_BOUNDARY, replay::identity::SampleBoundary::EndOfHour);
        let mut batch = LossBatch::default();
        batch.push(Some(clock(20, 3)));
        batch.push(Some(clock(21, 3)));
        let runs: Vec<_> = batch.iter().collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour, runs[0].loaded_hour),
            (20, 21, Some(20))
        );
    }

    #[test]
    fn capacity_growth_keeps_primary_rows_and_name_arena_sized() {
        let mut frame = Captured::new();
        let before = frame.allocated_bytes();
        let mut capacities = INITIAL_CAPACITIES;
        frame.wars_result = Outcome::Truncated {
            len: 1024,
            unread_rows: 7,
        };
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
        assert_eq!(frame.wars.len(), 2048);
        assert_eq!(frame.factions.len(), 128);
        assert_eq!(frame.faction_members.len(), 2048);
        assert_eq!(frame.faction_names.len(), 32768);
        assert!(frame.allocated_bytes() > before);
        let mut idle = Captured::new();
        idle.grow(&mut capacities);
        assert_eq!(idle.allocated_bytes(), frame.allocated_bytes());
    }

    #[test]
    fn fallback_census_growth_consumes_the_failed_attempt_once() {
        use super::divisions_test_kit as kit;
        let bytes = kit::fixture();
        let mut frame = Captured::new();
        frame.division_fallback = crate::division_census::Census::default();
        frame
            .division_fallback
            .capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert!(frame.division_fallback.overflowed);
        let mut capacities = INITIAL_CAPACITIES;
        frame.grow(&mut capacities);
        assert!(frame.division_fallback.initialized());
        assert!(!frame.division_fallback.overflowed);
        let grown = frame.division_fallback.allocated_bytes();
        assert!(grown > 0);

        // A subsequent frame that never used the fallback carries no new
        // overflow request and must retain, rather than double, its storage.
        frame.grow(&mut capacities);
        assert_eq!(frame.division_fallback.allocated_bytes(), grown);

        frame
            .division_fallback
            .capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE, 64);
        assert_eq!(
            frame.division_fallback.outcome(64),
            Outcome::Rows { len: 3 },
        );
        let lease = Arc::clone(frame.division_fallback.unit_inputs().unwrap());
        frame.extended.install_unit_inputs(Some(Arc::clone(&lease)));
        assert_eq!(Arc::strong_count(&lease), 3);
        frame.extended.release_unit_inputs();
        assert_eq!(Arc::strong_count(&lease), 2);
        drop(lease);
        frame.grow(&mut capacities);
        assert_eq!(frame.division_fallback.allocated_bytes(), grown);
    }

    fn cadence_clock(hour: i32, signal: i64) -> Clock {
        clock(hour, signal)
    }

    #[test]
    fn cadence_rotation_walks_recorded_families_once_per_24_hours() {
        use replay::cadence::{Cadence, cadence, is_refresh};
        use replay::rows::TableId;
        for base in [i32::MIN, -25, -1, 0, 1, 60_820_531, i32::MAX - 23] {
            let mut refreshes = [0; replay::frame::TABLE_COUNT];
            for offset in 0..24 {
                let hour: i32 = base + offset;
                let slot = hour.rem_euclid(24);
                for table in TableId::ALL {
                    let expected = match cadence(table) {
                        // A sliced table reads one owner slice every hour.
                        Cadence::Hourly | Cadence::Sliced => true,
                        Cadence::FourHourly(s) => s as i32 == hour.rem_euclid(4),
                        Cadence::Daily(s) => s as i32 == slot,
                        Cadence::Static => false,
                    };
                    assert_eq!(is_refresh(table, hour), expected, "{table:?} at {hour}");
                    let walk = replay::cadence::should_walk(table, hour, false);
                    assert_eq!(walk, expected, "{table:?} walk at {hour}");
                    assert!(
                        replay::cadence::should_walk(table, hour, true),
                        "{table:?} forced at {hour}"
                    );
                    refreshes[table.index()] += usize::from(walk);
                }
            }
            for table in TableId::ALL {
                let expected = match cadence(table) {
                    Cadence::Hourly | Cadence::Sliced => 24,
                    Cadence::FourHourly(_) => 6,
                    Cadence::Daily(_) => 1,
                    Cadence::Static => 0,
                };
                assert_eq!(refreshes[table.index()], expected, "{table:?} near {base}");
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
    fn periodic_keyframe_keeps_observation_continuous_but_load_gap_and_campaign_do_not() {
        let mut cadence = CadenceState::default();
        assert!(!cadence.continuous(&cadence_clock(100, 1)));
        cadence.force_full(&cadence_clock(100, 1));
        for hour in 101..=820 {
            let clock = cadence_clock(hour, 1);
            assert!(cadence.continuous(&clock));
            assert_eq!(cadence.force_full(&clock), hour == 820);
        }
        assert!(!cadence.continuous(&cadence_clock(822, 1)));
        assert!(!cadence.continuous(&cadence_clock(821, 2)));
        let mut changed = cadence_clock(821, 1);
        changed.campaign_id.bytes[0] ^= 1;
        assert!(!cadence.continuous(&changed));
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
    fn wars_typed_walk_preserves_primary_native_rows() {
        use super::wars_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let split = kit::direct(&memory, 64);
        // Spot-checks: the owned walk emits each war once from its first
        // endpoint, retaining native signed values.
        assert_eq!(split.owned_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.owned[0].first_country, 1);
        assert_eq!(split.owned[0].second_country, 2);
        assert_eq!(split.owned[0].start_hour, i32::MIN);
        assert_eq!(split.owned[0].first_casualties, i64::MAX);
        assert_eq!(split.owned[1].first_country, 3);
        assert_eq!(split.owned[1].second_country, 1);
        assert_eq!(split.owned[1].first_casualties, 11);
    }

    #[test]
    fn wars_typed_capacities_keep_full_validation_and_exact_unread_counts() {
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
        }
    }

    #[test]
    fn wars_primary_faults_preserve_native_validation_scope() {
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
        // The primary census validates each used link and ignores unrelated slots.
        for (fault, complete) in [
            (kit::GAME_STATE + cgame_state::COUNTRIES, false),
            (kit::GAME_STATE + cgame_state::COUNTRY_LINKS, false),
            (kit::CTAB + 8, false),
            (kit::LTAB + 4, false),
            (kit::C0, false),
            (kit::C0 + ccountry::ID, false),
            (kit::C0 + ccountry::DIPLOMACY_STATUS, false),
            (kit::D0, false),
            (kit::WAR1 + crelation::TYPE_ID, false),
            (kit::WAR1 + crelation::START_HOUR, false),
            (kit::D0 + cdiplomacy_status::OWNED_RELATIONS, false),
            (kit::O0 + 8, false),
            (kit::OTHER + crelation::TYPE_ID, false),
            (kit::RETIRED + crelation::TYPE_ID, false),
            (kit::D0 + cdiplomacy_status::RELATIONS, false),
            (kit::M0 + 8, true),
            (kit::status(1, 2), true),
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
                matches!(direct.owned_outcome, Outcome::Rows { .. }),
                complete,
                "fault={fault:x}",
            );
        }
    }

    #[test]
    fn divisions_core_is_hourly_and_five_details_refresh_on_daily_slot_two() {
        use replay::cadence::{is_refresh, refresh_slot, should_walk};

        for table in [
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            TableId::DivisionEtaInputs,
        ] {
            assert_eq!(refresh_slot(table), None, "{table:?}");
            for hour in [-25, -22, -1, 0, 1, 2, 3, 26, 60_820_531] {
                assert!(is_refresh(table, hour), "{table:?} at {hour}");
                assert!(should_walk(table, hour, true));
            }
        }
        for table in crate::decode::DIVISIONS.into_iter().skip(1) {
            assert_eq!(refresh_slot(table), Some(2), "{table:?}");
            for hour in -48i32..48 {
                let expected = hour.rem_euclid(24) == 2;
                assert_eq!(is_refresh(table, hour), expected, "{table:?} at {hour}");
                assert_eq!(should_walk(table, hour, false), expected);
                assert!(should_walk(table, hour, true));
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
        // exactly, including each independent table fault.
        // Shared census failures block every dependent table.
        let group = 0x60070usize;
        for (fault, complete) in [
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
            assert_eq!(
                [
                    split.divisions_outcome,
                    split.equipment_outcome,
                    split.groups_outcome,
                    split.requests_outcome,
                    split.deliveries_outcome,
                    split.produced_outcome,
                ]
                .map(|outcome| matches!(outcome, Outcome::Rows { .. })),
                complete,
                "fault={fault:x}"
            );
        }
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
        let report = copy_divisions_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        assert_eq!(first.source_outcome(64), Outcome::Rows { len: 3 });
        let (first_sources, _) = first.census.sources(64);
        assert_eq!(first_sources[0].division, 101);
        assert_eq!(first_sources[0].template, 0x3455_0000_0034);
        assert_eq!(first_sources[0].country, 1);
        let mut frame = Captured::new();
        first.supply_sources(&mut frame.extended, frame.divisions.len());
        frame
            .extended
            .install_unit_inputs(first.census.unit_inputs().map(Arc::clone));
        frame.extended.set_divisions_raw_mode(true);
        let ticks = frame
            .extended
            .capture(&memory, 0, kit::GAME_STATE, 0, false);
        for table in [TableId::DivisionMovement, TableId::DivisionStatus] {
            assert!(ticks[table.index()] > 0);
            assert_eq!(
                frame.extended.outcomes()[table.index()],
                Outcome::Rows { len: 3 }
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
            let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
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
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
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
        copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
        assert_eq!(first.source_outcome(0), Outcome::Rows { len: 0 });
        assert!(first.census.sources(64).0.is_empty()); // Recycled rows were cleared.
        first.census = crate::division_census::Census::default();
        let bytes = kit::fixture();
        let report = copy_divisions_refresh(&kit::memory(&bytes), ctx, &mut first);
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
    }

    #[test]
    fn divisions_seeded_worker_preserves_primary_trace_and_never_repeats_core() {
        use super::divisions_test_kit as kit;
        use crate::read_accounting;
        use replay::rows::TableId;
        struct Traced<'a> {
            bytes: &'a [u8],
            calls: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl engine::Memory for Traced<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                kit::memory(self.bytes).copy(address, output)
            }
        }
        struct NoRead;
        impl engine::Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("completed core repeated a read");
            }
        }
        let bytes = kit::fixture();
        let capture = read_accounting::Capture::new();
        let memory = CountedMemory::new(Traced {
            bytes: &bytes,
            calls: Default::default(),
        });
        let composed_memory = Traced {
            bytes: &bytes,
            calls: Default::default(),
        };
        let ctx = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            ..Default::default()
        };
        let mut payload = DivisionsJob::new();
        assert!(copy_divisions_inputs_refresh(&memory, ctx, &mut payload));
        let seed_reads = read_accounting::snapshot();
        assert!(seed_reads[TableId::Divisions.index()].bytes > 0);
        let inputs = Arc::clone(payload.census.unit_inputs().unwrap());
        let report = copy_divisions_core_refresh(&memory, ctx, &mut payload);
        assert!(report.raw_ok);
        assert!(Arc::ptr_eq(&inputs, payload.census.unit_inputs().unwrap()));
        let accounted = read_accounting::snapshot();
        let repeated = copy_divisions_core_refresh(&NoRead, ctx, &mut payload);
        assert_eq!(repeated.raw_bytes, report.raw_bytes);
        assert_eq!(repeated.copy_ticks, report.copy_ticks);
        assert_eq!(read_accounting::snapshot(), accounted);
        let mut composed = DivisionsJob::new();
        let original = copy_divisions_refresh(&composed_memory, ctx, &mut composed);
        assert_eq!(report.raw_bytes, original.raw_bytes);
        assert_eq!(
            payload.census.rows(ctx.row_capacity),
            composed.census.rows(ctx.row_capacity)
        );
        assert_eq!(*memory.0.calls.borrow(), *composed_memory.calls.borrow());
        assert_eq!(
            memory
                .0
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == kit::A1 + carmy::STRENGTH)
                .count(),
            1
        );
        drop(inputs);
        let mut actual = Captured::new();
        let mut expected = Captured::new();
        decode_divisions_payload(&mut actual, kit::BASE, &mut payload, &mut [0; 6]);
        decode_divisions_payload(&mut expected, kit::BASE, &mut composed, &mut [0; 6]);
        assert_eq!(actual.divisions, expected.divisions);
        assert_eq!(
            actual.extended.test_division_detail().tables(),
            expected.extended.test_division_detail().tables()
        );
        let mut total = ReadCounts::ZERO;
        for (index, counts) in accounted.iter().enumerate() {
            total += *counts;
            if [
                TableId::Divisions,
                TableId::DivisionEquipment,
                TableId::ArmyUpgradeGroups,
            ]
            .iter()
            .any(|table| table.index() == index)
            {
                assert!(counts.spans > 0 && counts.bytes > 0);
            } else {
                assert_eq!(*counts, ReadCounts::ZERO);
            }
        }
        let trace = memory.0.calls.borrow();
        assert_eq!(
            total.spans,
            trace.iter().filter(|(_, len)| *len != 0).count() as u64
        );
        assert_eq!(
            total.bytes,
            trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
        );
        assert_eq!(capture.finish(), accounted);
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
        ));
        let lease = Arc::clone(job.census.unit_inputs().unwrap());
        let before = job.payload_bytes();
        let changed = DivisionsCopyContext {
            row_capacity: 0,
            ..context
        };
        assert!(!copy_divisions_core_refresh(&NoRead, changed, &mut job).raw_ok);
        assert_eq!(job.payload_bytes(), before);
        assert!(Arc::ptr_eq(&lease, job.census.unit_inputs().unwrap()));
        assert!(!copy_divisions_core_refresh(&NoRead, context, &mut job).raw_ok);
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
        let report = copy_divisions_refresh(&memory, ctx, &mut first);
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
    fn inactive_division_details_skip_capacity_reads_and_decode_installation() {
        use super::divisions_test_kit as kit;
        struct Traced<'a> {
            bytes: &'a [u8],
            calls: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl engine::Memory for Traced<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                kit::memory(self.bytes).copy(address, output)
            }
        }
        let bytes = kit::fixture();
        let memory = CountedMemory::new(Traced {
            bytes: &bytes,
            calls: Default::default(),
        });
        let reference = Traced {
            bytes: &bytes,
            calls: Default::default(),
        };
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_capacity: 64,
            source_capacity: 64,
            detail_capacities: [usize::MAX; 5],
            details_active: false,
        };
        let mut job = DivisionsJob {
            census: crate::division_census::Census::new(),
            ..Default::default()
        };
        let reads = crate::read_accounting::Capture::new();
        assert!(copy_divisions_inputs_refresh(&memory, context, &mut job));
        let lease = Arc::clone(job.census.unit_inputs().unwrap());
        let report = copy_divisions_core_refresh(&memory, context, &mut job);
        assert!(report.raw_ok);
        assert!(Arc::ptr_eq(&lease, job.census.unit_inputs().unwrap()));
        assert_eq!(job.details.allocated_bytes(), 0);
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        assert_eq!(job.details.outcomes(), [skipped; 5]);
        assert_eq!(report.copy_ticks[1..], [0; 5]);
        assert_eq!(report.raw_bytes[1..], [0; 5]);
        let counts = reads.finish();
        assert!(counts[TableId::Divisions.index()].spans > 0);
        for (index, counts) in counts.iter().enumerate() {
            if index != TableId::Divisions.index() {
                assert_eq!(*counts, ReadCounts::ZERO);
            }
        }
        let mut census = crate::division_census::Census::new();
        census.capture_inputs(&reference, context.base, context.game_state);
        census.capture_core(&reference, context.row_capacity);
        assert_eq!(*memory.0.calls.borrow(), *reference.calls.borrow());
        assert_eq!(job.census.rows(64), census.rows(64));
        drop(lease);

        let mut frame = Captured::new();
        frame
            .extended
            .fail_division_details(replay::cadence::SKIPPED_REASON);
        frame.extended.set_divisions_raw_mode(true);
        let detail = frame.extended.test_division_detail();
        let pointer = detail.division_equipment.rows.as_ptr();
        let capacity = detail.division_equipment.rows.len();
        let mut ticks = [0; 6];
        decode_divisions_payload(&mut frame, context.base, &mut job, &mut ticks);
        assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
        let detail = frame.extended.test_division_detail();
        assert_eq!(detail.division_equipment.rows.as_ptr(), pointer);
        assert_eq!(detail.division_equipment.rows.len(), capacity);
        assert_eq!(detail.division_equipment.outcome, skipped);
        assert!(!detail.divisions_raw_mode);
        assert_eq!(ticks[1..], [0; 5]);
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
        let report = copy_divisions_refresh(&memory, ctx, &mut first);
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
            kit::assert_matches_direct(&split, &direct);
        }
        let (exact, _, _) = kit::split(&memory, [3; 6]);
        assert_eq!(exact.divisions_outcome, Outcome::Rows { len: 3 });
        for groups in [0, 1] {
            let mut caps = [3; 6];
            caps[2] = groups;
            let direct = kit::direct(&memory, [3; 6]);
            let (split, report, _) = kit::split(&memory, caps);
            assert!(report.raw_ok);
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(split.groups_outcome, Outcome::Rows { len: 3 });
        }
    }
}
