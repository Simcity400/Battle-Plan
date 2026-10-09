//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use std::sync::Arc;
mod divisions;
pub(crate) use divisions::{
    DivisionsCopyContext, DivisionsCopyReport, DivisionsJob, copy_divisions_core_refresh,
    copy_divisions_inputs_refresh, decode_divisions_payload,
};
#[cfg(test)]
pub(crate) use divisions::{copy_divisions_refresh, divisions_test_kit};
use divisions::{fail_divisions_capture, supply_division_census_sources};

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
    Memory, carmy, ccountry, cfaction, cfaction_system,
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
        Condvar, Mutex, OnceLock,
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
const AIR_RELATION_PARTS: usize = extended::air_relations::PARTS;
const AIR_HISTORY_PARTS: usize = extended::air_histories::PARTS;
const PORTRAIT_PARTS: usize = extended::character_portraits::PARTS;
pub const SEGMENTS: usize = 15;
pub const SEGMENTS_SCOPE: &str = "game-thread wall of the parallel phase split at fixed marks: 0 first dispatch burst (ShipModels census, World, Research, off-slot Governance first, slot-hour Variants); 1 division inputs copy + divisions/intel dispatch + character roster, relations, ordinary, slow and supply parts; 2 ShipModels census join + detail part dispatch, World join + Focus-slot governance/presentation/ETA dispatch; 3 unused; 4 AirWing join + wing install; 5 mid joins (divisions..intel) + Naval dispatch; 6 character roster join + portrait/Gated/character role dispatch + Naval join + owned wing install + AirMissions dispatch; 7 map sample; 8 AirMissions dispatch after a typed wing census + Orders and AirMissions joins + Late and history-part dispatch + core joins; 9 supply/portraits/order/relations/ordinary/slow/gated/governance/variants/research/character role joins; 10 capture_tables; 11 presentation + ETA joins; 12 Late and history-part joins + capture_tables_late; 13 visibility + failure handling; 14 ShipModels detail part joins";
/// Core row capacities: countries, provinces, links, states, divisions, wars,
/// factions, faction members, faction name bytes, border-conflict provinces,
/// their admitted group handles, and state name bytes.
pub const INITIAL_CAPACITIES: [usize; 12] = [
    512, 20_000, 1024, 2048, 4096, 1024, 64, 1024, 16_384, 64, 256, 32_768,
];

/// Countries has one row per canonical slot and CountryLinks one per raw
/// link id; FactionMembers holds each country at most once. Order counters and
/// the war snapshot follow the Countries capacity when the frame grows.
pub fn raise_for_countries(core: &mut [usize; 12], slots: usize) {
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
    prev_world: u64,
}

impl CadenceState {
    fn continuous(&self, clock: &Clock) -> bool {
        self.initialized
            && self.prev_hour.checked_add(1) == Some(clock.hour)
            && self.prev_signal == clock.play_session_started_at
            && self.prev_campaign == clock.campaign_id
            && self.prev_world == crate::world_lifetime::generation()
    }
    // New branches never open on stale data: first, signal, campaign and
    // discontinuity force a full refresh. A periodic encoding keyframe does
    // not: the spool builds it from the rows it already holds for every
    // staggered and sliced table, so it adds no native reads.
    fn force_full(&mut self, clock: &Clock) -> bool {
        if !self.initialized {
            self.initialized = true;
            self.prev_hour = clock.hour;
            self.prev_signal = clock.play_session_started_at;
            self.prev_campaign = clock.campaign_id;
            self.prev_world = crate::world_lifetime::generation();
            return true;
        }
        let signal = self.prev_signal != clock.play_session_started_at;
        let campaign = self.prev_campaign != clock.campaign_id;
        let gap = self.prev_hour.checked_add(1) != Some(clock.hour);
        let world = self.prev_world != crate::world_lifetime::generation();
        let force = signal || campaign || gap || world;
        self.prev_hour = clock.hour;
        self.prev_signal = clock.play_session_started_at;
        self.prev_campaign = clock.campaign_id;
        self.prev_world = crate::world_lifetime::generation();
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
            rows: engine::allocation::owned_vec(engine::allocation::default_vec::<
                ccountry::OrderInstanceCounter,
            >(capacity)),
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
            crate::table_capture::grow_vec(&mut self.rows, required);
        }
    }
}

pub struct Captured {
    pub world_generation: u64,
    /// This clocked hour's fixed-layout check, applied during conversion.
    pub fixed_layout: Option<crate::fixed_layout::Layout>,
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
    /// Provinces rows and the BorderConflicts rows of the same walk.
    pub provinces: cprovince::Capture,
    pub states: Vec<cstate::Row>,
    /// Both display names of every state row, spanned by `cstate::Row`.
    pub state_names: Vec<u8>,
    pub state_names_len: usize,
    pub state_names_required: usize,
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
        .unwrap()
    }

    pub(crate) fn with_capacities(
        capacities: &[usize; 12],
        map_capacities: &MapCapacities,
        extended_capacities: &extended::Capacities,
    ) -> std::io::Result<Self> {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        let mut division_fallback = crate::division_census::Census::new();
        division_fallback.reserve_sources(capacities[4].max(extended_capacities.division_sources));
        let frame = Self {
            world_generation: crate::world_lifetime::generation(),
            capture_seq: 0,
            fixed_layout: None,
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
            countries: engine::allocation::default_vec::<ccountry::Row>(capacities[0])?,
            provinces: cprovince::Capture::try_with_capacity(
                capacities[1],
                capacities[9],
                capacities[10],
            )?,
            states: engine::allocation::default_vec::<cstate::Row>(capacities[3])?,
            state_names: engine::allocation::default_vec(capacities[11])?,
            state_names_len: 0,
            state_names_required: 0,
            divisions: engine::allocation::default_vec::<carmy::Row>(capacities[4])?,
            division_fallback,
            wars: engine::allocation::default_vec::<cwar_relation::Row>(capacities[5])?,
            wars_snapshot: cwar_relation::OwnedSnapshotScratch::try_with_capacity(capacities[0])?,
            wars_snapshot_sampled: false,
            factions: engine::allocation::default_vec::<cfaction::Row>(capacities[6])?,
            faction_names: engine::allocation::default_vec(capacities[8])?,
            faction_names_len: 0,
            faction_names_required: 0,
            faction_members: engine::allocation::default_vec::<cfaction_system::MemberRow>(
                capacities[7],
            )?,
            links: engine::allocation::default_vec::<ccountry::LinkRow>(capacities[2])?,
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
        };
        if crate::allocation::failed() {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        Ok(frame)
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.order_counters.allocated_bytes()
            + self.map.allocated_bytes()
            + self.extended.allocated_bytes()
            + self.wars_snapshot.allocated_bytes()
            + self.division_fallback.allocated_bytes() as usize
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + self.provinces.allocated_bytes()
            + self.states.capacity() * size_of::<cstate::Row>()
            + self.state_names.capacity()
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
            names: acquisition_states::Names::take(&mut self.state_names),
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
        (self.state_names_len, self.state_names_required) =
            request.names.restore(&mut self.state_names);
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
    pub fn grow(&mut self, capacities: &mut [usize; 12]) {
        fn resize<T: Default + Clone>(rows: &mut Vec<T>, outcome: Outcome, capacity: &mut usize) {
            if let Outcome::Truncated { len, unread_rows } = outcome {
                grow_capacity(capacity, len.saturating_add(unread_rows as usize));
            }
            if rows.len() < *capacity {
                crate::table_capture::grow_vec(rows, *capacity);
            }
        }
        resize(
            &mut self.countries,
            self.countries_result,
            &mut capacities[0],
        );
        resize(
            &mut self.provinces.rows,
            self.provinces_result,
            &mut capacities[1],
        );
        let conflicts = self.provinces.conflict_walk;
        resize(
            &mut self.provinces.conflicts,
            conflicts.outcome,
            &mut capacities[9],
        );
        grow_capacity(&mut capacities[10], conflicts.required_groups);
        if self.provinces.groups.len() < capacities[10] {
            crate::table_capture::grow_vec(&mut self.provinces.groups, capacities[10]);
        }
        resize(&mut self.links, self.links_result, &mut capacities[2]);
        self.order_counters.grow(capacities[0]);
        if self.wars_snapshot.grow(capacities[0]).is_err() {
            return;
        }
        resize(&mut self.states, self.states_result, &mut capacities[3]);
        resize(
            &mut self.divisions,
            self.divisions_result,
            &mut capacities[4],
        );
        let census = &mut self.division_fallback;
        if census.overflowed || census.counts_need_growth() {
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
        crate::table_capture::grow_vec(&mut self.faction_names, capacities[8]);
        grow_capacity(&mut capacities[11], self.state_names_required);
        crate::table_capture::grow_vec(&mut self.state_names, capacities[11]);
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

/// Cumulative DivisionTemplateIcons rows whose model inputs failed, by
/// reason, since attach. Fixed size: folding an hour in allocates nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct TemplateModelStats {
    /// Hours with at least one failed row.
    pub hours: u64,
    pub rows: u64,
    pub first_reason: Option<&'static str>,
    pub reasons: [(&'static str, u64); extended::division_templates::MODEL_REASONS],
    pub other: u64,
}

impl TemplateModelStats {
    fn fold(&mut self, hour: extended::division_templates::ModelFailures) {
        if hour.rows == 0 {
            return;
        }
        self.hours += 1;
        self.rows += u64::from(hour.rows);
        self.other += u64::from(hour.other);
        for (reason, rows) in hour.reasons.into_iter().filter(|(_, rows)| *rows != 0) {
            self.first_reason.get_or_insert(reason);
            match self
                .reasons
                .iter_mut()
                .find(|slot| slot.1 == 0 || slot.0 == reason)
            {
                Some(slot) => *slot = (reason, slot.1 + u64::from(rows)),
                None => self.other += u64::from(rows),
            }
        }
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
    /// Immutable attachment setting; disabling adds no score operand reads.
    pub game_score_enabled: bool,
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
    /// One store per sliced family (`slicing::FAMILIES`), loaned to that
    /// family's job each hour. Only the game thread takes and returns them.
    slice_fingerprints: [UnsafeCell<Option<Box<crate::slicing::SliceFingerprints>>>;
        crate::slicing::FAMILIES.len()],
    /// Fresh country authentication for this quiescent callback. Game thread only.
    country_cache: UnsafeCell<Arc<Cache>>,
    eta_cadence:
        [UnsafeCell<Option<Box<extended::division_eta_inputs::CadenceStore>>>; ETA_INPUT_RANGES],
    eta_cadence_bytes: [AtomicU64; ETA_INPUT_RANGES],
    /// The held RaidAirScores table, loaned to the Late job each hour. Only
    /// the game thread takes and returns it.
    raid_air_scores: UnsafeCell<Option<Box<engine::raid_air_scores::Held>>>,
    raid_air_score_bytes: AtomicU64,
    pub eta_inputs: EtaInputStats,
    /// Template rows whose model inputs failed; the status thread copies it.
    pub template_models: Mutex<TemplateModelStats>,
    /// Hours whose failures were not folded because the status thread held
    /// `template_models`: the game thread never waits for it.
    pub template_models_missed: AtomicU64,
    /// Set by the spool when a staggered table truncated or a static table
    /// failed: the next hour walks everything again, with grown buffers.
    pub force_next: AtomicBool,
    /// Requested acquisition refresh; it does not revoke observer continuity.
    pub refresh_next: AtomicBool,
    /// Sliced families (bit per [`crate::slicing::FAMILIES`] index) whose
    /// walked read the spool could not publish; the next hour forgets their
    /// fingerprint stores, so every owner is read again.
    pub forget_families: std::sync::atomic::AtomicU32,
    /// The lexer's maximum token id at the last dictionary walk; a change
    /// means tokens were appended and the static dictionary is walked again.
    pub lexical_max: AtomicI32,
    /// The attachment's first consistent fixed-layout check.
    pub fixed_layout: OnceLock<engine::fixed_layout::Report>,
    /// The modifier definition table's data pointer and row count at the last
    /// hour; a change means a loader registered keys, and the static
    /// ModifierDefinitions table is walked again. `u64::MAX` before the first
    /// observation; an unreadable header is `(0, u64::MAX)`.
    pub modifier_table: [AtomicU64; 2],
    /// The unit and reference id counters when the last daily
    /// `DivisionEquipment` walk ran, packed; see `new_divisions::next_mark`.
    pub division_mark: AtomicU64,
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
        .expect("synthetic capture buffers")
    }

    /// Every frame and the country cache are allocated from `sizing`, which
    /// the loader derives from the live country registry before hooking.
    pub fn with_sizing(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
    ) -> std::io::Result<Self> {
        let roster = Box::new(Roster::with_counts(sizing.content));
        let country_cache = Arc::new(Cache::try_with_capacity(sizing.country_cells)?);
        let country_cells = country_cache.capacity();
        // Frame 0 allocates the CountryIntel buffer; the others borrow it.
        let first = Captured::with_capacities(&sizing.core, &sizing.map, &sizing.extended)?;
        if crate::allocation::failed() {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        let country_intel = first
            .extended
            .country_intel_home()
            .expect("a new frame holds its intel pack");
        let air = &sizing.extended.air;
        let mut raid_air_scores = Box::new(engine::raid_air_scores::Held::default());
        raid_air_scores.grow(air.scores.regions, air.scores.countries, air.scores.rows)?;
        let raid_air_score_bytes = raid_air_scores.allocated_bytes();
        let pool_bytes = roster.allocated_bytes()
            + country_cache.allocated_bytes()
            + country_intel.allocated_bytes()
            + raid_air_score_bytes;
        let mut slice_fingerprints = std::array::from_fn(|_| UnsafeCell::new(None));
        for (family, cell) in slice_fingerprints.iter_mut().enumerate() {
            if !matches!(
                family,
                crate::slicing::AIR_RELATIONS | crate::slicing::AIR_HISTORIES
            ) {
                *cell.get_mut() = Some(Box::new(crate::slicing::SliceFingerprints::new()));
            }
        }
        let mut eta_cadence = std::array::from_fn(|_| UnsafeCell::new(None));
        for cell in &mut eta_cadence {
            *cell.get_mut() = Some(Box::new(
                extended::division_eta_inputs::CadenceStore::default(),
            ));
        }
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
            game_score_enabled: sizing.extended.game_score.enabled,
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
            // The air families' stores belong to their decode steps.
            slice_fingerprints,
            country_cache: UnsafeCell::new(country_cache),
            eta_cadence,
            eta_cadence_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            raid_air_scores: UnsafeCell::new(Some(raid_air_scores)),
            raid_air_score_bytes: AtomicU64::new(raid_air_score_bytes as u64),
            eta_inputs: EtaInputStats::default(),
            template_models: Mutex::new(TemplateModelStats::default()),
            template_models_missed: AtomicU64::new(0),
            force_next: AtomicBool::new(false),
            refresh_next: AtomicBool::new(false),
            forget_families: std::sync::atomic::AtomicU32::new(0),
            lexical_max: AtomicI32::new(-1),
            fixed_layout: OnceLock::new(),
            modifier_table: [AtomicU64::new(u64::MAX), AtomicU64::new(u64::MAX)],
            division_mark: AtomicU64::new(extended::new_divisions::UNKNOWN),
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
            let mut frame = Captured::with_capacities(&sizing.core, &sizing.map, &borrowed)?;
            frame.extended.share_country_intel(&shared.country_intel);
            if crate::allocation::failed() {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            unsafe { shared.publish_frame(index, Box::new(frame)) };
        }
        Ok(shared)
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
            let Ok(next) = Cache::try_with_capacity(cells).map(Arc::new) else {
                return;
            };
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
        crate::allocation::failed() || self.acquisition_panics.load(Ordering::Acquire) != 0
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

/// Forget the fingerprint stores of every family the spool could not publish.
fn forget_unpublished_families(shared: &Shared) {
    let families = shared.forget_families.swap(0, Ordering::AcqRel);
    if families & (1 << crate::slicing::COMBAT_LOG) != 0 {
        extended::combat_log::forget_reads();
    }
    for (family, store) in shared.slice_fingerprints.iter().enumerate() {
        if families & (1 << family) != 0 {
            // SAFETY: game thread only, between callbacks' loans.
            if let Some(store) = unsafe { &mut *store.get() }.as_mut() {
                store.forget();
            }
        }
    }
}

/// Take a sliced family's fingerprint store for this hour's job.
///
/// # Safety
/// Game thread only, and the previous callback returned the store.
unsafe fn take_slice_store(
    shared: &Shared,
    family: usize,
) -> Box<crate::slicing::SliceFingerprints> {
    unsafe { &mut *shared.slice_fingerprints[family].get() }
        .take()
        .expect("sliced family store returned by the previous callback")
}

/// Return a store [`take_slice_store`] lent.
///
/// # Safety
/// Game thread only, in the callback that took it.
unsafe fn return_slice_store(
    shared: &Shared,
    family: usize,
    store: Box<crate::slicing::SliceFingerprints>,
) {
    unsafe { *shared.slice_fingerprints[family].get() = Some(store) };
}

fn lost_observer_interval() {
    crate::equipment_destruction::lost_drain();
    crate::naval_result_sources::lost_drain();
    crate::experience_gains::lost_drain();
    crate::event_feed_observer::lost_drain();
    crate::operation_outcomes::lost_drain();
    crate::politics_changes::invalidate();
    crate::land_events::fail();
    crate::air_losses::lost_drain();
    crate::building_bombing::lost_drain();
}

/// Restore an owned frame's absorbed witnesses, record its lost hour and return
/// the buffer. Shared by every queue-full branch; no new loss reasons.
fn drop_owned_frame(shared: &Shared, frame: &mut Captured, index: usize) {
    frame.extended.discard_event_feed();
    lost_observer_interval();
    // The dropped hour's sliced reads are never published: read them again.
    // A clockless frame read nothing, and its selections belong to an older hour.
    let read = frame.clock.is_some();
    if read {
        extended::combat_log::forget_reads();
    }
    frame.extended.drop_air_copies();
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
    world_generation: u64,
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
    if crate::allocation::failed() {
        shared.cancel_buffer_wait.store(true, Ordering::Release);
        shared.recording.store(false, Ordering::Release);
        return false;
    }
    if !shared.acquisition_failed() {
        match catch_unwind(AssertUnwindSafe(acquire)) {
            Ok(()) if !crate::allocation::failed() => return true,
            Ok(()) => {
                shared.cancel_buffer_wait.store(true, Ordering::Release);
                shared.recording.store(false, Ordering::Release);
                // The fixed witness names this admitted hour. Enqueue failure
                // may already have returned its loss; never add it a second time.
                return false;
            }
            Err(payload) => {
                // A panic payload can itself panic on Drop. At most one is
                // retained because this attachment now stops acquisition.
                std::mem::forget(payload);
                shared.cancel_buffer_wait.store(true, Ordering::Release);
                shared.acquisition_panics.fetch_add(1, Ordering::Release);
            }
        }
    }
    lost_observer_interval();
    unsafe { (&mut *shared.pending.get()).push(entry) };
    shared.dropped.fetch_add(1, Ordering::Relaxed);
    false
}

#[cfg(test)]
pub(crate) unsafe fn synthetic_callback(
    shared: &Shared,
    entry: Option<Clock>,
    acquire: impl FnOnce(),
    original: impl FnOnce(),
) -> bool {
    let _phase = crate::allocation::enter(crate::allocation::Phase::Callback);
    crate::allocation::set_context(crate::allocation::Context {
        campaign_id: entry.and_then(|clock| clock.campaign_id.as_bytes().try_into().ok()),
        branch: None,
        load_signal: entry.map(|clock| clock.play_session_started_at),
        failed_hour: entry.map(|clock| clock.hour),
    });
    let complete = unsafe { guarded_acquisition(shared, entry, acquire) };
    original();
    crate::allocation::attempted_hour(None);
    complete
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
    let admission = crate::runtime::admit_hourly_capture();
    let recording = admission.is_some()
        && shared.recording.load(Ordering::Acquire)
        && !crate::allocation::failed();
    if recording {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Callback);
        let _reads = crate::read_accounting::Capture::new();
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let world_generation = crate::world_lifetime::generation();
        // Read the clock at entry: +468 still the hour just ended, the trampoline
        // has not run. The frame's identity and the settled state the body reads.
        let clock_start = qpc();
        let entry = cgame_state::read_clock(&memory, shared.base).filter(|_| {
            world_generation != u64::MAX && world_generation == crate::world_lifetime::generation()
        });
        crate::world_lifetime::boundary(world_generation, entry.is_some());
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: entry.and_then(|clock| clock.campaign_id.as_bytes().try_into().ok()),
            branch: None,
            load_signal: entry.map(|clock| clock.play_session_started_at),
            failed_hour: entry.map(|clock| clock.hour),
        });
        crate::equipment_destruction::boundary(entry);
        crate::naval_result_sources::boundary(entry);
        crate::experience_gains::boundary(entry);
        crate::air_losses::boundary(entry);
        crate::building_bombing::boundary(entry);
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
            world_generation,
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
        crate::allocation::attempted_hour(None);
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
    drop(admission);
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

fn begin_event_interval(frame: &mut Captured, hour: i32, continuous: bool, recovery: bool) {
    crate::relation_changes::boundary(!continuous || recovery);
    crate::history_changes::boundary(!continuous || recovery);
    // Refresh/retry requests a full read without interrupting the native log.
    // Only first/load/campaign/gap boundaries discard the land prefix.
    frame.extended.drain_land_events(hour, !continuous);
}

/// Every live read of the hour that just ended, on the game thread inside the
/// hook. The trampoline has not run, so the state is the settled end-of-hour
/// boundary, read exactly as the original collector did.
unsafe fn acquire(shared: &Shared, dispatch: Dispatch) {
    let Dispatch {
        world_generation,
        index,
        entry,
        entry_ticks,
        dispatched,
    } = dispatch;
    let memory = CountedMemory::new(unsafe { LiveMemory::new() });

    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.world_generation = world_generation;
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
        frame.provinces.conflict_walk = cprovince::ConflictWalk::default();
        frame.states_result = unavailable;
        frame.state_names_len = 0;
        frame.state_names_required = 0;
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
        frame.fixed_layout = None;
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
            frame.fixed_layout = Some(match shared.fixed_layout.get() {
                Some(report) => Ok(*report),
                None => engine::fixed_layout::check(&memory, shared.base)
                    .inspect(|report| _ = shared.fixed_layout.set(*report)),
            });
            let cadence = unsafe { &mut *shared.cadence.get() };
            let continuous = cadence.continuous(&clock);
            let recovery = shared.force_next.swap(false, Ordering::AcqRel);
            let refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
            forget_unpublished_families(shared);
            let force_full = cadence.force_full(&clock) | recovery | refresh;
            // A periodic encoding keyframe does not interrupt native mutation
            // observation. Its freshly certified owned rows remain complete.
            if !continuous || recovery {
                crate::portrait_changes::invalidate();
                crate::advisor_changes::invalidate();
                crate::opinion_changes::invalidate();
                crate::operation_outcomes::invalidate();
                crate::politics_changes::invalidate();
                extended::intel::forget_relation_opinions();
            }
            // Before any capture of the hour, while this thread holds the game.
            begin_event_interval(frame, clock.hour, continuous, recovery);
            frame
                .extended
                .drain_event_feed(clock.hour, !continuous || recovery);
            // Every production tick before this sample is in the log: the
            // daily pass runs on this thread, inside the hourly update this
            // callback precedes. The spool totals the log per branch.
            frame.extended.drain_line_output();
            frame.extended.drain_fuel_awards();
            crate::fuel_awards::install_offered();
            frame
                .extended
                .set_load_signal(clock.play_session_started_at);
            // Every construction line's daily output before this sample is in
            // its log too: the same serial pass advances construction.
            frame.extended.drain_construction_ic();
            crate::line_output::install_offered();
            frame
                .extended
                .drain_peace_settlements(continuous && !recovery);
            crate::peace_settlements::install_offered();
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
            frame.extended.capture_operation_outcomes();
            frame.extended.capture_politics();
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
            frame
                .extended
                .set_modifier_definitions_moved(modifier_table_moved(shared, &memory));
            frame.extended.set_new_division_mark(new_division_mark(
                shared,
                &memory,
                replay::cadence::should_walk(
                    replay::rows::TableId::DivisionEquipment,
                    clock.hour,
                    force_full,
                ),
            ));
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
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { take_slice_store(shared, crate::slicing::TECHNOLOGIES) },
                );
                let bytes = request.allocated_bytes();
                (queue!(acquisition::Job::Governance(request)), bytes)
            });
            // Variant rows read live memory only; the family leaves on its slot.
            let variants_pending = frame
                .extended
                .take_variants_request(shared.base, state, clock.hour, force_full)
                .map(|request| {
                    let bytes = request.allocated_bytes();
                    (queue!(acquisition::Job::Variants(request)), bytes)
                });
            // Naval uses this callback's independently bounded wing-membership census.
            let mut naval_job = decode::acquire_naval_job();
            // The orders-group registry census, once for the pass: the Orders
            // job's army-order census and the Naval job's order-of-battle copy
            // both walk these roots. This callback holds the game thread
            // until both have finished; neither keeps them past the pass.
            frame.map.orders.walk_roots(&memory, shared.base);
            let naval_roots_ticks =
                naval_job.capture_roots(frame.map.orders.roots(), frame.map.orders.root_outcome);
            let orders_request =
                frame
                    .extended
                    .take_orders_request(&mut frame.map.orders, shared.base, state);
            let orders_bytes = orders_request.allocated_bytes();
            let orders_pending = queue!(acquisition::Job::Orders(orders_request));
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
                game_score: shared.game_score_enabled,
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
            // The relations context, activity and owner slice come from the
            // real Intel pack: IntelResidual borrows it from here to its join,
            // leaving a vacant placeholder with no rows and no selection.
            let (relations_ctx, mut relations_selection) =
                frame.extended.relations_pilot().begin_relations(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    relations_refresh,
                );
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
                    .cloned()
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
                frame
                    .extended
                    .install_division_counts(&frame.division_fallback);
                supply_division_census_sources(
                    &frame.division_fallback,
                    &mut frame.extended,
                    division_source_capacity,
                );
                frame.table_ns[6] = shared.ns(qpc() - begin);
                frame.division_fallback.unit_inputs().cloned()
            };
            frame.extended.install_unit_inputs(inputs.clone());
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
                inputs: inputs.clone(),
            }));
            // The fat late-joined jobs go out after Divisions and Intel so those
            // find a free worker; the burst's short jobs have freed some by now.
            let (trait_rows, trait_bytes) =
                frame.extended.characters_pilot().trait_capture_capacities();
            let characters_ctx = extended::characters::CharactersCopyContext {
                base: shared.base,
                game_state: state,
                force_full,
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
            let character_part = |part: usize| -> [bool; 6] {
                std::array::from_fn(|index| {
                    characters_ctx.active[index] && extended::characters::PART_MASKS[part][index]
                })
            };
            // The roster job gathers the role census only when a role job runs.
            let role_hour =
                (1..CHARACTER_PARTS).any(|part| character_part(part).iter().any(|&value| value));
            let roster_active = character_part(0);
            if characters_refresh && roster_active.iter().any(|&value| value) {
                characters_pending[0] = Some(queue!(acquisition::Job::Characters(
                    acquisition::CharactersRequest {
                        context: extended::characters::CharactersCopyContext {
                            active: roster_active,
                            ..characters_ctx
                        },
                        payload: decode::acquire_characters_job(0),
                        roles: if role_hour {
                            extended::characters::Roles::Gather
                        } else {
                            extended::characters::Roles::Independent
                        },
                    }
                )));
            }
            let relations_pending = if relations_refresh {
                let mut payload = decode::acquire_relations_job();
                // The hour's owner slice and the family's store go with the
                // job and come back at its join. The slice was taken from the
                // real Intel pack before IntelResidual borrowed it.
                payload.selection = relations_selection.take();
                // SAFETY: only this callback takes and returns the store.
                payload.fingerprints =
                    Some(unsafe { take_slice_store(shared, crate::slicing::RELATIONS) });
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
            // SAFETY: only this callback takes and returns the stores.
            let (launch_store, state_stores) = unsafe {
                (
                    take_slice_store(shared, crate::slicing::RAID_LAUNCH),
                    crate::extended::RaidStateStores {
                        states: take_slice_store(shared, crate::slicing::RAID_STATE),
                        buildings: take_slice_store(shared, crate::slicing::RAID_BUILDINGS),
                    },
                )
            };
            let raid_state_pending = queue!(acquisition::Job::RaidState(
                frame.extended.take_raid_state_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    state_stores,
                )
            ));
            let raid_targets_pending = queue!(acquisition::Job::RaidTargets(
                frame.extended.take_raid_targets_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                )
            ));
            let raid_launch_pending = queue!(acquisition::Job::RaidLaunch(
                frame.extended.take_raid_launch_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    launch_store,
                )
            ));
            // The stability and naval mission families' stores travel with the
            // Slow job. A missing store only drops that family's early reads
            // for this hour.
            // SAFETY: only this callback takes and returns the stores.
            let slow_stores = unsafe {
                crate::extended::SlowStores {
                    stability: (*shared.slice_fingerprints[crate::slicing::STABILITY].get()).take(),
                    naval_missions: (*shared.slice_fingerprints[crate::slicing::NAVAL_MISSIONS]
                        .get())
                    .take(),
                }
            };
            let slow_pending = queue!(acquisition::Job::Slow(frame.extended.take_slow_request(
                &country_snapshot,
                shared.base,
                state,
                clock.hour,
                force_full,
                slow_stores
            )));
            let mut air_relations_pending: [Option<PendingAcquisition>; AIR_RELATION_PARTS] =
                [const { None }; AIR_RELATION_PARTS];
            for (part, pending) in air_relations_pending.iter_mut().enumerate() {
                *pending = Some(queue!(acquisition::Job::AirRelations(
                    frame.extended.take_air_relations_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        part,
                    )
                )));
            }
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
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { take_slice_store(shared, crate::slicing::TECHNOLOGIES) },
                    );
                    let bytes = request.allocated_bytes();
                    (queue!(acquisition::Job::Governance(request)), bytes)
                }
            };
            let presentation_request = frame.extended.take_presentation_request(
                &country_snapshot,
                shared.base,
                state,
                clock.hour,
                force_full,
                // SAFETY: only this callback takes and returns the store.
                Some(unsafe { take_slice_store(shared, crate::slicing::RETREAT_BANS) }),
            );
            let presentation_bytes = presentation_request.allocated_bytes();
            let presentation_pending = queue!(acquisition::Job::Presentation(presentation_request));
            let eta_ranges = frame.extended.plan_eta_input_ranges();
            let mut eta_inputs_pending: [Option<PendingAcquisition>; ETA_INPUT_RANGES] =
                [const { None }; ETA_INPUT_RANGES];
            for (range, pending) in eta_inputs_pending.iter_mut().enumerate().take(eta_ranges) {
                // SAFETY: this callback alone loans and returns each store.
                let cadence = unsafe { &mut *shared.eta_cadence[range].get() }
                    .take()
                    .expect("ETA cadence loan");
                *pending = Some(queue!(acquisition::Job::EtaInputs(
                    frame.extended.take_eta_inputs_request(
                        range,
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                        cadence
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
            let mut naval_install_ticks = naval_roots_ticks;
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
                        // Return any census loan before the roster job can recycle.
                        drop(request.roles);
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
            // Role jobs borrow the roster's census like the portrait shares;
            // without one they walk the roster independently.
            let role_census = characters_payload[0]
                .as_ref()
                .and_then(|job| job.role_census());
            for (part, pending) in characters_pending.iter_mut().enumerate().skip(1) {
                let active = character_part(part);
                if characters_refresh && active.iter().any(|&value| value) {
                    *pending = Some(queue!(acquisition::Job::Characters(
                        acquisition::CharactersRequest {
                            context: extended::characters::CharactersCopyContext {
                                active,
                                ..characters_ctx
                            },
                            payload: decode::acquire_characters_job(part),
                            roles: role_census.as_ref().map_or(
                                extended::characters::Roles::Independent,
                                |census| {
                                    extended::characters::Roles::Borrow(Arc::clone(census))
                                },
                            ),
                        }
                    )));
                }
            }
            drop(role_census);
            let (naval_request, report, naval_failure) = completed!(naval_pending, Naval);
            naval_report = report;
            if naval_report.raw_ok {
                naval_report.copy_ticks[0] += naval_install_ticks;
            }
            if let Some(wings) = air_wing_payload.as_ref() {
                naval_report.copy_ticks[0] += wings.membership_ticks;

                air_wing_report.copy_ticks[0] += frame.map.install_owned_air(wings);
                frame
                    .extended
                    .air_wing_pilot()
                    .install_owned_mission_identities(wings);
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
            // The owned census supplied the wing identities: the missions
            // and naval-patrol combats overlap the map sample.
            let air_missions_pending = if air_wing_payload.is_some() {
                Some(queue!(acquisition::Job::AirMissions(
                    frame
                        .extended
                        .take_air_missions_request(&country_snapshot, shared.base, state)
                )))
            } else {
                None
            };
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
            // Otherwise the typed visitor supplied them in the map sample.
            let air_missions_pending = match air_missions_pending {
                Some(pending) => pending,
                None => queue!(acquisition::Job::AirMissions(
                    frame
                        .extended
                        .take_air_missions_request(&country_snapshot, shared.base, state)
                )),
            };
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
            // The Late job joins the registrations; the history parts need
            // the combats' regions.
            let (air_missions_request, _, air_missions_failure) =
                completed!(air_missions_pending, AirMissions);
            frame
                .extended
                .restore_air_missions_request(air_missions_request, air_missions_failure.is_none());
            // SAFETY: only this callback takes and returns the store.
            let air_scores = unsafe { &mut *shared.raid_air_scores.get() }
                .take()
                .expect("raid air score loan");
            let late_pending = queue!(acquisition::Job::Late(frame.extended.take_late_request(
                &mut frame.map.orders,
                naval_report.raw_ok || naval_failure.is_some(),
                shared.base,
                state,
                clock.hour,
                force_full,
                air_scores
            )));
            let mut air_histories_pending: [Option<PendingAcquisition>; AIR_HISTORY_PARTS] =
                [const { None }; AIR_HISTORY_PARTS];
            for (part, pending) in air_histories_pending.iter_mut().enumerate() {
                *pending = Some(queue!(acquisition::Job::AirHistories(
                    frame.extended.take_air_histories_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        part,
                    )
                )));
            }
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
                let (fingerprints, payload) = frame.extended.relations_pilot().join_relations(
                    request.payload,
                    report,
                    failure,
                );
                if fingerprints.is_some() {
                    // SAFETY: only this callback takes and returns the store.
                    unsafe {
                        *shared.slice_fingerprints[crate::slicing::RELATIONS].get() = fingerprints
                    };
                }
                relations_report = report;
                relations_failure = failure;
                match payload {
                    Ok(payload) => relations_payload = Some(payload),
                    Err(payload) => decode::recycle_relations_job(payload),
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
            let stores = frame
                .extended
                .restore_slow_request(slow_request, slow_failure.is_none());
            // SAFETY: only this callback takes and returns the stores.
            unsafe {
                if let Some(store) = stores.stability {
                    *shared.slice_fingerprints[crate::slicing::STABILITY].get() = Some(store);
                }
                if let Some(store) = stores.naval_missions {
                    *shared.slice_fingerprints[crate::slicing::NAVAL_MISSIONS].get() = Some(store);
                }
            };
            // Only a completed Slow job's hour is counted; the game thread
            // never waits for the status thread's copy.
            let failures = frame.extended.template_model_failures();
            if slow_failure.is_none() && failures.rows != 0 {
                match shared.template_models.try_lock() {
                    Ok(mut stats) => stats.fold(failures),
                    Err(_) => {
                        shared
                            .template_models_missed
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            for pending in air_relations_pending.into_iter().flatten() {
                let (request, _, failure) = completed!(pending, AirRelations);
                frame
                    .extended
                    .restore_air_relations_request(request, failure.is_none());
            }
            let (raid_state_request, _, raid_state_failure) =
                completed!(raid_state_pending, RaidState);
            let state_stores = frame
                .extended
                .restore_raid_state_request(raid_state_request, raid_state_failure.is_none());
            let (raid_targets_request, _, raid_targets_failure) =
                completed!(raid_targets_pending, RaidTargets);
            frame
                .extended
                .restore_raid_targets_request(raid_targets_request, raid_targets_failure.is_none());
            let (raid_launch_request, _, raid_launch_failure) =
                completed!(raid_launch_pending, RaidLaunch);
            let launch_store = frame
                .extended
                .restore_raid_launch_request(raid_launch_request, raid_launch_failure.is_none());
            // SAFETY: only this callback takes and returns the stores.
            unsafe {
                return_slice_store(shared, crate::slicing::RAID_LAUNCH, launch_store);
                return_slice_store(shared, crate::slicing::RAID_STATE, state_stores.states);
                return_slice_store(
                    shared,
                    crate::slicing::RAID_BUILDINGS,
                    state_stores.buildings,
                );
            };
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
            let technologies = frame
                .extended
                .restore_governance_request(governance_request, governance_report);
            // SAFETY: only this callback takes and returns the store.
            unsafe {
                *shared.slice_fingerprints[crate::slicing::TECHNOLOGIES].get() = Some(technologies)
            };
            if let Some((pending, loaned_bytes)) = variants_pending {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, pending) };
                let acquisition::Job::Variants(request) = completed.request else {
                    unreachable!("Variants ticket type")
                };
                let acquired_bytes = request.allocated_bytes();
                if acquired_bytes >= loaned_bytes {
                    shared
                        .pool_bytes
                        .fetch_add((acquired_bytes - loaned_bytes) as u64, Ordering::Relaxed);
                } else {
                    shared
                        .pool_bytes
                        .fetch_sub((loaned_bytes - acquired_bytes) as u64, Ordering::Relaxed);
                }
                let report = completed.report.map(|report| {
                    let acquisition::Report::Variants(ticks) = report else {
                        unreachable!("Variants report type")
                    };
                    ticks
                });
                frame.extended.restore_variants_request(request, report);
            }
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
            // Role and trait jobs need their verdicts here, before the typed
            // fallback; they must not delay dispatch of unrelated map/gated/late work.
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
            if let Some(store) = frame
                .extended
                .restore_presentation_request(presentation_request, presentation_report)
            {
                // SAFETY: only this callback takes and returns the store.
                unsafe { return_slice_store(shared, crate::slicing::RETREAT_BANS, store) };
            }
            let border_ticks = frame.extended.capture_border_inputs(
                &cached,
                shared.base,
                state,
                (&frame.provinces.rows, frame.provinces_result),
                (&mut frame.countries, &mut frame.countries_result),
            );
            ticks[replay::rows::TableId::DivisionStatus.index()] += border_ticks[0];
            frame.table_ns[replay::rows::TableId::Countries.index()] +=
                shared.ns(border_ticks[1] as i64);
            for pending in eta_inputs_pending.into_iter().flatten() {
                // SAFETY: this thread still owns the live acquisition boundary.
                let completed = unsafe { finish_acquisition(shared, &mut batch, pending) };
                let acquisition::Job::EtaInputs(mut request) = completed.request else {
                    unreachable!("EtaInputs ticket type")
                };
                let report = completed.report.map(|report| {
                    let acquisition::Report::EtaInputs(report) = report else {
                        unreachable!("EtaInputs report type")
                    };
                    report
                });
                if report.is_err() {
                    request
                        .captured
                        .cadence
                        .as_mut()
                        .expect("ETA cadence loan")
                        .invalidate();
                }
                let bytes = request
                    .captured
                    .cadence
                    .as_ref()
                    .expect("ETA cadence loan")
                    .allocated_bytes() as u64;
                let previous =
                    shared.eta_cadence_bytes[request.range].swap(bytes, Ordering::Relaxed);
                if bytes >= previous {
                    shared
                        .pool_bytes
                        .fetch_add(bytes - previous, Ordering::Relaxed);
                } else {
                    shared
                        .pool_bytes
                        .fetch_sub(previous - bytes, Ordering::Relaxed);
                }
                // SAFETY: the owned job has joined; no worker retains this store.
                unsafe {
                    *shared.eta_cadence[request.range].get() = request.captured.cadence.take()
                };
                frame.extended.restore_eta_inputs_request(request, report);
            }
            segment!(11);
            let (late_request, _, late_failure) = completed!(late_pending, Late);
            let air_scores = frame.extended.restore_late_request(
                late_request,
                late_failure.is_none(),
                &mut frame.map.orders,
            );
            for pending in air_histories_pending.into_iter().flatten() {
                let (request, _, failure) = completed!(pending, AirHistories);
                frame
                    .extended
                    .restore_air_histories_request(request, failure.is_none());
            }
            frame.extended.settle_air_histories();
            let bytes = air_scores.allocated_bytes() as u64;
            let previous = shared.raid_air_score_bytes.swap(bytes, Ordering::Relaxed);
            shared
                .pool_bytes
                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
            // SAFETY: the owned job has joined; no worker retains this store.
            unsafe { *shared.raid_air_scores.get() = Some(air_scores) };
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
                frame.extended.game_score_pilot().fail_air(reason);
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
                fail_divisions_capture(frame, reason, divisions_ctx.details_active);
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
                frame.table_ns[index] = exported_table_ticks(table, walk, ticks[index])
                    .map_or(0, |ticks| shared.ns(ticks as i64));
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
                for (table, ticks) in [
                    TableId::CountryGameScoreInputs,
                    TableId::AirWingGameScoreStats,
                ]
                .into_iter()
                .zip(air_wing_report.game_score_ticks)
                {
                    let index = table.index();
                    frame.table_ns[index] =
                        frame.table_ns[index].saturating_add(shared.ns(ticks as i64));
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
        if frame.world_generation != crate::world_lifetime::generation() {
            frame.extended.discard_event_feed();
            lost_observer_interval();
            frame.clock = None;
            shared.force_next.store(true, Ordering::Release);
        }
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
        lost_observer_interval();
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
            let report = match &mut request {
                acquisition::Job::Governance(governance) => {
                    let report = governance.fail("governance worker admission failed");
                    acquisition::Report::Governance(report)
                }
                acquisition::Job::Variants(variants) => {
                    let ticks = variants.fail("variant worker admission failed");
                    acquisition::Report::Variants(ticks)
                }
                acquisition::Job::Orders(orders) => {
                    let report = orders.fail("army order worker admission failed");
                    acquisition::Report::Orders(report)
                }
                // An inline part could copy megabytes of vectors on the game
                // thread; its countries are copied afresh at the next hour.
                acquisition::Job::AirRelations(relations) => {
                    relations
                        .staging
                        .fail("air relation worker admission failed");
                    extended::air_relations::certificates(relations.part).forget();
                    acquisition::Report::AirRelations(acquisition::AirRelationsReport)
                }
                // Likewise for a history part: the hour's settle then withdraws
                // every part's certificates and the next hour copies afresh.
                acquisition::Job::AirHistories(histories) => {
                    extended::air_histories::refuse(&mut histories.staging);
                    acquisition::Report::AirHistories(acquisition::AirHistoriesReport)
                }
                acquisition::Job::AirMissions(missions) => {
                    missions.refuse();
                    acquisition::Report::AirMissions(acquisition::AirMissionsReport)
                }
                acquisition::Job::Ordinary(ordinary) => {
                    let report = ordinary.fail("ordinary worker admission failed");
                    acquisition::Report::Ordinary(report)
                }
                acquisition::Job::Presentation(presentation) => {
                    let report = presentation.fail("division presentation worker admission failed");
                    acquisition::Report::Presentation(report)
                }
                acquisition::Job::EtaInputs(eta) => {
                    let report = eta
                        .captured
                        .fail_owned_capture("division ETA worker admission failed");
                    acquisition::Report::EtaInputs(report)
                }
                _ => {
                    // SAFETY: this request was rejected before acquiring any live state.
                    return unsafe { batch.run_inline(request) };
                }
            };
            acquisition::Completion {
                request,
                report: Ok(report),
                table_reads: crate::read_accounting::ZERO,
                #[cfg(feature = "callback_profile")]
                profile: Default::default(),
            }
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

/// Shared synthetic address space for the typed Wars capture and FIFO tests:
/// three countries plus one alias slot, two wars with signed-extreme rows,
/// one foreign-type relation, one retired war, and matrix slots with an
/// alias raw target. Mirrors the engine `cwar_relation` fixture shape.
#[cfg(test)]
pub(crate) mod wars_test_kit;

/// Whether the modifier definition table header differs from the previous
/// hour's; one guarded 16-byte read on the game thread. Records the new value.
fn modifier_table_moved(shared: &Shared, memory: &impl engine::Memory) -> bool {
    let observed = engine::modifier_definition::table(memory, shared.base)
        .map_or([0, u64::MAX], |table| {
            [table.data as u64, u64::from(table.count)]
        });
    let moved = shared
        .modifier_table
        .iter()
        .zip(observed)
        .any(|(held, value)| held.load(Ordering::Relaxed) != value);
    if moved {
        for (held, value) in shared.modifier_table.iter().zip(observed) {
            held.store(value, Ordering::Relaxed);
        }
    }
    moved
}

/// Divisions are minted from two monotonic counters, by handle type: the
/// counters at the daily walk split walked divisions from new ones.
fn new_division_mark(
    shared: &Shared,
    memory: &impl Memory,
    daily: bool,
) -> extended::new_divisions::Mark {
    extended::new_divisions::next_mark(
        &shared.division_mark,
        daily,
        engine::read::<i32>(memory, shared.base + engine::cship::NEXT_ID_RVA),
        engine::read::<u32>(
            memory,
            shared.base + engine::creference_registry::ID_COUNTER_RVA,
        ),
    )
}

/// The game-thread ticks a frame records for an extended table: a skipped
/// table keeps zero cost, except CountryIntel, whose walk moves to a later
/// hour when its shared buffer was held (`daily_intel::Loan::borrow`). That
/// retried walk is a real read, so it keeps its ticks: the oracle tells a
/// table read at an hour by its measured cost.
fn exported_table_ticks(table: replay::rows::TableId, walk: bool, ticks: u64) -> Option<u64> {
    (walk || (table == replay::rows::TableId::CountryIntel && ticks != 0)).then_some(ticks)
}

#[cfg(test)]
mod exported_ticks_tests;

#[cfg(test)]
mod tests;
