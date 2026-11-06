//! The sole game-thread callback. Buffers are allocated before publication to its free queue.

use std::sync::Arc;
mod divisions;
use divisions::fail_divisions_capture;
pub(crate) use divisions::{
    DivisionsCopyContext, DivisionsCopyReport, DivisionsJob, copy_divisions_core_refresh,
    copy_divisions_inputs_refresh_growing, decode_divisions_payload,
};
#[cfg(test)]
pub(crate) use divisions::{
    copy_divisions_inputs_refresh, copy_divisions_refresh, divisions_test_kit,
};

pub use crate::clock::qpc;
use crate::{
    acquisition_factions, acquisition_jobs as acquisition, acquisition_states, acquisition_wars,
    decode, extended,
    map_capture::{MapCapacities, MapCaptured, ShipCensus},
    roster::Roster,
    table_capture::grow_capacity,
};
pub use collector_extended_support::hourly::BUFFERS;
use collector_extended_support::hourly::TICKING;
use crossbeam_queue::ArrayQueue;
#[cfg(test)]
use engine_native::cunit;
use engine_native::{
    Memory, capture_text::Owned, carmy, ccountry, cfaction, cfaction_system, cgame_state,
    cgame_state::Clock, country_memory::Cache, country_memory::SeedStatus, cprovince, cstate,
    cwar_relation, memory::CountedMemory, memory::LiveMemory, memory::ReadCounts,
    save_allocation_counters, save_allocation_counters::AllocationCounters, walk::Outcome,
};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
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

const SUPPLY_PARTS: usize = extended::supply_trains::PARTS;
const CHARACTER_PARTS: usize = extended::characters::PARTS;
const AIR_RELATION_PARTS: usize = extended::air_relations::PARTS;
const AIR_HISTORY_PARTS: usize = extended::air_histories::PARTS;
const PORTRAIT_PARTS: usize = extended::character_portraits::PARTS;
pub const SEGMENTS: usize = 15;
pub const SEGMENTS_SCOPE: &str = "game-thread wall of the parallel phase split at fixed marks: 0 first held burst of independent jobs; 1 completion-driven dependency scheduling through Late/history and FinalInputs dispatch (root census -> Orders/parts -> stitch -> Fronts and, with Naval/wing, ChildFronts; division inputs/parts -> Divisions -> ArmyIcons and PresentationPrepare -> status ranges -> Presentation; census consumers, raw verdicts, map sample, AirMissions and ScoresRails are joined only when their full producer barrier is ready); 2-7 unused; 8 weather/lexical + core joins (presentation publication edges continue during these and later joins); 9 supply/portraits/order/template/slow/war overview/stability/raid/gated/character input/governance/research joins + event-feed seal dispatch + ShipModels detail joins + Ordinary family joins + event-feed seal join + slot-8 production and part joins + independent Variants join; 10 capture_tables; 11 Late join + Fronts join + presentation and movement joins + order transport + border inputs; 12 history-part joins + FinalInputs join + capture_tables_late; 13 Intel and AirBases joins + visibility + failure handling; 14 ShipModels completed projection";
/// Core row capacities: countries, provinces, links, states, divisions, wars,
/// factions, faction members, faction name bytes, border-conflict provinces,
/// their admitted group handles, and state name bytes.
pub const INITIAL_CAPACITIES: [usize; 12] = [0; 12];

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
/// the trampoline runs, so a frame labelled with the entry hour is the state
/// at the end of that hour. Recorded in every session so readers know its instant.
pub const SAMPLE_BOUNDARY: replay_data::identity::SampleBoundary =
    replay_data::identity::SampleBoundary::EndOfHour;
const LOSS_RUNS: usize = 64;
pub static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
// Staggered cadence branch/keyframe state. Game thread only.
#[derive(Debug, Default)]
struct CadenceState {
    initialized: bool,
    prev_hour: i32,
    prev_signal: i64,
    prev_campaign: Owned,
    prev_world: u64,
}

impl CadenceState {
    fn continuous(&self, clock: &Clock<&[u8]>) -> bool {
        self.initialized
            && self.prev_hour.checked_add(1) == Some(clock.hour)
            && self.prev_signal == clock.play_session_started_at
            && self.prev_campaign.as_bytes() == clock.campaign_id
            && self.prev_world == crate::world_lifetime::generation()
    }
    // New branches never open on stale data: first, signal, campaign and
    // discontinuity force a full refresh. A periodic encoding keyframe does
    // not: the spool builds it from the rows it already holds for every
    // staggered and sliced table, so it adds no native reads.
    fn force_full(&mut self, clock: &Clock<&[u8]>) -> bool {
        if !self.initialized {
            self.initialized = true;
            self.prev_hour = clock.hour;
            self.prev_signal = clock.play_session_started_at;
            self.prev_campaign
                .copy_prepared(clock.campaign_id)
                .expect("campaign prepared before callback");
            self.prev_world = crate::world_lifetime::generation();
            return true;
        }
        let signal = self.prev_signal != clock.play_session_started_at;
        let campaign = self.prev_campaign.as_bytes() != clock.campaign_id;
        let gap = self.prev_hour.checked_add(1) != Some(clock.hour);
        let world = self.prev_world != crate::world_lifetime::generation();
        let force = signal || campaign || gap || world;
        self.prev_hour = clock.hour;
        self.prev_signal = clock.play_session_started_at;
        self.prev_campaign
            .copy_prepared(clock.campaign_id)
            .expect("campaign prepared before callback");
        self.prev_world = crate::world_lifetime::generation();
        force
    }
}

#[derive(Clone, Debug, Default)]
pub struct LostObservation {
    pub campaign_id: Owned,
    pub first_hour: i32,
    pub last_hour: i32,
    pub loaded_hour: Option<i32>,
    pub load_signal: Option<i64>,
    pub count: u64,
}

pub struct LossBatch {
    items: [LostObservation; LOSS_RUNS],
    len: usize,
    pub unknown_count: u64,
}

impl Default for LossBatch {
    fn default() -> Self {
        Self {
            items: std::array::from_fn(|_| LostObservation::default()),
            len: 0,
            unknown_count: 0,
        }
    }
}

impl LossBatch {
    fn prepare(&mut self, len: usize) -> std::io::Result<()> {
        for item in &mut self.items {
            item.campaign_id.prepare(len)?;
        }
        Ok(())
    }
    fn clear(&mut self) {
        self.len = 0;
        self.unknown_count = 0;
    }
    fn allocated_bytes(&self) -> usize {
        self.items
            .iter()
            .map(|item| item.campaign_id.allocated_bytes())
            .sum()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, LostObservation> {
        self.items[..self.len].iter()
    }

    fn push(&mut self, clock: Option<Clock<&[u8]>>) {
        let Some(clock) = clock else {
            self.unknown_count += 1;
            return;
        };
        if let Some(last) = self.items[..self.len].last_mut()
            && last.campaign_id.as_bytes() == clock.campaign_id
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
        let item = &mut self.items[self.len];
        if item.campaign_id.copy_prepared(clock.campaign_id).is_err() {
            self.unknown_count += 1;
            return;
        }
        item.first_hour = clock.hour;
        item.last_hour = clock.hour;
        item.loaded_hour = Some(clock.hour);
        item.load_signal = Some(clock.play_session_started_at);
        item.count = 1;
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
            rows: engine_allocation::owned_vec(engine_allocation::default_vec::<
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
        memory: &impl engine_native::Memory,
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
    clock_spare: Owned,
    pub counters: Option<AllocationCounters>,
    pub countries: Vec<ccountry::Row>,
    /// Provinces rows and the BorderConflicts rows of the same walk.
    pub provinces: cprovince::Capture,
    pub states: Vec<cstate::Row>,
    pub(crate) state_inputs: acquisition_states::Inputs,
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
    pub faction_goals: crate::faction_goals::Captured,
    pub faction_companions: crate::faction_companions::Captured,
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
    /// This hour's diagnostics, replaced in full before decode publication.
    pub(crate) hour_cost: HourCost,
    pub qpc: i64,
    /// Reserved collector storage, including the executor's actual shared
    /// allocation, cells, words, metadata, queue, thread ids/handles, boxed
    /// runner and enabled help histories; copied into the hour's Metrics.
    pub pool_bytes: u64,
    pub dropped_frames: u64,
    pub dropped_before: LossBatch,
    /// The verdicts were reset for the next hour when the spool returned
    /// this frame ([`Self::reset_verdicts`]); the callback then skips it.
    verdicts_reset: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HourCost {
    pub entry_ns: u64,
    pub buffer_wait_ns: u64,
    pub setup_ns: u64,
    pub parallel_ns: u64,
    pub residual_ns: u64,
    pub segments_ns: [u64; SEGMENTS],
    pub slowest_join: Option<SlowestJoin>,
    pub collect_stalls_over_1ms: u64,
    pub growths: u64,
    pub diagnostics: crate::acquisition_diagnostics::Hour,
    pub idle: crate::acquisition::idle_diagnostics::Hour,
    pub variants: Option<crate::design_changes::profile::Hour>,
    #[cfg(feature = "callback_profile")]
    pub join_timeline: Option<crate::acquisition_diagnostics::JoinSpan>,
    pub first_since_attach: bool,
    pub continuous: bool,
    pub force_full: bool,
    pub recovery: bool,
    pub refresh: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlowestJoin {
    pub role: Option<usize>,
    pub readiness: bool,
    pub wait_ns: u64,
    pub collect_stall_ns: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HourJoins {
    slowest: Option<SlowestJoin>,
    collect_stalls_over_1ms: u64,
}

#[derive(Clone, Copy, Default)]
struct JoinTally {
    hour: HourJoins,
    // The opt-in sample is consumed after acquire takes the hour's diagnostics.
    #[cfg(feature = "callback_profile")]
    profile: crate::callback_profile::JoinSample,
    #[cfg(feature = "callback_profile")]
    profile_span: Option<crate::acquisition_diagnostics::JoinSpan>,
}

thread_local! {
    static HOUR_JOINS: std::cell::Cell<Option<JoinTally>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn reset_hour_joins() {
    HOUR_JOINS.set(None);
    crate::acquisition_diagnostics::reset();
    crate::acquisition::idle_diagnostics::reset();
}

pub(crate) fn record_hour_join(
    role: usize,
    wait_ns: u64,
    collect_stall_ns: u64,
    #[cfg(feature = "callback_profile")] profile: crate::acquisition::Profile,
    #[cfg(feature = "callback_profile")] span: Option<crate::acquisition_diagnostics::JoinSpan>,
) {
    let mut tally = HOUR_JOINS.get().unwrap_or_default();
    if tally.hour.slowest.is_none_or(|join| wait_ns > join.wait_ns) {
        tally.hour.slowest = Some(SlowestJoin {
            role: Some(role),
            readiness: false,
            wait_ns,
            collect_stall_ns,
        });
        #[cfg(feature = "callback_profile")]
        {
            tally.profile = crate::callback_profile::JoinSample {
                role: Some(role),
                wall_ns: wait_ns,
                readiness: false,
                profile,
            };
            tally.profile_span = span;
        }
    }
    tally.hour.collect_stalls_over_1ms += u64::from(collect_stall_ns >= JoinCosts::STALL_NS[1]);
    HOUR_JOINS.set(Some(tally));
}

pub(crate) fn record_readiness_wait(
    wait_ns: u64,
    outstanding: crate::acquisition_diagnostics::Outstanding,
    completion: Option<crate::acquisition_diagnostics::Completion>,
    ending_probe_exhausted: bool,
) {
    let completion = completion.filter(|_| !ending_probe_exhausted);
    crate::acquisition_diagnostics::update(|hour| {
        hour.readiness(wait_ns, outstanding, completion, ending_probe_exhausted)
    });
    let mut tally = HOUR_JOINS.get().unwrap_or_default();
    if tally.hour.slowest.is_none_or(|join| wait_ns > join.wait_ns) {
        tally.hour.slowest = Some(SlowestJoin {
            role: completion.map(|completion| completion.identity.role),
            readiness: true,
            wait_ns,
            collect_stall_ns: 0,
        });
        #[cfg(feature = "callback_profile")]
        {
            tally.profile = crate::callback_profile::JoinSample {
                role: completion.map(|completion| completion.identity.role),
                wall_ns: wait_ns,
                readiness: true,
                ..Default::default()
            };
            tally.profile_span = None;
        }
    }
    HOUR_JOINS.set(Some(tally));
}

fn take_hour_joins() -> HourJoins {
    let mut tally = HOUR_JOINS.get().unwrap_or_default();
    let hour = std::mem::take(&mut tally.hour);
    HOUR_JOINS.set(Some(tally));
    hour
}

#[cfg(feature = "callback_profile")]
pub(crate) fn take_profile_join() -> crate::callback_profile::JoinSample {
    HOUR_JOINS.take().unwrap_or_default().profile
}

impl Captured {
    /// Clear every verdict and cost of the previous hour, as the callback did
    /// before reading. The spool runs it when it returns the frame, after
    /// growth, so the hour's game thread only takes the frame.
    pub(crate) fn reset_verdicts(&mut self) {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.table_ns = [0; TABLE_COUNT];
        self.hour_cost = HourCost::default();
        self.table_reads = crate::read_accounting::ZERO;
        self.countries_result = unavailable;
        self.provinces_result = unavailable;
        self.provinces.conflict_walk = cprovince::ConflictWalk::default();
        self.states_result = unavailable;
        self.state_inputs.reset();
        self.state_names_len = 0;
        self.state_names_required = 0;
        self.divisions_result = unavailable;
        self.wars_result = unavailable;
        self.wars_snapshot_sampled = false;
        self.factions_result = unavailable;
        self.faction_names_len = 0;
        self.faction_names_required = 0;
        self.faction_members_result = unavailable;
        self.links_result = unavailable;
        self.map.reset();
        self.extended.reset_keeping_country_intel();
        self.country_cache_status = SeedStatus::Unavailable {
            reason: "clock unavailable",
        };
        self.country_cache_entries = 0;
        self.country_cache_ns = 0;
        self.order_counters.reset();
        self.fixed_layout = None;
    }

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
        let mut division_fallback = crate::division_census::Census::new_worker();
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
            clock_spare: Owned::default(),
            counters: None,
            countries: engine_allocation::default_vec::<ccountry::Row>(capacities[0])?,
            provinces: cprovince::Capture::try_with_capacity(
                capacities[1],
                capacities[9],
                capacities[10],
            )?,
            states: engine_allocation::default_vec::<cstate::Row>(capacities[3])?,
            state_inputs: acquisition_states::Inputs::with_capacity(capacities[3])?,
            state_names: engine_allocation::default_vec(capacities[11])?,
            state_names_len: 0,
            state_names_required: 0,
            divisions: engine_allocation::default_vec::<carmy::Row>(capacities[4])?,
            division_fallback,
            wars: engine_allocation::default_vec::<cwar_relation::Row>(capacities[5])?,
            wars_snapshot: cwar_relation::OwnedSnapshotScratch::try_with_capacity(capacities[0])?,
            wars_snapshot_sampled: false,
            factions: engine_allocation::default_vec::<cfaction::Row>(capacities[6])?,
            faction_names: engine_allocation::default_vec(capacities[8])?,
            faction_goals: Default::default(),
            faction_companions: Default::default(),
            faction_names_len: 0,
            faction_names_required: 0,
            faction_members: engine_allocation::default_vec::<cfaction_system::MemberRow>(
                capacities[7],
            )?,
            links: engine_allocation::default_vec::<ccountry::LinkRow>(capacities[2])?,
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
            hour_cost: HourCost::default(),
            qpc: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            dropped_before: LossBatch::default(),
            verdicts_reset: false,
        };
        if crate::allocation::failed() {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        Ok(frame)
    }

    pub(crate) fn prepare_clock(&mut self, len: usize) -> std::io::Result<()> {
        if let Some(clock) = &mut self.clock {
            clock.campaign_id.prepare(len)?;
        } else {
            self.clock_spare.prepare(len)?;
        }
        self.dropped_before.prepare(len)
    }
    fn set_clock(&mut self, entry: Option<Clock<&[u8]>>) {
        if let Some(previous) = self.clock.take() {
            self.clock_spare = previous.campaign_id;
        }
        self.clock = entry.map(|clock| {
            self.clock_spare
                .copy_prepared(clock.campaign_id)
                .expect("frame campaign prepared before callback");
            clock.with_campaign_id(std::mem::take(&mut self.clock_spare))
        });
    }
    pub fn allocated_bytes(&self) -> usize {
        self.core_allocated_bytes() + self.map.allocated_bytes() + self.extended.allocated_bytes()
    }

    /// Frame-owned storage outside the map and extended packs. Spool diagnostics
    /// already measure those packs individually; do not traverse them twice.
    pub(crate) fn core_allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .clock
                .as_ref()
                .map_or(0, |clock| clock.campaign_id.allocated_bytes())
            + self.clock_spare.allocated_bytes()
            + self.dropped_before.allocated_bytes()
            + self.order_counters.allocated_bytes()
            + self.wars_snapshot.allocated_bytes()
            + self.division_fallback.allocated_bytes() as usize
            + self.countries.capacity() * size_of::<ccountry::Row>()
            + self.provinces.allocated_bytes()
            + self.states.capacity() * size_of::<cstate::Row>()
            + self.state_inputs.allocated_bytes()
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
            inputs: std::mem::take(&mut self.state_inputs),
            store: None,
            countries: None,
            occupation_store: None,
        }
    }

    pub(crate) fn restore_states_request(
        &mut self,
        mut request: acquisition_states::Request,
        report: acquisition_states::Report,
        failure: Option<&'static str>,
    ) -> acquisition_states::Report {
        let report = failure.map_or(report, |reason| {
            request
                .inputs
                .retain_required(request.visitor.completed_sources().2);
            request.visitor.fail_sources(reason);
            request.inputs.fail(reason);
            acquisition_states::Report::failed(reason)
        });
        request.census.restore(&mut self.states);
        (self.state_names_len, self.state_names_required) =
            request.names.restore(&mut self.state_names);
        self.extended.restore_state_visitor(request.visitor);
        self.state_inputs = request.inputs;
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
            store: None,
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

    /// Growth requires exclusive ownership of the frame's queue index.
    pub fn grow(&mut self, capacities: &mut [usize; 12]) {
        engine_allocation::advisory(|| self.grow_hints(capacities));
    }

    fn grow_hints(&mut self, capacities: &mut [usize; 12]) {
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
        grow_capacity(&mut capacities[3], self.state_inputs.required());
        resize(&mut self.states, self.states_result, &mut capacities[3]);
        self.state_inputs.grow(capacities[3]);
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

/// The game thread's own work in the parked hour, by kind: cumulative QPC
/// wall since attach over `hours` acquired hours. Worker time overlaps it and
/// is not here; join waits are `acquisition_wait_ns`.
#[derive(Default)]
pub struct GameThreadPhases {
    pub hours: AtomicU64,
    /// Resetting the reused frame's verdicts and reserving its decode slot.
    pub frame_reset_ns: AtomicU64,
    /// The observer boundary job, submission and join included.
    pub boundaries_ns: AtomicU64,
    /// The shared country authentication snapshot.
    pub country_snapshot_ns: AtomicU64,
    /// Jobs submitted, and their submission wall.
    pub dispatched_jobs: AtomicU64,
    /// Wakes that released held bursts, the jobs they released and their wall.
    pub releases: AtomicU64,
    pub released_jobs: AtomicU64,
    pub release_ns: AtomicU64,
    /// The map sample's typed walk.
    pub map_sample_ns: AtomicU64,
    /// Table assembly after the joins: capture_tables, border inputs, the
    /// late tables and the ShipModels projection.
    pub table_assembly_ns: AtomicU64,
}

impl GameThreadPhases {
    fn release(&self, jobs: usize, ns: u64) {
        if jobs != 0 {
            self.releases.fetch_add(1, Ordering::Relaxed);
            self.released_jobs.fetch_add(jobs as u64, Ordering::Relaxed);
        }
        self.release_ns.fetch_add(ns, Ordering::Relaxed);
    }
}

/// Cumulative Presentation acquisition by pass, one observation per hour.
#[derive(Default)]
pub struct PresentationStats {
    pub hours: AtomicU64,
    /// [`extended::division_presentation::PHASE_NAMES`] order.
    pub phase_ns: [AtomicU64; extended::division_presentation::PHASES],
    pub range_dispatch_ns: AtomicU64,
    pub store_wait_ns: AtomicU64,
    pub input_consumers_ns: AtomicU64,
    pub input_restore_ns: AtomicU64,
    pub early_steps: [EarlyStepStats; EarlyStep::ALL.len()],
    pub wait_advance: EarlyStepStats,
    pub queue_ns: AtomicU64,
}

#[derive(Default)]
pub struct EarlyStepStats {
    pub calls: AtomicU64,
    pub ns: AtomicU64,
}

struct EarlyStepTimer<'a> {
    shared: &'a Shared,
    stats: &'a EarlyStepStats,
    begin: i64,
    wait: u64,
    step: EarlyStep,
}
impl<'a> EarlyStepTimer<'a> {
    fn new(shared: &'a Shared, stats: &'a EarlyStepStats, step: EarlyStep) -> Self {
        Self {
            shared,
            stats,
            begin: qpc(),
            step,
            wait: shared.acquisition_wait_ns.load(Ordering::Relaxed),
        }
    }
}
impl Drop for EarlyStepTimer<'_> {
    fn drop(&mut self) {
        let wall_ns = self.shared.ns(qpc() - self.begin);
        crate::acquisition_diagnostics::update(|hour| hour.step(self.step as usize, wall_ns));
        let ns = wall_ns.saturating_sub(
            self.shared
                .acquisition_wait_ns
                .load(Ordering::Relaxed)
                .saturating_sub(self.wait),
        );
        self.stats.ns.fetch_add(ns, Ordering::Relaxed);
        self.stats.calls.fetch_add(1, Ordering::Relaxed);
    }
}

/// Map's DivisionModelInputs job since attach: the hours it read a census and
/// the rows whose orders group trained, the only ones it reads exercise operands for.
#[derive(Debug, Default)]
pub struct ModelInputStats {
    pub hours: AtomicU64,
    pub training: AtomicU64,
    pub training_peak: AtomicU64,
}
impl ModelInputStats {
    fn record(&self, training: u32) {
        self.hours.fetch_add(1, Ordering::Relaxed);
        self.training
            .fetch_add(u64::from(training), Ordering::Relaxed);
        self.training_peak
            .fetch_max(u64::from(training), Ordering::Relaxed);
    }
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

/// One pool role's collected jobs, as offsets from their acquisition body's
/// entry: when each was queued, when a pool participant began and ended its
/// body, and when the game thread began and ended its join. Sums of QPC
/// ticks; status reports the mean per job. Atomics only, written once per
/// collected job on the game thread.
pub struct RoleTimeline {
    /// Collected jobs, and those whose body a participant ran.
    jobs: AtomicU64,
    bodies: AtomicU64,
    /// Queued, body begin, body end, join begin, join end.
    offsets: [AtomicU64; 5],
    /// Joins whose wait fell in [0.25, 1), [1, 4) and [4, ∞) ms
    /// ([`Self::SLOW_JOIN_NS`]): one increment at most per join.
    slow: [AtomicU64; 3],
}

impl RoleTimeline {
    /// The join-wait thresholds the slow counts start at, in ns.
    pub const SLOW_JOIN_NS: [u64; 3] = [250_000, 1_000_000, 4_000_000];

    const fn new() -> Self {
        Self {
            jobs: AtomicU64::new(0),
            bodies: AtomicU64::new(0),
            offsets: [const { AtomicU64::new(0) }; 5],
            slow: [const { AtomicU64::new(0) }; 3],
        }
    }

    /// The joins whose wait reached each of [`Self::SLOW_JOIN_NS`].
    pub fn slow_joins(&self) -> [u64; 3] {
        let [quarter, one, four] = self
            .slow
            .each_ref()
            .map(|count| count.load(Ordering::Relaxed));
        [quarter + one + four, one + four, four]
    }

    fn record(
        &self,
        entry: i64,
        timeline: &crate::acquisition::Timeline,
        join: i64,
        joined: i64,
        wait_ns: u64,
    ) {
        if let Some(bucket) = Self::SLOW_JOIN_NS
            .iter()
            .rposition(|&floor| wait_ns >= floor)
        {
            self.slow[bucket].fetch_add(1, Ordering::Relaxed);
        }
        let offset = |qpc: i64| u64::try_from(qpc.saturating_sub(entry)).unwrap_or(0);
        let ran = timeline.body_begin_qpc != 0 && timeline.body_end_qpc != 0;
        for (index, qpc) in [
            (0, timeline.queued_qpc),
            (1, timeline.body_begin_qpc),
            (2, timeline.body_end_qpc),
            (3, join),
            (4, joined),
        ] {
            if ran || !matches!(index, 1 | 2) {
                self.offsets[index].fetch_add(offset(qpc), Ordering::Relaxed);
            }
        }
        self.jobs.fetch_add(1, Ordering::Relaxed);
        if ran {
            self.bodies.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(jobs, [queued, body begin, body end, join begin, join end])`, the
    /// mean offsets in ticks; the body means cover only the jobs that ran.
    pub fn means(&self) -> (u64, [u64; 5]) {
        let jobs = self.jobs.load(Ordering::Relaxed);
        let bodies = self.bodies.load(Ordering::Relaxed);
        let mean = |index: usize, count: u64| {
            self.offsets[index]
                .load(Ordering::Relaxed)
                .checked_div(count)
                .unwrap_or(0)
        };
        (
            jobs,
            [
                mean(0, jobs),
                mean(1, bodies),
                mean(2, bodies),
                mean(3, jobs),
                mean(4, jobs),
            ],
        )
    }
}

/// Where a pool role's joins spend their wall once the awaited job is done,
/// over every collected job: the collector's first wait for the queue mutex
/// when the initial lock-free collection CAS did not succeed,
/// the stall from the moment the job is both awaited and published to its
/// collection (the atomic handoff, address wake and rescheduling), and
/// the worker's completion bookkeeping after its body ended, before atomic
/// publication. Failure publication also stops/cancels under the queue mutex.
/// Diagnostics only; optional helped bodies are part of their caller's join.
pub struct JoinCosts {
    /// Collected jobs whose completion was published (canceled ones too).
    published: AtomicU64,
    /// Of those, the jobs already published when their join began.
    ready: AtomicU64,
    /// Summed ticks: first finish-lock wait, collection stall, publication wait.
    ticks: [AtomicU64; 3],
    /// Stalls of at least [`Self::STALL_NS`] (0.25 and 1 ms).
    stalls: [AtomicU64; 2],
}

impl JoinCosts {
    /// The collection-stall thresholds the stall counts start at, in ns.
    pub const STALL_NS: [u64; 2] = [250_000, 1_000_000];

    const fn new() -> Self {
        Self {
            published: AtomicU64::new(0),
            ready: AtomicU64::new(0),
            ticks: [const { AtomicU64::new(0) }; 3],
            stalls: [const { AtomicU64::new(0) }; 2],
        }
    }

    /// One collected job: `join` is when its join began; `ns` converts
    /// ticks for the stall thresholds. A job that no participant ran or that
    /// was never published (rejected or inline work) is not counted.
    fn record(
        &self,
        timeline: &crate::acquisition::Timeline,
        join: i64,
        ns: impl Fn(i64) -> u64,
    ) -> u64 {
        if timeline.published_qpc == 0 || timeline.collected_qpc == 0 {
            return 0;
        }
        let ticks = |from: i64, to: i64| u64::try_from(to.saturating_sub(from)).unwrap_or(0);
        let stall = ticks(join.max(timeline.published_qpc), timeline.collected_qpc);
        self.published.fetch_add(1, Ordering::Relaxed);
        if timeline.published_qpc <= join {
            self.ready.fetch_add(1, Ordering::Relaxed);
        }
        self.ticks[0].fetch_add(timeline.finish_lock_ticks, Ordering::Relaxed);
        self.ticks[1].fetch_add(stall, Ordering::Relaxed);
        if timeline.body_end_qpc != 0 {
            self.ticks[2].fetch_add(
                ticks(timeline.body_end_qpc, timeline.published_qpc),
                Ordering::Relaxed,
            );
        }
        let stall_ns = ns(i64::try_from(stall).unwrap_or(i64::MAX));
        for (count, floor) in self.stalls.iter().zip(Self::STALL_NS) {
            if stall_ns >= floor {
                count.fetch_add(1, Ordering::Relaxed);
            }
        }
        stall_ns
    }

    /// `(published jobs, of them ready at their join, [finish-lock wait,
    /// collection stall, publication wait] summed ticks, [stalls of at least
    /// 0.25 ms, at least 1 ms])`.
    pub fn totals(&self) -> (u64, u64, [u64; 3], [u64; 2]) {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        (
            load(&self.published),
            load(&self.ready),
            self.ticks.each_ref().map(load),
            self.stalls.each_ref().map(load),
        )
    }
}

/// Free frames, taken newest first: an hour takes the frame the spool returned last, whose
/// held stores are the previous hour's and whose memory is still cached.
pub struct FreeFrames {
    /// Bit `index` is set while that frame is free.
    free: AtomicU64,
    /// When each frame was last returned; an idle lease keeps its frame's.
    returned: [AtomicU64; BUFFERS],
    returns: AtomicU64,
}

const _: () = assert!(BUFFERS <= 64);

impl FreeFrames {
    pub const fn new() -> Self {
        Self {
            free: AtomicU64::new(0),
            returned: [const { AtomicU64::new(0) }; BUFFERS],
            returns: AtomicU64::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.free.load(Ordering::Acquire).count_ones() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Frees a returned frame as the newest.
    pub fn push(&self, index: usize) -> Result<(), usize> {
        if index >= BUFFERS {
            return Err(index);
        }
        let stamp = self.returns.fetch_add(1, Ordering::Relaxed) + 1;
        self.returned[index].store(stamp, Ordering::Relaxed);
        self.restore(index)
    }

    /// Frees a leased frame again, as old as it was when taken.
    pub fn restore(&self, index: usize) -> Result<(), usize> {
        if index >= BUFFERS || self.free.fetch_or(1 << index, Ordering::Release) & (1 << index) != 0
        {
            return Err(index);
        }
        Ok(())
    }

    /// The newest free frame.
    pub fn pop(&self) -> Option<usize> {
        let mut free = self.free.load(Ordering::Acquire);
        loop {
            let index = (0..BUFFERS)
                .filter(|&index| free & (1 << index) != 0)
                .max_by_key(|&index| self.returned[index].load(Ordering::Relaxed))?;
            match self.free.compare_exchange_weak(
                free,
                free & !(1 << index),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(index),
                Err(actual) => free = actual,
            }
        }
    }

    /// Takes `index` if it is free, and with `keep_one` only while another frame stays free.
    pub fn take(&self, index: usize, keep_one: bool) -> bool {
        if index >= BUFFERS {
            return false;
        }
        let bit = 1_u64 << index;
        self.free
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |free| {
                (free & bit != 0 && (!keep_one || free.count_ones() >= 2)).then_some(free & !bit)
            })
            .is_ok()
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
    /// QPC at the current acquisition body's entry, the reference of
    /// [`Self::role_timeline`].
    pub acquisition_entry_qpc: AtomicI64,
    /// Each pool role's collected jobs on the timeline of their hour,
    /// indexed by `acquisition::ROLES`.
    pub role_timeline: [RoleTimeline; acquisition::ROLE_COUNT],
    /// Each pool role's collection costs after its jobs finished, indexed by
    /// `acquisition::ROLES`.
    pub join_costs: [JoinCosts; acquisition::ROLE_COUNT],
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
    /// The session's capture mode, set before hook publication.
    pub capture_mode: replay_data::scope::CaptureMode,
    /// The recorder's event, set while the game is not ticking, which its packing waits on.
    pub tick_idle: Option<crate::runtime::Event>,
    pub model_inputs: ModelInputStats,
    /// Every slot is allocated before hook installation and never moves.
    pub frames: [AtomicPtr<Captured>; BUFFERS],
    pub completed: [AtomicI64; BUFFERS],
    pub callback_tail: [AtomicI64; BUFFERS],
    pub(crate) history_frames: crate::frame_fence::Frames,
    frame_return: Mutex<()>,
    frame_available: Condvar,
    pub cancel_buffer_wait: AtomicBool,
    pub buffer_wait_ns: AtomicU64,
    pub buffer_wait_calls: AtomicU64,
    pub buffer_wait_max_ns: AtomicU64,
    pub free: FreeFrames,
    pub(crate) frame_priority: Arc<crate::frame_priority::Pipeline>,
    pub ready: ArrayQueue<usize>,
    /// Single FIFO decode queue between the game thread and the spool ready
    /// queue. Every frame transits it in hour order (lightweight jobs; raw
    /// payloads are bounded recycled boxes, never pool memory).
    pub decode_queue: decode::DecodeQueue,
    pub dropped: AtomicU64,
    /// Reserved collector storage outside the frames; executor bytes use real
    /// allocation sizes and capacities, including synchronization and enabled
    /// help histories. [`Shared::pool_owned`] adds the frames.
    pub pool_bytes: Arc<AtomicU64>,
    /// Each frame's measured bytes, stored by its exclusive holder whenever it
    /// recycles, grows or prepares it: a gauge, never a running delta.
    pub frame_bytes: [AtomicU64; BUFFERS],
    pub recording: AtomicBool,
    pub stop_spool: AtomicBool,
    /// The spool thread, set once when its loop starts. Its idle wait parks
    /// until a ready buffer, a parked clock preparation or a stop request
    /// unparks it ([`Shared::wake_spool`]), or its next periodic deadline.
    pub spool_waker: OnceLock<std::thread::Thread>,
    pub(crate) decode_waker: OnceLock<std::thread::Thread>,
    /// The game thread owns this until ready is published; the spool thread
    /// then takes it once, independent of any particular FIFO buffer.
    pub roster: UnsafeCell<Option<Box<Roster>>>,
    pub roster_ready: AtomicBool,
    roster_sampling: AtomicBool,
    /// Only the game thread writes this, inside the hook. The spool worker takes
    /// it after the hook is quiescent.
    pub pending: UnsafeCell<LossBatch>,
    clock_capture: Mutex<ClockCapture>,
    clock_available: Condvar,
    clock_required: AtomicUsize,
    clock_misses: AtomicU64,
    clock_hint_misses: AtomicU64,
    clock_failure: Mutex<Option<&'static str>>,
    cadence: UnsafeCell<CadenceState>,
    /// One store per sliced family (`slicing::FAMILIES`), loaned to that
    /// family's job each hour. Only the game thread takes and returns them.
    slice_fingerprints: [UnsafeCell<Option<Box<crate::slicing::SliceFingerprints>>>;
        crate::slicing::FAMILIES.len()],
    /// Fresh country authentication for this quiescent callback. Game thread only.
    country_cache: UnsafeCell<Arc<Cache>>,
    /// The carried province weather records, loaned to the Weather job each
    /// hour ([`extended::weather::carry`]). Only the game thread takes and
    /// returns it.
    weather_carry: UnsafeCell<Option<Box<extended::weather::carry::WeatherCarry>>>,
    /// The held RaidAirScores table, loaned to the Late job each hour. Only
    /// the game thread takes and returns it.
    raid_air_scores: UnsafeCell<Option<Box<engine_raid_inputs::raid_air_scores::Held>>>,
    raid_air_score_bytes: AtomicU64,
    /// Held CharacterRoleTraits role rows, loaned to the Gated job each hour
    /// with the roster job's role census. Only the game thread takes and
    /// returns it.
    character_role_store: UnsafeCell<Option<Box<extended::character_inputs::RoleStore>>>,
    character_role_store_bytes: AtomicU64,
    /// Held ScriptFlags rows, loaned to the IntelResidual job each hour. Only
    /// the game thread takes and returns it.
    script_flag_store: UnsafeCell<Option<Box<extended::intel::FlagStore>>>,
    script_flag_store_bytes: AtomicU64,
    /// Held ScriptVariables rows, loaned to the IntelResidual job each hour.
    /// Only the game thread takes and returns it.
    script_variable_store: UnsafeCell<Option<Box<extended::intel::VariableStore>>>,
    script_variable_store_bytes: AtomicU64,
    faction_goal_store: UnsafeCell<Option<Box<crate::faction_goals::Store>>>,
    faction_goal_store_bytes: AtomicU64,
    faction_companion_store: UnsafeCell<Option<Box<crate::faction_companions::Store>>>,
    faction_companion_store_bytes: AtomicU64,
    /// Held StateBuildings and ProvinceBuildings rows, loaned to the
    /// Governance job each hour. Only the game thread takes and returns it.
    building_store: UnsafeCell<Option<Box<extended::construction::BuildingStore>>>,
    building_store_bytes: AtomicU64,
    /// Held PartyLeaders rows, loaned to the Governance job each hour. Only
    /// the game thread takes and returns it.
    leader_store: UnsafeCell<Option<Box<extended::governance::LeaderStore>>>,
    leader_store_bytes: AtomicU64,
    /// Held PoliticalParties and ActiveIdeas rows, loaned to the Governance
    /// job each hour. Only the game thread takes and returns it.
    party_store: UnsafeCell<Option<Box<extended::governance::PartyStore>>>,
    party_store_bytes: AtomicU64,
    /// Held WingEquipment rows, lent to the AirWing job's payload each hour.
    /// Only the game thread takes and returns it.
    wing_store: UnsafeCell<Option<Box<extended::air::WingStore>>>,
    wing_store_bytes: AtomicU64,
    /// The AirWing census parts after the first, each with its own held
    /// WingEquipment rows, lent to their jobs each hour, and where the next
    /// hour's parts start ([`extended::air::part_starts`]). Only the game
    /// thread takes, returns, joins and splits them.
    air_wing_parts: UnsafeCell<[extended::air::AirWingJob; extended::air::WING_PARTS - 1]>,
    air_wing_part_starts: UnsafeCell<([u32; extended::air::WING_PARTS - 1], usize)>,
    /// The production modifier capture's split, from the last slot hour.
    production_part_starts:
        UnsafeCell<([u32; extended::production_modifiers::MAX_PARTS - 1], usize)>,
    /// Held AirBaseRegistrations subtrees, lent to the AirBases job each
    /// hour. Only the game thread takes and returns it.
    air_base_store: UnsafeCell<Option<Box<extended::air::RegistrationStore>>>,
    air_base_store_bytes: AtomicU64,
    /// Held AirRegionalMissions registrations, lent to the AirMissions job
    /// each hour. Only the game thread takes and returns it.
    mission_store: UnsafeCell<Option<Box<extended::air::MissionStore>>>,
    mission_store_bytes: AtomicU64,
    /// Held RailCooldowns inputs and RailEdges rows, lent to the Late job
    /// each hour. Only the game thread takes and returns it.
    rail_store: UnsafeCell<Option<Box<extended::naval_history::RailStore>>>,
    rail_store_bytes: AtomicU64,
    /// Held States rows, lent to the States job each hour. Only the game
    /// thread takes and returns it.
    state_store: UnsafeCell<Option<Box<acquisition_states::StateStore>>>,
    state_store_bytes: AtomicU64,
    /// One acquisition store is lent across frame-buffer rotation; frames retain immutable handles.
    occupation_store: UnsafeCell<Option<Box<acquisition_states::OccupationStore>>>,
    occupation_store_bytes: AtomicU64,
    /// Held WarRelations rows, lent to the Wars job each hour. Only the game
    /// thread takes and returns it.
    war_store: UnsafeCell<Option<Box<acquisition_wars::WarStore>>>,
    war_store_bytes: AtomicU64,
    /// Held DivisionStatus attack operands, lent to the Presentation job
    /// each hour. Acquisition takes it; FIFO decode returns it after settlement.
    army_stat_store: Option<Arc<extended::division_presentation::held_store::Slot>>,
    army_stat_store_bytes: AtomicU64,
    /// The censuses of the division input parts, lent to their jobs each
    /// hour, and where the next hour's parts start
    /// ([`crate::division_census::Census::part_starts`]). Only the game
    /// thread takes, returns and splits them.
    division_parts: UnsafeCell<
        [Option<Box<crate::division_census::Census>>; crate::division_census::MAX_PARTS - 1],
    >,
    division_part_starts: UnsafeCell<([u32; crate::division_census::MAX_PARTS - 1], usize)>,
    /// The order census split the last stitched hour planned for the next
    /// ([`crate::orders_parts::Split`]). Only the game thread reads and
    /// writes it.
    order_split: UnsafeCell<crate::orders_parts::Split>,
    /// The joined census's storage the next hour's inputs job reserves, so
    /// the join copies without allocating
    /// ([`crate::division_census::Census::join_capacity`]).
    division_join_capacity: UnsafeCell<crate::division_census::JoinCapacity>,
    /// The held ProvinceUnitOrder walk, lent to its job each hour. Only the
    /// game thread takes and returns it.
    province_unit_store: UnsafeCell<Option<Box<extended::province_unit_order::ProvinceUnitStore>>>,
    province_unit_store_bytes: AtomicU64,
    /// Held controller and owner areas, lent to the Provinces job each hour
    /// ([`crate::area_changes`]). Only the game thread takes and returns it.
    area_store: UnsafeCell<Option<Box<engine_native::controller_areas::Store>>>,
    area_store_bytes: AtomicU64,
    /// Held Provinces rows, lent to the Provinces job each hour
    /// ([`crate::province_changes`]). Only the game thread takes and returns it.
    province_store: UnsafeCell<Option<Box<engine_native::cprovince::carry::RowStore>>>,
    province_store_bytes: AtomicU64,
    /// Held AvailableAdvisors and AdvisorTokens rows, loaned to the Slow job
    /// each hour. Only the game thread takes and returns it.
    advisor_store: UnsafeCell<Option<Box<extended::hired_advisors::AdvisorStore>>>,
    advisor_store_bytes: AtomicU64,
    /// Held CountryLocalSubunits rows, loaned to the Slow job each hour.
    /// Only the game thread takes and returns it.
    local_subunit_store: UnsafeCell<Option<Box<extended::model_inputs::LocalStore>>>,
    local_subunit_store_bytes: AtomicU64,
    doctrine_store: UnsafeCell<Option<Box<Mutex<collector_slow_tables::doctrines::Store>>>>,
    doctrine_store_bytes: AtomicU64,
    /// Held CountryCounterInputs rows, loaned to the Ordinary counter part
    /// each hour. Only the game thread takes and returns it.
    counter_store: UnsafeCell<Option<Box<extended::country_counters::CounterStore>>>,
    counter_store_bytes: AtomicU64,
    /// Held CountryAppearance rows, loaned to the Countries job each hour.
    /// Only the game thread takes and returns it.
    appearance_store: UnsafeCell<Option<Box<extended::country_appearance::AppearanceStore>>>,
    appearance_store_bytes: AtomicU64,
    pub presentation: PresentationStats,
    pub game_thread: GameThreadPhases,
    /// Template rows whose model inputs failed; the status thread copies it.
    pub template_models: Mutex<TemplateModelStats>,
    /// Hours whose failures were not folded because the status thread held
    /// `template_models`: the game thread never waits for it.
    pub template_models_missed: AtomicU64,
    /// Capture recovery after lost frame continuity; the next hour also
    /// restarts native interval observation.
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
    pub fixed_layout: OnceLock<engine_fixed_layout::fixed_layout::Report>,
    /// Prepared before hook publication and borrowed only by the hourly callback.
    pub fixed_layout_buffer: UnsafeCell<Vec<u8>>,
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
    /// The division template stores every frame's capture takes; counted once in
    /// `pool_bytes`, at their high-water mark.
    pub template_home: crate::extended::division_templates::Home,
    /// The province combat walk every frame's hourly capture takes; counted once in
    /// `pool_bytes`, at its high-water mark.
    pub naval_combat_home: crate::extended::naval_combat_sources::Home,
    /// The weather temperature pass across hours; only the single FIFO decode
    /// worker locks it, in hour order.
    pub weather_temperature: Mutex<extended::weather::temperature::Tracker>,
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

impl crate::design_changes::ScratchPoolAccounting for Shared {
    fn replace_scratch_bytes(&self, previous: usize, next: usize) {
        if next >= previous {
            self.pool_bytes
                .fetch_add((next - previous) as u64, Ordering::Relaxed);
        } else {
            self.pool_bytes
                .fetch_sub((previous - next) as u64, Ordering::Relaxed);
        }
    }
}

impl Shared {
    /// A published table shortage needs fresh acquisition, while its native
    /// mutation observers retain the interval they have already captured.
    pub(crate) fn request_retry_full(&self) {
        self.refresh_next.store(true, Ordering::Release);
    }

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
        #[cfg(test)]
        let fixed_layout_bytes = if engine_executable::executable_tables::is_initialized() {
            engine_fixed_layout::fixed_layout::longest_name()
        } else {
            0
        };
        #[cfg(not(test))]
        let fixed_layout_bytes = engine_fixed_layout::fixed_layout::longest_name();
        let fixed_layout_buffer = engine_allocation::default_vec::<u8>(fixed_layout_bytes)?;
        let roster = engine_allocation::boxed(Roster::with_counts(sizing.content))?;
        let country_cache = Arc::new(Cache::try_with_capacity(sizing.country_cells)?);
        crate::faction_goal_changes::prime();
        crate::faction_companion_changes::prime();
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
        let template_home = first
            .extended
            .template_home()
            .expect("a new frame holds its template home");
        let naval_combat_home = first
            .extended
            .naval_combat_home()
            .expect("a new frame holds its province combat home");
        let air = &sizing.extended.air;
        let mut raid_air_scores = Box::new(engine_raid_inputs::raid_air_scores::Held::default());
        raid_air_scores.grow(air.scores.regions, air.scores.countries, air.scores.rows)?;
        let raid_air_score_bytes = raid_air_scores.allocated_bytes();
        let pool_bytes = roster.allocated_bytes()
            + country_cache.allocated_bytes()
            + country_intel.allocated_bytes()
            + raid_air_score_bytes
            + fixed_layout_buffer.capacity();
        let character_role_store = cfg!(feature = "character_changes")
            .then(|| Box::new(extended::character_inputs::RoleStore::new()));
        let script_flag_store =
            cfg!(feature = "flag_changes").then(|| Box::new(extended::intel::FlagStore::new()));
        let script_variable_store = cfg!(feature = "variable_changes")
            .then(|| Box::new(extended::intel::VariableStore::new()));
        let faction_goal_store = cfg!(feature = "faction_changes")
            .then(|| engine_allocation::boxed(crate::faction_goals::Store::default()))
            .transpose()?;
        let faction_companion_store = cfg!(feature = "faction_changes")
            .then(|| engine_allocation::boxed(crate::faction_companions::Store::default()))
            .transpose()?;
        let building_store = cfg!(feature = "building_changes")
            .then(|| Box::new(extended::construction::BuildingStore::new()));
        let leader_store = cfg!(feature = "party_leader_changes")
            .then(|| Box::new(extended::governance::LeaderStore::new()));
        let party_store = cfg!(feature = "party_changes")
            .then(|| Box::new(extended::governance::PartyStore::new()));
        let wing_store =
            cfg!(feature = "wing_changes").then(|| Box::new(extended::air::WingStore::new()));
        let air_base_store = cfg!(feature = "air_base_carry_changes")
            .then(|| Box::new(extended::air::RegistrationStore::new()));
        let mission_store =
            cfg!(feature = "mission_changes").then(|| Box::new(extended::air::MissionStore::new()));
        let rail_store = cfg!(feature = "rail_changes")
            .then(|| Box::new(extended::naval_history::RailStore::new()));
        let state_store = cfg!(feature = "territory_changes").then(|| {
            Box::new(acquisition_states::StateStore::new(
                &crate::territory_changes::Observer,
            ))
        });
        let war_store =
            cfg!(feature = "war_changes").then(|| Box::new(acquisition_wars::WarStore::default()));
        let advisor_store = cfg!(feature = "advisor_pool_changes")
            .then(|| Box::new(extended::hired_advisors::AdvisorStore::new()));
        let local_subunit_store = cfg!(feature = "local_subunit_changes")
            .then(|| Box::new(extended::model_inputs::LocalStore::default()));
        let doctrine_store = Box::new(Mutex::new(
            collector_slow_tables::doctrines::Store::default(),
        ));
        // SAFETY: Shared owns this stable box through hook restoration and
        // native callback quiescence; loans move the box without moving it.
        unsafe {
            collector_observers::doctrine_changes::bind(
                (&*doctrine_store as *const Mutex<collector_slow_tables::doctrines::Store>)
                    as usize,
                collector_slow_tables::doctrines::observe,
            );
            collector_observers::doctrine_changes::bind_cost_consumption(
                collector_slow_tables::doctrines::observe_cost_consumption,
            );
            collector_observers::doctrine_changes::bind_gain(
                collector_slow_tables::doctrines::observe_gain,
            );
        }
        let counter_store = cfg!(feature = "counter_changes")
            .then(|| Box::new(extended::country_counters::CounterStore::observed()));
        let appearance_store = cfg!(feature = "appearance_changes")
            .then(|| Box::new(extended::country_appearance::AppearanceStore::default()));
        let frame_priority = Arc::new(crate::frame_priority::Pipeline::new());
        let army_stat_store = cfg!(feature = "army_stat_changes").then(|| {
            let slot = extended::division_presentation::held_store::Slot::new(Box::new(
                extended::attack_stats::ArmyStatStore::new(),
            ));
            slot.set_priority(Arc::clone(&frame_priority));
            slot
        });
        let province_unit_store = crate::province_unit_changes::CARRY.then(|| {
            Box::new(extended::province_unit_order::ProvinceUnitStore::new(
                &crate::province_unit_changes::Observer,
            ))
        });
        let area_store = cfg!(feature = "area_changes")
            .then(|| Box::new(engine_native::controller_areas::Store::new()));
        let province_store = cfg!(feature = "province_changes")
            .then(|| Box::new(engine_native::cprovince::carry::RowStore::new()));
        let mut slice_fingerprints = std::array::from_fn(|_| UnsafeCell::new(None));
        for (family, cell) in slice_fingerprints.iter_mut().enumerate() {
            if !matches!(
                family,
                crate::slicing::AIR_RELATIONS | crate::slicing::AIR_HISTORIES
            ) {
                *cell.get_mut() = Some(Box::new(crate::slicing::SliceFingerprints::new()));
            }
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
            acquisition_entry_qpc: AtomicI64::new(0),
            role_timeline: std::array::from_fn(|_| RoleTimeline::new()),
            join_costs: std::array::from_fn(|_| JoinCosts::new()),
            acquisition_dispatch_ns: AtomicU64::new(0),
            acquisition_wait_ns: AtomicU64::new(0),
            native_tick_ns: AtomicU64::new(0),
            native_tick_calls: AtomicU64::new(0),
            base,
            trampoline,
            frequency,
            game_score_enabled: sizing.extended.game_score.enabled,
            capture_mode: replay_data::scope::CaptureMode::Full,
            tick_idle: None,
            model_inputs: ModelInputStats::default(),
            frames: std::array::from_fn(|_| AtomicPtr::new(ptr::null_mut())),
            frame_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            completed: std::array::from_fn(|_| AtomicI64::new(0)),
            callback_tail: std::array::from_fn(|_| AtomicI64::new(0)),
            history_frames: crate::frame_fence::Frames::default(),
            frame_return: Mutex::new(()),
            frame_available: Condvar::new(),
            cancel_buffer_wait: AtomicBool::new(false),
            buffer_wait_ns: AtomicU64::new(0),
            buffer_wait_calls: AtomicU64::new(0),
            buffer_wait_max_ns: AtomicU64::new(0),
            free: FreeFrames::new(),
            frame_priority,
            ready: ArrayQueue::new(BUFFERS),
            decode_queue: decode::new_queue(),
            dropped: AtomicU64::new(0),
            pool_bytes: Arc::new(AtomicU64::new(
                (pool_bytes
                    + size_of::<AtomicU64>()
                    + 2 * size_of::<usize>()
                    + size_of::<crate::frame_priority::Pipeline>()
                    + 2 * size_of::<usize>()) as u64,
            )),
            recording: AtomicBool::new(true),
            stop_spool: AtomicBool::new(false),
            spool_waker: OnceLock::new(),
            decode_waker: OnceLock::new(),
            roster: UnsafeCell::new(Some(roster)),
            roster_ready: AtomicBool::new(false),
            roster_sampling: AtomicBool::new(false),
            pending: UnsafeCell::new(LossBatch::default()),
            clock_capture: Mutex::new(ClockCapture::default()),
            clock_available: Condvar::new(),
            clock_required: AtomicUsize::new(0),
            clock_misses: AtomicU64::new(0),
            clock_hint_misses: AtomicU64::new(0),
            clock_failure: Mutex::new(None),
            cadence: UnsafeCell::new(CadenceState::default()),
            // The air families' stores belong to their decode steps.
            slice_fingerprints,
            country_cache: UnsafeCell::new(country_cache),
            raid_air_scores: UnsafeCell::new(Some(raid_air_scores)),
            weather_carry: UnsafeCell::new(Some(Box::default())),
            raid_air_score_bytes: AtomicU64::new(raid_air_score_bytes as u64),
            character_role_store: UnsafeCell::new(character_role_store),
            character_role_store_bytes: AtomicU64::new(0),
            script_flag_store: UnsafeCell::new(script_flag_store),
            script_flag_store_bytes: AtomicU64::new(0),
            script_variable_store: UnsafeCell::new(script_variable_store),
            script_variable_store_bytes: AtomicU64::new(0),
            faction_goal_store: UnsafeCell::new(faction_goal_store),
            faction_goal_store_bytes: AtomicU64::new(0),
            faction_companion_store: UnsafeCell::new(faction_companion_store),
            faction_companion_store_bytes: AtomicU64::new(0),
            building_store: UnsafeCell::new(building_store),
            building_store_bytes: AtomicU64::new(0),
            leader_store: UnsafeCell::new(leader_store),
            leader_store_bytes: AtomicU64::new(0),
            party_store: UnsafeCell::new(party_store),
            party_store_bytes: AtomicU64::new(0),
            wing_store: UnsafeCell::new(wing_store),
            wing_store_bytes: AtomicU64::new(0),
            air_wing_parts: UnsafeCell::new(std::array::from_fn(|_| {
                extended::air::AirWingJob::part()
            })),
            air_wing_part_starts: UnsafeCell::new(([0; extended::air::WING_PARTS - 1], 0)),
            production_part_starts: UnsafeCell::new((
                [0; extended::production_modifiers::MAX_PARTS - 1],
                0,
            )),
            air_base_store: UnsafeCell::new(air_base_store),
            air_base_store_bytes: AtomicU64::new(0),
            mission_store: UnsafeCell::new(mission_store),
            mission_store_bytes: AtomicU64::new(0),
            rail_store: UnsafeCell::new(rail_store),
            rail_store_bytes: AtomicU64::new(0),
            state_store: UnsafeCell::new(state_store),
            state_store_bytes: AtomicU64::new(0),
            occupation_store: UnsafeCell::new(Some(Box::default())),
            occupation_store_bytes: AtomicU64::new(0),
            war_store: UnsafeCell::new(war_store),
            war_store_bytes: AtomicU64::new(0),
            army_stat_store,
            army_stat_store_bytes: AtomicU64::new(0),
            division_parts: UnsafeCell::new(std::array::from_fn(|_| {
                Some(Box::new(crate::division_census::Census::new_worker()))
            })),
            division_part_starts: UnsafeCell::new(([0; crate::division_census::MAX_PARTS - 1], 0)),
            order_split: UnsafeCell::new(crate::orders_parts::Split::default()),
            division_join_capacity: UnsafeCell::new(Default::default()),
            province_unit_store: UnsafeCell::new(province_unit_store),
            province_unit_store_bytes: AtomicU64::new(0),
            area_store: UnsafeCell::new(area_store),
            area_store_bytes: AtomicU64::new(0),
            province_store: UnsafeCell::new(province_store),
            province_store_bytes: AtomicU64::new(0),
            advisor_store: UnsafeCell::new(advisor_store),
            advisor_store_bytes: AtomicU64::new(0),
            local_subunit_store: UnsafeCell::new(local_subunit_store),
            local_subunit_store_bytes: AtomicU64::new(0),
            doctrine_store: UnsafeCell::new(Some(doctrine_store)),
            doctrine_store_bytes: AtomicU64::new(0),
            counter_store: UnsafeCell::new(counter_store),
            counter_store_bytes: AtomicU64::new(0),
            appearance_store: UnsafeCell::new(appearance_store),
            appearance_store_bytes: AtomicU64::new(0),
            presentation: PresentationStats::default(),
            game_thread: GameThreadPhases::default(),
            template_models: Mutex::new(TemplateModelStats::default()),
            template_models_missed: AtomicU64::new(0),
            force_next: AtomicBool::new(false),
            refresh_next: AtomicBool::new(false),
            forget_families: std::sync::atomic::AtomicU32::new(0),
            lexical_max: AtomicI32::new(-1),
            fixed_layout: OnceLock::new(),
            fixed_layout_buffer: UnsafeCell::new(fixed_layout_buffer),
            modifier_table: [AtomicU64::new(u64::MAX), AtomicU64::new(u64::MAX)],
            division_mark: AtomicU64::new(extended::new_divisions::UNKNOWN),
            country_cache_swap: Mutex::new(CountryCacheSwap::default()),
            country_cache_cells: AtomicUsize::new(country_cells),
            country_intel,
            template_home: std::sync::Arc::clone(&template_home),
            naval_combat_home: std::sync::Arc::clone(&naval_combat_home),
            weather_temperature: Mutex::new(Default::default()),
        };
        let mut borrowed = sizing.extended;
        borrowed.intel.rows[15] = 0;
        borrowed.intel.intel_scratch = 0;
        // No callback or spool thread can observe Shared during construction.
        unsafe { shared.publish_frame(0, Box::new(first)) };
        for index in 1..BUFFERS {
            let mut frame = Captured::with_capacities(&sizing.core, &sizing.map, &borrowed)?;
            frame.extended.share_country_intel(&shared.country_intel);
            frame.extended.share_template_home(&template_home);
            frame.extended.share_naval_combat_home(&naval_combat_home);
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
        self.frame_bytes[index].store(frame.allocated_bytes() as u64, Ordering::Relaxed);
        // Acquire after popping a free index makes the entire allocation visible.
        self.frames[index].store(Box::into_raw(frame), Ordering::Release);
        // A newly allocated slot has no index in either queue. Both queues have
        // room for every allocated slot, including ones currently held by a thread.
        self.frame_priority.publish(index);
        let published = self.free.push(index);
        debug_assert!(published.is_ok());
    }

    /// The callback retains the hourly boundary while an existing frame drains.
    /// The common path is only a queue pop; no native reads or allocations are
    /// repeated while waiting. Loader cancellation prevents shutdown deadlocks.
    /// Writes this call's wait to `wait_ns`, zero on the common path.
    pub fn take_frame(&self, wait_ns: &mut u64) -> Option<usize> {
        *wait_ns = 0;
        if self.acquisition_failed() {
            return None;
        }
        if let Some(index) = self.free.pop() {
            self.frame_priority
                .claim(index, crate::frame_priority::Caller::Callback);
            return Some(index);
        }
        let started = qpc();
        self.frame_priority
            .refresh(crate::frame_priority::Caller::Callback);
        self.buffer_wait_calls.fetch_add(1, Ordering::Relaxed);
        let mut returned = self.frame_return.lock().unwrap_or_else(|e| e.into_inner());
        let index = loop {
            if let Some(index) = self.free.pop() {
                break Some(index);
            }
            if self.cancel_buffer_wait.load(Ordering::Acquire) {
                break None;
            }
            self.frame_priority
                .refresh(crate::frame_priority::Caller::Callback);
            // Cancellation must work even if a failed consumer cannot notify us.
            returned = self
                .frame_available
                .wait_timeout(returned, std::time::Duration::from_millis(5))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        };
        drop(returned);
        if let Some(index) = index {
            self.frame_priority
                .claim(index, crate::frame_priority::Caller::Callback);
        } else {
            self.frame_priority
                .refresh(crate::frame_priority::Caller::Callback);
        }
        let elapsed = self.ns(qpc() - started);
        self.buffer_wait_ns.fetch_add(elapsed, Ordering::Relaxed);
        self.buffer_wait_max_ns
            .fetch_max(elapsed, Ordering::Relaxed);
        *wait_ns = elapsed;
        index
    }

    /// An idle frame while another stays free for the game's next hour.
    pub(crate) fn take_idle_frame_for_growth(&self, index: usize) -> bool {
        if !self.free.take(index, true) {
            return false;
        }
        self.frame_priority
            .claim(index, crate::frame_priority::Caller::Worker);
        true
    }

    /// End the spool loop's idle wait. Allocation-free and lock-free, so the
    /// game thread may call it; a wake before the wait begins is kept and ends
    /// that wait at once, so none is lost.
    pub(crate) fn wake_spool(&self) {
        if let Some(spool) = self.spool_waker.get() {
            spool.unpark();
        }
    }

    /// The legacy sizing verdict; frame priority promotes one frame earlier.
    pub fn buffers_backlogged(&self) -> bool {
        self.free.len() <= 1
    }

    pub fn acquisition_failed(&self) -> bool {
        crate::allocation::failed() || self.acquisition_panics.load(Ordering::Acquire) != 0
    }

    /// Requires the frame's exclusive lease. Its whole measurement replaces the
    /// last, so growth on any thread, loans and resets can never drift the total.
    pub(crate) fn measure_frame(&self, index: usize) {
        let frame = unsafe { &*self.frames[index].load(Ordering::Acquire) };
        self.frame_bytes[index].store(frame.allocated_bytes() as u64, Ordering::Relaxed);
    }

    /// The out-of-frame stores plus every frame as last measured.
    pub(crate) fn pool_owned(&self) -> u64 {
        self.frame_bytes
            .iter()
            .map(|bytes| bytes.load(Ordering::Relaxed))
            .fold(self.pool_bytes.load(Ordering::Relaxed), u64::wrapping_add)
    }

    /// Reserved collector storage: [`Self::pool_owned`] plus the held air
    /// history rings, which their owner measures.
    pub(crate) fn pool_total(&self) -> u64 {
        self.pool_owned()
            .wrapping_add(crate::extended::air_histories::retained_bytes() as u64)
    }

    /// Requires the frame's exclusive queue lease; clock preparation may allocate.
    pub(crate) fn prepare_frame_for_reuse(&self, index: usize) {
        let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
        frame.reset_verdicts();
        frame.verdicts_reset = true;
        let result = frame.prepare_clock(self.clock_required());
        self.measure_frame(index);
        if result.is_err() {
            self.cancel_buffer_wait.store(true, Ordering::Release);
            self.recording.store(false, Ordering::Release);
        }
    }

    pub(crate) fn recycle_frame_prepared(&self, index: usize) {
        self.prepare_frame_for_reuse(index);
        self.recycle_frame(index);
    }

    /// Frees an idle frame again, no newer than when it was taken.
    pub(crate) fn restore_idle_frame_prepared(&self, index: usize) {
        self.prepare_frame_for_reuse(index);
        self.return_frame(index, false);
    }

    /// Publish all completed writes before waking a callback waiting for reuse.
    pub fn recycle_frame(&self, index: usize) {
        self.return_frame(index, true);
    }

    fn return_frame(&self, index: usize, newest: bool) {
        self.history_frames.recycle(index);
        // The caller owns this queue lease; its loss witnesses were consumed or restored.
        unsafe {
            (&mut *self.frames[index].load(Ordering::Acquire))
                .dropped_before
                .clear();
        }
        let _returned = self.frame_return.lock().unwrap_or_else(|e| e.into_inner());
        self.frame_priority.publish(index);
        let published = if newest {
            self.free.push(index)
        } else {
            self.free.restore(index)
        };
        debug_assert!(published.is_ok());
        self.frame_available.notify_one();
        drop(_returned);
        self.frame_priority
            .refresh(crate::frame_priority::Caller::Worker);
    }

    fn take_army_stat_store(&self) -> Option<extended::division_presentation::held_store::Loan> {
        let begin = qpc();
        let store = self.army_stat_store.as_ref().and_then(|slot| slot.take());
        self.presentation
            .store_wait_ns
            .fetch_add(self.ns(qpc() - begin), Ordering::Relaxed);
        store
    }

    pub(crate) fn return_army_stat_store(
        &self,
        mut store: extended::division_presentation::held_store::Loan,
    ) {
        store.prepare_return();
        let bytes = store.allocated_bytes() as u64;
        let previous = self.army_stat_store_bytes.swap(bytes, Ordering::Relaxed);
        self.pool_bytes
            .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
        drop(store);
    }

    /// Fold one hour's Presentation pass split into the cumulative counters.
    fn record_presentation(&self, ticks: [u64; extended::division_presentation::PHASES]) {
        self.presentation.hours.fetch_add(1, Ordering::Relaxed);
        self.record_owned_presentation(ticks);
    }

    /// Add post-resume phase work without counting the same hour twice.
    pub(crate) fn record_owned_presentation(
        &self,
        ticks: [u64; extended::division_presentation::PHASES],
    ) {
        for (slot, ticks) in self.presentation.phase_ns.iter().zip(ticks) {
            slot.fetch_add(self.ns(ticks as i64), Ordering::Relaxed);
        }
    }

    pub fn ns(&self, ticks: i64) -> u64 {
        ((ticks.max(0) as u128 * 1_000_000_000) / self.frequency as u128) as u64
    }

    /// Loader/spool only. The clock gate excludes native callback mutations of
    /// cadence and pending losses; frame queue leases exclude their readers.
    pub(crate) fn prepare_clock_capture(
        &self,
        memory: &impl engine_native::Memory,
    ) -> std::io::Result<()> {
        let native = cgame_state::campaign_id_len(memory, self.base).unwrap_or(0);
        let Ok(mut capture) = self.clock_capture.try_lock() else {
            return Ok(());
        };
        let requested = self.clock_required.load(Ordering::Acquire);
        let required = requested.max(native);
        if capture.prepared >= required {
            return Ok(());
        }
        if requested > capture.prepared {
            // A parked original callback needs this destination before it can read.
            self.grow_clock_capture(&mut capture, required)
        } else {
            // Startup/wall-clock polling is optional preparation, never terminal.
            if engine_allocation::advisory(|| self.grow_clock_capture(&mut capture, required))
                .is_err()
            {
                self.clock_hint_misses.fetch_add(1, Ordering::Relaxed);
                *self.clock_failure.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some("clock preparation advisory allocation failed");
            }
            Ok(())
        }
    }

    fn grow_clock_capture(
        &self,
        capture: &mut ClockCapture,
        required: usize,
    ) -> std::io::Result<()> {
        let cadence = unsafe { &mut *self.cadence.get() };
        let pending = unsafe { &mut *self.pending.get() };
        let before = capture.allocated_bytes()
            + cadence.prev_campaign.allocated_bytes()
            + pending.allocated_bytes();
        let result = (|| {
            capture.entry.prepare(required)?;
            capture.previous.prepare(required)?;
            cadence.prev_campaign.prepare(required)?;
            pending.prepare(required)
        })();
        self.pool_bytes.fetch_add(
            (capture.allocated_bytes()
                + cadence.prev_campaign.allocated_bytes()
                + pending.allocated_bytes()
                - before) as u64,
            Ordering::Relaxed,
        );
        result?;
        for index in 0..BUFFERS {
            if !self.free.take(index, false) {
                continue;
            }
            self.frame_priority
                .claim(index, crate::frame_priority::Caller::Worker);
            let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
            let result = frame.prepare_clock(required);
            self.measure_frame(index);
            self.return_frame(index, false);
            result?;
        }
        capture.prepared = required;
        self.clock_available.notify_all();
        Ok(())
    }

    /// Park before any clock payload read if native load introduced longer text.
    /// The spool uses try_lock, so an ordinary callback waiting for a frame can
    /// never prevent that same spool from returning its queue lease.
    fn wait_clock_preparation(
        &self,
        memory: &impl engine_native::Memory,
    ) -> std::sync::MutexGuard<'_, ClockCapture> {
        let required = cgame_state::campaign_id_len(memory, self.base).unwrap_or(0);
        self.clock_required.fetch_max(required, Ordering::Release);
        let mut capture = self.clock_capture.lock().unwrap_or_else(|e| e.into_inner());
        while capture.prepared < required
            && !self.cancel_buffer_wait.load(Ordering::Acquire)
            && !crate::allocation::failed()
        {
            // The spool prepares the destination; end its idle wait.
            self.wake_spool();
            capture = self
                .clock_available
                .wait_timeout(capture, std::time::Duration::from_millis(5))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        capture
    }

    pub(crate) fn clock_status(&self) -> (u64, u64, Option<&'static str>) {
        (
            self.clock_misses.load(Ordering::Relaxed),
            self.clock_hint_misses.load(Ordering::Relaxed),
            *self.clock_failure.lock().unwrap_or_else(|e| e.into_inner()),
        )
    }

    pub(crate) fn clock_required(&self) -> usize {
        self.clock_required.load(Ordering::Acquire)
    }

    /// Loader only, before any Arc or hook can expose the roster.
    pub(crate) fn prepare_roster(&mut self, memory: &impl engine_native::Memory) {
        let roster = self
            .roster
            .get_mut()
            .as_mut()
            .expect("roster before publication");
        let before = roster.allocated_bytes();
        roster.prepare(memory, self.base);
        self.pool_bytes.fetch_add(
            (roster.allocated_bytes() - before) as u64,
            Ordering::Relaxed,
        );
    }

    /// Called only by the game thread inside the hook. A failed clock postpones
    /// this once-only read until the next readable hour, free buffer or not.
    unsafe fn sample_roster(
        &self,
        memory: &impl engine_native::Memory,
        clock: Option<Clock<&[u8]>>,
    ) {
        self.sample_content_roster(memory, clock.map(|clock| clock.hour));
    }

    pub(crate) fn sample_content_roster(&self, memory: &impl Memory, hour: Option<i32>) {
        if hour.is_none()
            || self.roster_ready.load(Ordering::Acquire)
            || self
                .roster_sampling
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        if !self.roster_ready.load(Ordering::Acquire) {
            let _reads = crate::read_accounting::table(replay_data::rows::TableId::Players);
            let roster = unsafe { (&mut *self.roster.get()).as_mut().unwrap() };
            roster.sample(memory, self.base, hour);
            self.roster_ready.store(roster.sampled(), Ordering::Release);
        }
        self.roster_sampling.store(false, Ordering::Release);
        self.wake_spool();
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
    crate::land_events::fail("land observer frame not captured");
    crate::air_losses::lost_drain();
    crate::building_bombing::lost_drain();
    crate::nuclear_strikes::lost_drain();
    crate::market_changes::lost_drain();
    crate::map_events::interval::lost_output();
}

/// Restore an owned frame's absorbed witnesses, record its lost hour and return
/// the buffer. Shared by every queue-full branch; no new loss reasons.
fn drop_owned_frame(shared: &Shared, frame: &mut Captured, index: usize) {
    frame.extended.discard_event_feed();
    frame.map.naval.discard_owned();
    frame.extended.discard_owned_censuses();
    // A lost owned settlement cannot certify next hour's held state.
    if let Some(store) = frame.extended.take_settled_army_store() {
        shared.return_army_stat_store(store);
    }
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
        std::ptr::swap(shared.pending.get(), &mut frame.dropped_before);
        (&mut *shared.pending.get()).push(frame.clock.as_ref().map(Clock::borrowed));
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

/// The hour's read, built at the tick's entry. `index` is None when no frame
/// could be obtained before shutdown cancellation: retain an explicit loss witness.
#[derive(Default)]
struct ClockCapture {
    entry: Owned,
    previous: Owned,
    initialized: bool,
    // Attachment-local identity: advance only after an exact full-byte change.
    epoch: u64,
    prepared: usize,
}
impl ClockCapture {
    fn allocated_bytes(&self) -> usize {
        self.entry.allocated_bytes() + self.previous.allocated_bytes()
    }
    fn read(
        &mut self,
        memory: &impl engine_native::Memory,
        base: usize,
    ) -> Result<(Clock<&[u8]>, u64), &'static str> {
        let clock = cgame_state::read_clock_prepared(memory, base, &mut self.entry)?;
        if !self.initialized || self.previous.as_bytes() != clock.campaign_id {
            let epoch = self
                .epoch
                .checked_add(1)
                .ok_or("clock campaign epoch overflow")?;
            self.previous.copy_prepared(clock.campaign_id)?;
            self.epoch = epoch;
            self.initialized = true;
        }
        Ok((clock, self.epoch))
    }
}

struct Dispatch<'a> {
    world_generation: u64,
    campaign_epoch: u64,
    index: Option<usize>,
    entry: Option<Clock<&'a [u8]>>,
    entry_ticks: i64,
    buffer_wait_ns: u64,
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
    entry: Option<Clock<&[u8]>>,
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
    entry: Option<Clock<&[u8]>>,
    acquire: impl FnOnce(),
    original: impl FnOnce(),
) -> bool {
    let _phase = crate::allocation::enter(crate::allocation::Phase::Callback);
    crate::allocation::set_context(crate::allocation::Context {
        campaign_id: entry.and_then(|clock| clock.campaign_id.try_into().ok()),
        branch: None,
        load_signal: entry.map(|clock| clock.play_session_started_at),
        failed_hour: entry.map(|clock| clock.hour),
    });
    let complete = unsafe { guarded_acquisition(shared, entry, acquire) };
    original();
    crate::allocation::attempted_hour(None);
    complete
}

/// The hooked hourly update boundary. Its entry is the
/// settled end-of-hour state (every command drained, tick_busy 0, before the
/// clock increment). The game thread runs the acquisition body here on its
/// own hot core, driving the reader pool and helping drain it; the trampoline
/// runs only after every live read has finished, so the recorded frame is that
/// settled end-of-hour state.
pub unsafe extern "system" fn tick(state: usize, commands: usize) {
    #[cfg(feature = "worker_sampler")]
    let sampler_callback = crate::worker_sampler::callback();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::Hourly);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let shared = unsafe { &*SHARED.load(Ordering::Acquire) };
    let admission = crate::runtime::admit_hourly_capture();
    let recording = admission.is_some()
        && shared.recording.load(Ordering::Acquire)
        && !crate::allocation::failed();
    if recording {
        let history_capture = shared.history_frames.capture();
        let _frame_priority = shared.frame_priority.callback();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Callback);
        let _reads = crate::read_accounting::Capture::new();
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let world_generation = crate::world_lifetime::generation();
        // Read the clock at entry: it is still the hour just ended, the trampoline
        // has not run. The frame's identity and the settled state the body reads.
        let clock_start = qpc();
        let mut clock_capture = shared.wait_clock_preparation(&memory);
        let (entry, campaign_epoch) = match clock_capture.read(&memory, shared.base) {
            Ok(value) => (Some(value.0), value.1),
            Err(reason) => {
                shared.clock_misses.fetch_add(1, Ordering::Relaxed);
                *shared
                    .clock_failure
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(reason);
                (None, 0)
            }
        };
        let entry = entry.filter(|_| {
            world_generation != u64::MAX && world_generation == crate::world_lifetime::generation()
        });
        crate::world_lifetime::boundary(world_generation, entry.is_some());
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: entry.and_then(|clock| clock.campaign_id.try_into().ok()),
            branch: None,
            load_signal: entry.map(|clock| clock.play_session_started_at),
            failed_hour: entry.map(|clock| clock.hour),
        });
        crate::equipment_destruction::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        crate::naval_result_sources::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        crate::experience_gains::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        crate::air_losses::boundary(entry.map(|clock| clock.with_campaign_id(campaign_epoch)));
        crate::fuel_awards::boundary();
        crate::building_bombing::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        crate::nuclear_strikes::boundary(entry.map(|clock| clock.with_campaign_id(campaign_epoch)));
        let entry_ticks = qpc() - clock_start;
        #[cfg(feature = "callback_profile")]
        let profile_before = callback_profile_counters(shared);
        #[cfg(feature = "callback_profile")]
        crate::callback_profile::reset_join();
        #[cfg(feature = "callback_profile")]
        let profile_stamp = crate::callback_profile::Stamp::start();
        let dispatched = qpc();
        shared
            .acquisition_entry_qpc
            .store(dispatched, Ordering::Relaxed);
        let mut buffer_wait_ns = 0;
        let index = shared.take_frame(&mut buffer_wait_ns);
        if let Some(index) = index {
            history_capture.frame(index);
            shared.completed[index].store(0, Ordering::Relaxed);
            shared.callback_tail[index].store(0, Ordering::Relaxed);
        }
        // The body reads on this hot core; pool workers read in parallel and
        // this thread helps drain them. A Rust panic must never unwind across
        // the extern boundary into the game: contain it and drop the hour.
        let job = Dispatch {
            world_generation,
            campaign_epoch,
            index,
            entry,
            entry_ticks,
            buffer_wait_ns,
            dispatched,
        };
        let priority = CallbackPriority::raise();
        if unsafe { guarded_acquisition(shared, entry, || acquire(shared, job)) }
            && let Some(index) = index
        {
            shared.callback_tail[index].store(1, Ordering::Release);
        }
        drop(priority);
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
            sample.diagnostics = crate::callback_profile::take_diagnostics();
            crate::callback_profile::record(sample);
        }
    }
    drop(admission);
    #[cfg(feature = "worker_sampler")]
    drop(sampler_callback);
    // Run the trampoline (this hour's tick) only after every live read finished.
    let original_begin = qpc();
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(shared.trampoline) };
    let ticking = TICKING.replace(true);
    crate::tick_gate::enter(shared.tick_idle.as_ref());
    timing.original(|| unsafe { original(state, commands) });
    crate::tick_gate::leave(shared.tick_idle.as_ref());
    TICKING.set(ticking);
    let original_end = qpc();
    if recording {
        shared
            .native_tick_ns
            .fetch_add(shared.ns(original_end - original_begin), Ordering::Relaxed);
        shared.native_tick_calls.fetch_add(1, Ordering::Relaxed);
    }
    drop(timing);
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

/// An observer boundary job on its way.
struct PendingBoundary {
    pending: PendingAcquisition,
}

/// # Safety
/// The caller holds the original native boundary through the join.
unsafe fn submit_observer_boundary(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    request: extended::observer_boundary::ObserverBoundaryRequest,
) -> PendingBoundary {
    PendingBoundary {
        // SAFETY: forwarded from the caller.
        pending: unsafe {
            submit_acquisition(shared, batch, acquisition::Job::ObserverBoundary(request))
        },
    }
}

/// # Safety
/// As [`submit_observer_boundary`].
unsafe fn finish_observer_boundary(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    frame: &mut Captured,
    PendingBoundary { pending }: PendingBoundary,
) {
    // SAFETY: as above.
    let (request, report) = unsafe {
        collect_acquisition(
            shared,
            batch,
            pending,
            |job| {
                let acquisition::Job::ObserverBoundary(request) = job else {
                    unreachable!("observer ticket retains its submitted type")
                };
                request
            },
            |report| {
                let acquisition::Report::ObserverBoundary(report) = report else {
                    unreachable!("observer report retains its submitted type")
                };
                report
            },
        )
    };
    let failure = report
        .as_ref()
        .err()
        .copied()
        .map(acquisition_failure_reason);
    frame
        .extended
        .restore_observer_boundary_request(request, report.unwrap_or_default(), failure);
}

/// Pool workers run above normal. While the game thread waits on them it must
/// take a core back as soon as it is readied, not queue behind a worker's
/// slice. Raised for the acquisition only; the game's own priority returns
/// before any native code runs, and nothing is raised if it is already higher.
struct CallbackPriority(Option<i32>);
impl CallbackPriority {
    fn raise() -> Self {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
        };
        // GetThreadPriority's failure value.
        const UNKNOWN: i32 = i32::MAX;
        // SAFETY: the current-thread pseudo-handle needs no close.
        let previous = unsafe { GetThreadPriority(GetCurrentThread()) };
        let raised = previous != UNKNOWN
            && previous < THREAD_PRIORITY_HIGHEST
            // SAFETY: as above.
            && unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } != 0;
        Self(raised.then_some(previous))
    }
}
impl Drop for CallbackPriority {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
        if let Some(previous) = self.0 {
            // SAFETY: the current-thread pseudo-handle needs no close.
            unsafe { SetThreadPriority(GetCurrentThread(), previous) };
        }
    }
}

/// Every live read of the hour that just ended, on the game thread inside the
/// hook. The trampoline has not run, so the state is the settled end-of-hour
/// boundary, read exactly as the original collector did.
unsafe fn acquire(shared: &Shared, dispatch: Dispatch<'_>) {
    let memory = CountedMemory::new(unsafe { LiveMemory::new() });
    unsafe { acquire_with_memory(shared, dispatch, memory) };
}

unsafe fn acquire_with_memory(
    shared: &Shared,
    dispatch: Dispatch<'_>,
    memory: CountedMemory<impl Memory>,
) {
    let _callback_parks = crate::acquisition::callback_parks::Scope::enter();
    let Dispatch {
        world_generation,
        campaign_epoch,
        index,
        entry,
        entry_ticks,
        buffer_wait_ns,
        dispatched,
    } = dispatch;
    reset_hour_joins();
    let growths_before = engine_allocation::growths();

    if let Some(index) = index {
        let reset_begin = qpc();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // The spool reset this frame's verdicts when it returned it; a frame
        // returned another way is reset here.
        if !std::mem::take(&mut frame.verdicts_reset) {
            frame.reset_verdicts();
        }
        // The shared CountryIntel buffer goes home at this callback, as when
        // the callback reset every frame.
        frame.extended.release_country_intel();
        frame.world_generation = world_generation;
        frame.set_clock(entry);
        frame.qpc = dispatched;
        let before = qpc();
        // Passive scalar reads of the same counters the save header emits.
        frame.counters = frame.clock.as_ref().map(|clock| {
            save_allocation_counters::read_counters(&memory, shared.base, clock.game_state as usize)
        });
        let clock_reads = qpc() - before;
        let clock_ticks = entry_ticks + clock_reads;
        // The hour's decode job is filled in place in its queue slot, so its
        // payloads never sit in this frame. Owning a buffer guarantees a slot.
        let mut decode_slot = shared
            .decode_queue
            .reserve()
            .expect("an owned buffer has a decode slot");
        shared.game_thread.frame_reset_ns.fetch_add(
            shared.ns(qpc() - reset_begin - clock_reads),
            Ordering::Relaxed,
        );
        let decode::DecodeJob {
            queued_qpc: _,
            relations_payload,
            characters_payload,
            projects_payload,
            lexical_payload,
            divisions_payload,
            air_wing_payload,
            air_stats_payload,
            naval_payload,
            players_payload,
            session_payload,
            production_rest_payload,
            construction_rest_payload,
            economy_rest_payload,
            air_bases_payload,
            supremacy_payload,
            sunk_payload,
            radar_payload,
            intel_payload,
            weather_payload,
            capitulation_payload,
            ..
        } = decode_slot.job();
        // Relations handoff state: same FIFO, same ordering guarantees.
        let mut relations_report = extended::intel::RelationsCopyReport::default();
        // Characters handoff state: same FIFO, same ordering guarantees.
        let mut characters_report =
            [extended::characters::CharactersCopyReport::default(); CHARACTER_PARTS];
        // Projects handoff state: same FIFO, same ordering guarantees.
        let mut projects_report = extended::projects::ProjectsCopyReport::default();
        // Lexical handoff state: same FIFO, same ordering guarantees.
        let mut lexical_report = extended::lexical::LexicalCopyReport::default();
        // Divisions handoff state: same FIFO, same ordering guarantees.
        // The hourly payload carries the divisions walk and any scheduled
        // equipment/upgrade detail walks sharing its census.
        let mut divisions_report = DivisionsCopyReport::default();
        let mut air_wing_report = extended::air::AirWingCopyReport::default();
        let mut air_stats_report = extended::air::AirStatsCopyReport::default();
        let mut naval_report = extended::naval_history::NavalCopyReport::default();
        // Players handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the independent roster
        // and controller walks with its own root refs and no
        // cross-family decoded roots. Staging overflow falls back to the
        // typed walks with no payload enqueued.
        let mut players_report = crate::map_capture::PlayersCopyReport::default();
        // Session family handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries the manager, sources
        // and aggregates with its own root refs and no cross-family
        // decoded roots. Staging overflow falls back to the typed walks
        // with no payload enqueued.
        let mut session_report = extended::session::SessionCopyReport::default();
        // Production rest handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries lines, stockpiles,
        // licences, the deployment queue and licence records with its own
        // census and no cross-family decoded roots. Staging overflow
        // falls back to the typed walks with no payload enqueued.
        let mut production_rest_report = extended::production::ProductionRestCopyReport::default();
        // Construction rest handoff state: same FIFO, same ordering
        // guarantees. The scheduled payload carries resources, the
        // construction queue, occupation, resistance and collaboration
        // with its own census and no cross-family decoded roots. Staging
        // overflow falls back to the typed walks with no payload enqueued.
        let mut construction_rest_report =
            extended::construction::ConstructionRestCopyReport::default();
        // Economy hourly rest handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries province supply plus scheduled
        // fuel, experience, hubs, disrupted, foreign, definitions, resources,
        // exchanges and purchases with its own census and no
        // cross-family decoded roots. Staging overflow falls back to the
        // typed walks with no payload enqueued.
        let mut economy_rest_report = extended::economy::EconomyRestCopyReport::default();
        // Shared hourly bases handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the wing/manager census
        // once with bases, registrations and scheduled aces detail; the
        // decodes read the shared spans. Staging overflow falls back to
        // the typed walks with no payload enqueued.
        let mut air_bases_report = extended::air::AirBasesCopyReport::default();
        // Scheduled supremacy handoff state: same FIFO, same ordering
        // guarantees. Staging overflow falls back to the typed walk with
        // no payload enqueued.
        let mut supremacy_report = extended::naval_history::SupremacyCopyReport::default();
        // Scheduled sunk-ships handoff state: same FIFO, same ordering
        // guarantees. Staging overflow falls back to the typed walk with
        // no payload enqueued.
        let mut sunk_report = extended::naval_history::SunkCopyReport::default();
        // Scheduled radar handoff state: same FIFO, same ordering
        // guarantees. Staging overflow falls back to the typed walk with
        // no payload enqueued.
        let mut radar_report = extended::fog::RadarCopyReport::default();
        // Hourly intel handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the walk's own
        // division/fleet census once with viewer detail; both unit paths
        // decode from the shared spans. Staging overflow falls back to
        // the typed walk with no payload enqueued.
        let mut intel_report = extended::fog::IntelCopyReport::default();
        // Hourly weather handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the manager header once
        // with limits, regions and provinces detail; all three decodes
        // read the shared spans. Staging overflow falls back to the typed
        // walks with no payload enqueued.
        let mut weather_report = extended::weather::WeatherCopyReport::default();
        // Hourly capitulation handoff state: same FIFO, same ordering
        // guarantees. The hourly payload carries the country census once
        // with diplomacy detail; the decode reads the shared spans.
        // Staging overflow falls back to the typed walk with no payload
        // enqueued.
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
        let mut segments_ns = [0; SEGMENTS];
        let mut first_since_attach = false;
        let mut continuous = false;
        let mut force_full = false;
        let mut recovery = false;
        let mut refresh = false;
        let mut attach_diagnostics = None;
        if let Some(clock) = entry {
            attach_diagnostics = Some(crate::attach_diagnostics::begin(
                clock.hour,
                dispatched,
                shared.frequency,
            ));
            // Out of line: the layout report (about 10 KiB) stays off this frame.
            out_of_line(|| {
                frame.fixed_layout = Some(match shared.fixed_layout.get() {
                    Some(report) => Ok(*report),
                    None => {
                        let buffer = unsafe { &mut *shared.fixed_layout_buffer.get() };
                        engine_fixed_layout::fixed_layout::check(&memory, shared.base, buffer)
                            .inspect(|report| _ = shared.fixed_layout.set(*report))
                    }
                });
            });
            let cadence = unsafe { &mut *shared.cadence.get() };
            first_since_attach = !cadence.initialized;
            continuous = cadence.continuous(&clock);
            recovery = shared.force_next.swap(false, Ordering::AcqRel);
            refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
            forget_unpublished_families(shared);
            force_full = cadence.force_full(&clock) | recovery | refresh;
            // A periodic encoding keyframe does not interrupt native mutation
            // observation. Its freshly certified owned rows remain complete.
            if !continuous || recovery {
                crate::portrait_changes::invalidate();
                crate::advisor_changes::invalidate();
                crate::opinion_changes::invalidate();
                crate::operation_outcomes::invalidate();
                crate::politics_changes::invalidate();
                crate::map_events::interval::invalidate();
                extended::intel::forget_relation_opinions();
            }
            // Constant time: two counters and each family's fault bits.
            frame.extended.seal_map_events(clock.hour);
            let acquisition = shared.acquisition.get().expect("initialized before hook");
            let mut batch = acquisition::Batch::new(acquisition);
            let final_inputs_dispatch = batch.final_inputs();
            let mut boundary_request = frame.extended.take_observer_boundary_request(
                shared.base,
                clock.hour,
                continuous,
                recovery,
            );
            boundary_request.market_clock = Some(clock.with_campaign_id(campaign_epoch));
            boundary_request.market_refresh = force_full;
            let boundary_begin = qpc();
            // SAFETY: the native hourly call stays parked through the boundary job.
            let boundary_pending =
                unsafe { submit_observer_boundary(shared, &mut batch, boundary_request) };
            let mut boundary_ticks = qpc() - boundary_begin;
            let state = clock.game_state as usize;
            // The country snapshot reads only native memory and the shared
            // cache: it runs while a worker drains the observers.
            let cache_begin = qpc();
            shared.install_offered_country_cache();
            let country_snapshot =
                crate::read_accounting::with_table(replay_data::rows::TableId::Countries, || {
                    Cache::prepare_shared(
                        unsafe { &mut *shared.country_cache.get() },
                        &memory,
                        shared.base,
                        state,
                    )
                })
                .expect("every country snapshot lease returned before the next callback");
            let cached = country_snapshot.view(&memory);
            let snapshot_ticks = qpc() - cache_begin;
            frame
                .extended
                .set_load_signal(clock.play_session_started_at);
            frame.extended.set_capture_mode(shared.capture_mode);
            frame.extended.prepare_hour(clock.hour, force_full);
            if let Some(store) = &shared.army_stat_store {
                store.prepare_hour(clock.hour, force_full);
            }
            shared
                .game_thread
                .country_snapshot_ns
                .fetch_add(shared.ns(snapshot_ticks), Ordering::Relaxed);
            frame.country_cache_status = cached.seed_status();
            frame.country_cache_entries = cached.cached_entries();
            let cache_prepare_ticks = qpc() - cache_begin;
            let characters_refresh = decode::characters_refresh_hour(clock.hour, force_full);
            let projects_refresh = decode::projects_refresh_hour(clock.hour, force_full);
            let lexical_max =
                engine_sites::at(shared.base, engine_native::cpdx_lexer::MAX_TOKEN_ID_RVA)
                    .and_then(|at| engine_native::read::<i32>(&memory, at))
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
                replay_data::cadence::should_walk(
                    replay_data::rows::TableId::DivisionEquipment,
                    clock.hour,
                    force_full,
                ),
            ));
            let relations_refresh = decode::relations_refresh_hour(clock.hour, force_full);
            // The observer join, as late as the first dispatch allows. Nothing
            // since the submission reads what the boundary job writes: the
            // lanes it took from the frame, the observer registries'
            // boundaries, its drained and sealed lanes and the offered output
            // stores. Any job below may, and a held job can still run, so it
            // joins before the first `queue!`.
            let boundary_begin = qpc();
            // SAFETY: as above.
            unsafe { finish_observer_boundary(shared, &mut batch, frame, boundary_pending) };
            boundary_ticks += qpc() - boundary_begin;
            shared
                .game_thread
                .boundaries_ns
                .fetch_add(shared.ns(boundary_ticks), Ordering::Relaxed);
            // Every accepted request is owned by this callback-scoped draining guard.
            // Rejected submissions retain their buffers. Only fixed-storage
            // readers may execute inline; allocating readers require a worker.
            ts_setup = qpc();
            let mut segment_prev = ts_setup;
            macro_rules! segment {
                ($index:expr) => {{
                    let now = qpc();
                    let ns = shared.ns(now - segment_prev);
                    shared.phase_segments[$index].fetch_add(ns, Ordering::Relaxed);
                    segments_ns[$index] += ns;
                    segment_prev = now;
                }};
            }
            let wait_at_setup = shared.acquisition_wait_ns.load(Ordering::Relaxed);
            // Requests are built, submitted and collected out of line, so this
            // frame holds tickets and variant results, never a job or completion.
            macro_rules! queue {
                ($job:expr) => {{
                    let begin = qpc();
                    let job = || $job;
                    // SAFETY: batch drains all accepted work before this callback returns.
                    let pending = unsafe { queue_acquisition(shared, &mut batch, job) };
                    shared
                        .presentation
                        .queue_ns
                        .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
                    pending
                }};
            }
            // One collector wake; workers relay the held burst's peer wakes.
            macro_rules! release {
                () => {{
                    let begin = qpc();
                    let jobs = batch.release();
                    shared.game_thread.release(jobs, shared.ns(qpc() - begin));
                }};
            }
            let mut presentation = PresentationPending::default();
            macro_rules! collected {
                ($pending:expr, $variant:ident) => {{
                    let pending = $pending;
                    // Keep presentation edges live during later assembly-only joins too.
                    // SAFETY: this thread still owns the live acquisition boundary.
                    unsafe {
                        presentation.wait_for(shared, &mut batch, Some(&pending));
                        collect_acquisition(
                            shared,
                            &mut batch,
                            pending,
                            |job| {
                                let acquisition::Job::$variant(request) = job else {
                                    unreachable!(
                                        "acquisition ticket retains its submitted job type"
                                    )
                                };
                                request
                            },
                            |report| {
                                let acquisition::Report::$variant(report) = report else {
                                    unreachable!(
                                        "acquisition report retains its submitted job type"
                                    )
                                };
                                report
                            },
                        )
                    }
                }};
            }
            macro_rules! completed {
                ($pending:expr, $variant:ident) => {{
                    let (request, report) = collected!($pending, $variant);
                    let failure = report
                        .as_ref()
                        .err()
                        .copied()
                        .map(acquisition_failure_reason);
                    (request, report.unwrap_or_default(), failure)
                }};
            }
            // The first burst: every job that needs no other job's output,
            // except the slack burst that leaves behind the Naval copy. It
            // queues without waking the pool and is released at once
            // (`Batch::release`), the jobs that gate others first.
            batch.hold();
            // The census publisher releases details without waiting for this callback.
            let ship_census = std::mem::replace(&mut frame.map.naval, ShipCensus::vacant());
            let ship_models_pending = out_of_line(|| {
                let mut request = frame.extended.take_ship_models_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    ship_census,
                );
                batch.arm_ship_details(&mut request);
                queue!(acquisition::Job::ShipModels(request))
            });
            // The Ordinary counter part reads the naming this job leaves in
            // the appearance pack; it leaves as soon as this job joins.
            let countries_pending = queue!(acquisition::Job::Countries(
                frame.extended.take_countries_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    &mut frame.countries,
                    &mut frame.links,
                    (clock.hour, force_full),
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { (*shared.appearance_store.get()).take() },
                )
            ));
            // The template dictionary reads only the shared country cache: it
            // walks beside Countries and joins before the tables.
            let templates_pending = queue!(acquisition::Job::Templates(
                frame.extended.take_templates_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    (clock.hour, force_full),
                )
            ));
            // Original inputs grow on a worker before any consumer receives a
            // lease. Rejection/failure is terminal for this attempt, without a
            // callback fallback or a second native roster walk. Presentation,
            // Divisions and Intel wait for this join.
            let (division_source_capacity, detail_capacities) =
                frame.extended.division_capture_capacities();
            let divisions_ctx = DivisionsCopyContext {
                base: shared.base,
                game_state: state,
                row_capacity: frame.divisions.len(),
                source_capacity: division_source_capacity,
                detail_capacities,
                details_active: replay_data::cadence::should_walk(
                    TableId::DivisionEquipment,
                    clock.hour,
                    force_full,
                ),
            };
            // The country slots split into parts by the previous hour's
            // census; the inputs job walks the first.
            // SAFETY: only this callback reads and writes the split.
            let (starts, split) = unsafe { *shared.division_part_starts.get() };
            let slots = |part: usize| {
                let first = if part == 0 { 1 } else { starts[part - 1] };
                (
                    first,
                    starts
                        .get(part)
                        .filter(|_| part < split)
                        .copied()
                        .unwrap_or(u32::MAX),
                )
            };
            // Without the status side the presentation reads only the movement pass.
            let status_recorded = frame
                .extended
                .records_any(&extended::division_presentation::STATUS_TABLES);
            let intel_recorded = frame.extended.records_any(&decode::INTEL);
            let mut input_payload = decode::acquire_divisions_job();
            input_payload.slots = (split != 0).then(|| slots(0));
            // SAFETY: only this callback reads and writes the split.
            input_payload.join_capacity = unsafe { *shared.division_join_capacity.get() };
            input_payload.presentation = None;
            let presentation_inputs = out_of_line(|| {
                let mut request = frame.extended.take_presentation_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    None,
                );
                request.inputs = None;
                request.captured.status_workers = acquisition.worker_count();
                if !status_recorded {
                    return batch.arm_presentation_inputs(request, split, None, true);
                }
                batch.arm_presentation(&mut request);
                batch.arm_presentation_inputs(request, split, shared.army_stat_store.clone(), false)
            });
            input_payload.presentation = Some(Arc::clone(&presentation_inputs));
            let input_pending = queue!(acquisition::Job::DivisionInputs(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: divisions_ctx,
                payload: input_payload,
            }));
            let mut part_pending: [Option<PendingAcquisition>;
                crate::division_census::MAX_PARTS - 1] =
                [const { None }; crate::division_census::MAX_PARTS - 1];
            for (part, pending) in part_pending.iter_mut().enumerate().take(split) {
                // SAFETY: only this callback takes and returns the censuses.
                if let Some(census) = unsafe { (*shared.division_parts.get())[part].take() } {
                    *pending = Some(queue!(acquisition::Job::DivisionInputParts(
                        acquisition::DivisionPartRequest {
                            countries: Arc::clone(&country_snapshot),
                            base: shared.base,
                            game_state: state,
                            slots: slots(part + 1),
                            census,
                            presentation: Some((Arc::clone(&presentation_inputs), part)),
                        }
                    )));
                }
            }
            // Naval uses this callback's independently bounded wing-membership census.
            // Both consumers borrow the same original registry census. Its
            // worker prepares their full storage before either job is queued.
            // The wing job bounds its naval memberships by the naval payload's
            // membership capacity, which the root census does not change
            // (`NavalJob::capture_roots_growing` reserves only its roots): it
            // leaves in this burst with the capacity taken here.
            let naval = decode::acquire_naval_job();
            let naval_limit = naval.wing_limit();
            // SAFETY: the hourly callback is the split's only reader and writer.
            unsafe { begin_order_parts(shared, &mut frame.map.orders) };
            let root_pending = queue!(acquisition::Job::OrderRoots(
                acquisition::OrderRootsRequest {
                    base: shared.base,
                    orders: std::mem::replace(
                        &mut frame.map.orders,
                        crate::map_capture::Orders::vacant()
                    ),
                    naval,
                }
            ));
            // The wing census reads only the country snapshot; AirMissions
            // waits for its join, and the Naval copy takes its memberships
            // when it returns.
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
                naval_limit,
                game_score: shared.game_score_enabled,
            };
            // Split by owner slots, sized by the last joined census: every
            // part after the first leaves now too, and the join below puts
            // them together in slot order.
            // SAFETY: only this callback reads and writes the split.
            let (air_starts, air_split) = unsafe { *shared.air_wing_part_starts.get() };
            let air_slots = |part: usize| {
                let first = if part == 0 { 0 } else { air_starts[part - 1] };
                let end = air_starts
                    .get(part)
                    .filter(|_| part < air_split)
                    .copied()
                    .unwrap_or(u32::MAX);
                first..end
            };
            let mut air_payload = decode::acquire_air_wing_job();
            // SAFETY: only this callback takes and returns the store.
            air_payload.wing_store = unsafe { (*shared.wing_store.get()).take() };
            air_payload.slots = air_slots(0);
            air_payload.split = air_split != 0;
            let air_pending = queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                countries: Arc::clone(&country_snapshot),
                context: air_ctx,
                payload: air_payload,
            }));
            let mut air_part_pending: [Option<PendingAcquisition>; extended::air::WING_PARTS - 1] =
                [const { None }; extended::air::WING_PARTS - 1];
            for (part, pending) in air_part_pending.iter_mut().enumerate().take(air_split) {
                // SAFETY: only this callback takes and returns the parts.
                let mut payload =
                    std::mem::take(unsafe { &mut (*shared.air_wing_parts.get())[part] });
                payload.slots = air_slots(part + 1);
                payload.split = true;
                *pending = Some(queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    // A part grows its own storage; the frame's grants size
                    // the joined census.
                    context: extended::air::AirWingCopyContext {
                        limits: [0; 3],
                        naval_limit: 0,
                        ..air_ctx
                    },
                    payload,
                })));
            }
            // The character roster supplies the portrait shares, the role jobs
            // and the Gated job's role census.
            let (trait_rows, trait_bytes) =
                frame.extended.characters_pilot().trait_capture_capacities();
            let characters_ctx = extended::characters::CharactersCopyContext {
                base: shared.base,
                game_state: state,
                force_full,
                trait_rows,
                trait_bytes,
                active: decode::CHARACTERS
                    .map(|table| replay_data::cadence::should_walk(table, clock.hour, force_full)),
            };
            frame
                .extended
                .characters_pilot()
                .set_character_activity(characters_ctx.active);
            let mut characters_pending: [Option<PendingAcquisition>; CHARACTER_PARTS] =
                [const { None }; CHARACTER_PARTS];
            let character_part = |part: usize| -> [bool; extended::characters::N] {
                std::array::from_fn(|index| {
                    characters_ctx.active[index] && extended::characters::PART_MASKS[part][index]
                })
            };
            // The roster job gathers the role census only when a role job or
            // the character inputs run.
            let role_hour = (1..CHARACTER_PARTS)
                .any(|part| character_part(part).iter().any(|&value| value))
                || extended::character_inputs::TABLES
                    .iter()
                    .any(|&table| replay_data::cadence::should_walk(table, clock.hour, force_full));
            let character_part_recorded = |part: usize| {
                decode::CHARACTERS
                    .iter()
                    .zip(extended::characters::PART_MASKS[part])
                    .any(|(&table, member)| {
                        member && replay_data::scope::records(shared.capture_mode, table)
                    })
            };
            let roster_active = character_part(0);
            if characters_refresh && roster_active.iter().any(|&value| value) {
                characters_pending[0] = Some(queue!(acquisition::Job::Characters(
                    acquisition::CharactersRequest {
                        final_inputs: None,
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
            // The production and construction rests, and beside them the
            // economy rest, each its own job joined at the same point.
            // Out of line: the job-sized request stays off the hourly body's
            // frame. The construction rest's verdict, installed in the Gated
            // packs before they leave, is whether its part was scheduled:
            // `settle_world` set it to exactly this.
            // A World its capture mode leaves out stands as a failed one: its
            // scheduled parts keep their raw verdicts, so neither Gated nor
            // FinalInputs walks them, and refuse after the assembly.
            let world_scheduled = world_schedule(clock.hour, force_full);
            let world_pending = (frame.extended.records_any(&decode::PRODUCTION_REST)
                || frame.extended.records_any(&decode::CONSTRUCTION_REST))
            .then(|| {
                out_of_line(|| {
                    queue!(acquisition::Job::World(acquire_world_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                    )))
                })
            });
            let construction_active = world_scheduled.1;
            let economy_pending = queue!(acquisition::Job::Economy(acquire_economy_request(
                &country_snapshot,
                shared.base,
                state,
                frame.extended.economy_rest_pilot(),
                clock.hour,
                force_full,
            )));
            // The research tables are loaned before the family that owns them.
            let research_pending = frame
                .extended
                .records_any(
                    &extended::governance::RESEARCH
                        .map(|index| extended::governance::TABLES[index]),
                )
                .then(|| {
                    // SAFETY: only this callback takes and returns the store.
                    let fingerprints =
                        unsafe { &mut *shared.slice_fingerprints[crate::slicing::RESEARCH].get() }
                            .take()
                            .expect("research fingerprints returned by the previous callback");
                    queue!(acquisition::Job::Research(
                        frame.extended.take_research_request(
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                            force_full,
                            fingerprints,
                        )
                    ))
                });
            let governance_pending =
                queue!(acquisition::Job::Governance(frame.extended.take_governance_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { take_slice_store(shared, crate::slicing::TECHNOLOGIES) },
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { take_slice_store(shared, crate::slicing::FOCUS) },
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { (*shared.building_store.get()).take() },
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { (*shared.leader_store.get()).take() },
                        // SAFETY: only this callback takes and returns the store.
                        unsafe { (*shared.party_store.get()).take() },
                    )));
            // The hourly change drain precedes the worker's roster decision, not
            // the first burst. Recovery can require a read off the daily slot.
            let variants_pending = {
                let request = frame.extended.take_variants_request(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                );
                queue!(acquisition::Job::Variants(request))
            };
            // The Ordinary family's independent parts: combat and naval
            // details, map counters, task forces, and the physical navy pass
            // with the naval mission ledgers. The naval mission store travels
            // with the navy part. The task-force part is taken here with the
            // others and queued in the slack burst behind the Naval copy.
            // Out of line: the parts' requests stay off the hourly body's frame.
            let (mut ordinary_parts_pending, mut ordinary_slack_parts) =
                out_of_line(|| {
                    let parts = frame.extended.take_ordinary_parts(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        crate::fixed_layout::task_force_eta_guard(frame.fixed_layout.as_ref()),
                        crate::extended::NavalMissionLoan {
                            force_full,
                            // SAFETY: only this callback takes and returns this store.
                            fingerprints: unsafe {
                                (*shared.slice_fingerprints[crate::slicing::NAVAL_MISSIONS].get())
                                    .take()
                            },
                        },
                    );
                    let mut slack: [Option<acquisition::OrdinaryPartRequest>;
                        extended::ORDINARY_PARTS] = [const { None }; extended::ORDINARY_PARTS];
                    let mut index = 0;
                    let pending = parts.map(|part| {
                        let slot = index;
                        index += 1;
                        if matches!(part.part, extended::OrdinaryPart::TaskForces(_)) {
                            slack[slot] = Some(part);
                            None
                        } else {
                            Some(queue!(acquisition::Job::OrdinaryPart(part)))
                        }
                    });
                    (pending, slack)
                });
            // SAFETY: only this callback takes and returns the stores.
            let slow_stores = unsafe {
                crate::extended::SlowStores {
                    available: (*shared.slice_fingerprints[crate::slicing::AVAILABLE_ADVISORS]
                        .get())
                    .take(),
                    advisors: (*shared.advisor_store.get()).take(),
                    local_subunits: (*shared.local_subunit_store.get()).take(),
                    doctrine_bytes: 0,
                    doctrines: (*shared.doctrine_store.get()).take(),
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
            // The daily production slot's walks, beside Slow, at that slot.
            // MIO is the longest independent walk, so submit it before the
            // modifier ranges. All still join at the late assembly barrier.
            let production_daily_due = extended::ProductionDailyPacks::due(clock.hour, force_full)
                && extended::ProductionDailyPacks::recorded(shared.capture_mode);
            let industrial_organisations_pending = production_daily_due.then(|| {
                queue!(acquisition::Job::IndustrialOrganisations(
                    frame.extended.take_industrial_organisations_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                    )
                ))
            });
            // On its slot the production modifier capture splits at the last
            // slot hour's balance. Loan the extra ranges before range zero's
            // pack leaves; all modifier ranges join in slot order.
            let mut production_part_pending: [Option<PendingAcquisition>;
                extended::production_modifiers::MAX_PARTS - 1] =
                [const { None }; extended::production_modifiers::MAX_PARTS - 1];
            if production_daily_due {
                // SAFETY: only this callback reads and writes the split.
                let (starts, split) = unsafe { *shared.production_part_starts.get() };
                let parts = frame.extended.prepare_production_modifier_parts(
                    &memory,
                    shared.base,
                    &starts[..split],
                );
                for (part, pending) in production_part_pending
                    .iter_mut()
                    .enumerate()
                    .take(parts - 1)
                {
                    *pending = Some(queue!(acquisition::Job::ProductionModifierPart(
                        frame.extended.take_production_modifier_part(
                            part + 1,
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                            force_full,
                        )
                    )));
                }
            }
            let production_daily_pending = production_daily_due.then(|| {
                queue!(acquisition::Job::ProductionDaily(
                    frame.extended.take_production_daily_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                    )
                ))
            });
            // The war overview, beside Slow.
            let war_overview_pending = frame
                .extended
                .records_any(&extended::war_overview::TABLES)
                .then(|| {
                    queue!(acquisition::Job::WarOverview(
                        frame.extended.take_war_overview_request(
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                            force_full,
                        )
                    ))
                });
            // The stability tables, beside Slow. Their store travels with the
            // job; missing it drops only that family's early reads this hour.
            let stability_pending = frame
                .extended
                .records_any(&extended::country_stability::TABLES)
                .then(|| {
                    // SAFETY: only this callback takes and returns the store.
                    let stability_store = unsafe {
                        (*shared.slice_fingerprints[crate::slicing::STABILITY].get()).take()
                    };
                    queue!(acquisition::Job::Stability(
                        frame.extended.take_stability_request(
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                            force_full,
                            stability_store,
                        )
                    ))
                });
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
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { (*shared.script_flag_store.get()).take() },
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { (*shared.script_variable_store.get()).take() },
                )
            ));
            let air_bases_ctx = extended::air::AirBasesCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let air_bases_pending = {
                let mut payload = decode::acquire_air_bases_job();
                // SAFETY: only this callback takes and returns the store.
                payload.registration_store = unsafe { (*shared.air_base_store.get()).take() };
                queue!(acquisition::Job::AirBases(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: air_bases_ctx,
                    payload,
                }))
            };
            // Without their stores the Factions walk visits no goal or companion.
            let goals_recorded = frame.extended.records_any(&crate::faction_goals::TABLES);
            let companions_recorded = frame
                .extended
                .records_any(&crate::faction_companions::TABLES);
            if !goals_recorded {
                frame.faction_goals.failure = Some(extended::NOT_RECORDED);
            }
            if !companions_recorded {
                frame.faction_companions.refuse(extended::NOT_RECORDED);
            }
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
                    // SAFETY: the callback alone loans and restores this store.
                    goal_store: goals_recorded
                        .then(|| unsafe { (*shared.faction_goal_store.get()).take() })
                        .flatten(),
                    goals: std::mem::take(&mut frame.faction_goals),
                    // SAFETY: the callback alone loans and restores this store.
                    companion_store: companions_recorded
                        .then(|| unsafe { (*shared.faction_companion_store.get()).take() })
                        .flatten(),
                    companions: std::mem::take(&mut frame.faction_companions),
                    force_full,
                }));
            let players_ctx = crate::map_capture::PlayersCopyContext {
                base: shared.base,
                game_state: state,
            };
            let players_pending = {
                queue!(acquisition::Job::Players(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: players_ctx,
                    payload: decode::acquire_players_job(),
                }))
            };
            let air_stats_ctx = extended::air::AirStatsCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            // A raw-copy job its capture mode leaves out stands as a failed one: its
            // raw verdict keeps the typed fallback from reading and its failure
            // refuses the tables after the joins.
            let air_stats_due =
                replay_data::cadence::should_walk(TableId::AirStatistics, clock.hour, force_full);
            let air_stats_omitted =
                air_stats_due && !frame.extended.records_any(&decode::AIR_STATS);
            let air_stats_pending = if air_stats_due && !air_stats_omitted {
                Some(queue!(acquisition::Job::AirStats(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: air_stats_ctx,
                        payload: decode::acquire_air_stats_job(),
                    }
                )))
            } else {
                frame.extended.air_stats_pilot().stats_raw_mode = air_stats_omitted;
                None
            };
            let supremacy_ctx = extended::naval_history::SupremacyCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let supremacy_pending = if replay_data::cadence::should_walk(
                TableId::NavalSupremacy,
                clock.hour,
                force_full,
            ) {
                Some(queue!(acquisition::Job::Supremacy(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: supremacy_ctx,
                        payload: decode::acquire_supremacy_job(),
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
            let sunk_due =
                replay_data::cadence::should_walk(TableId::SunkShips, clock.hour, force_full);
            let sunk_omitted = sunk_due && !frame.extended.records_any(&decode::SUNK);
            let sunk_pending = if sunk_due && !sunk_omitted {
                Some(queue!(acquisition::Job::Sunk(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: sunk_ctx,
                    payload: decode::acquire_sunk_job(),
                })))
            } else {
                frame.extended.sunk_pilot().sunk_raw_mode = sunk_omitted;
                None
            };
            let radar_ctx = frame.extended.radar_pilot().radar_copy_context(
                shared.base,
                state,
                clock.hour,
                force_full,
            );
            let radar_pending = if replay_data::cadence::should_walk(
                TableId::RadarCoverage,
                clock.hour,
                force_full,
            ) {
                Some(queue!(acquisition::Job::Radar(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: radar_ctx,
                    payload: decode::acquire_radar_job(),
                })))
            } else {
                frame.extended.radar_pilot().radar_raw_mode = false;
                None
            };
            let projects_ctx = extended::projects::ProjectsCopyContext {
                base: shared.base,
                game_state: state,
                active: projects_refresh,
            };
            let projects_omitted =
                projects_refresh && !frame.extended.records_any(&decode::PROJECTS);
            let projects_pending = if projects_refresh && !projects_omitted {
                Some(queue!(acquisition::Job::Projects(
                    acquisition::RawRequest {
                        countries: Arc::clone(&country_snapshot),
                        context: projects_ctx,
                        payload: decode::acquire_projects_job(),
                    }
                )))
            } else {
                frame.extended.projects_pilot().raw_mode = false;
                None
            };
            // Raw admission already decides these modes; joins only stage rows or failures.
            let intel_ctx = frame
                .extended
                .intel_pilot()
                .intel_copy_context(shared.base, state);
            frame.extended.radar_pilot().radar_raw_mode = radar_pending.is_some();
            frame.extended.radar_pilot().registrations_raw_mode = true;
            frame.extended.intel_pilot().intel_raw_mode = true;
            frame.extended.projects_pilot().raw_mode =
                projects_pending.is_some() || projects_omitted;
            frame.extended.construction_rest_pilot().rest_raw_mode = construction_active;
            let (gated_pending, gated_projects_pending) = out_of_line(|| {
                let mut request = frame.extended.take_gated_request(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { take_slice_store(shared, crate::slicing::PROJECTS) },
                    Some(Arc::clone(&country_snapshot)),
                );
                let projects = request.take_projects();
                let projects = queue!(acquisition::Job::GatedProjects(projects));
                (queue!(acquisition::Job::Gated(request)), projects)
            });
            let lexical_ctx = extended::lexical::LexicalCopyContext { base: shared.base };
            let lexical_pending = if lexical_refresh {
                Some(queue!(acquisition::Job::Lexical(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: lexical_ctx,
                    payload: decode::acquire_lexical_job(),
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
            let session_due =
                replay_data::cadence::should_walk(TableId::WorldTension, clock.hour, force_full);
            let session_omitted = session_due && !frame.extended.records_any(&decode::SESSION);
            let session_pending = if session_due && !session_omitted {
                Some(queue!(acquisition::Job::Session(acquisition::RawRequest {
                    countries: Arc::clone(&country_snapshot),
                    context: session_ctx,
                    payload: decode::acquire_session_job(),
                })))
            } else {
                frame.extended.session_pilot().raw_mode = session_omitted;
                None
            };
            // SAFETY: only this callback takes and returns the stores.
            let (launch_store, state_stores) = unsafe {
                (
                    take_slice_store(shared, crate::slicing::RAID_LAUNCH),
                    crate::extended::RaidStateStores {
                        states: take_slice_store(shared, crate::slicing::RAID_STATE),
                        buildings: take_slice_store(shared, crate::slicing::RAID_BUILDINGS),
                        routes: take_slice_store(shared, crate::slicing::TRADE_ROUTES),
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
            let raid_targets_pending = frame
                .extended
                .records_any(&extended::raid_target_statuses::TABLES)
                .then(|| {
                    queue!(acquisition::Job::RaidTargets(
                        frame.extended.take_raid_targets_request(
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                            force_full,
                        )
                    ))
                });
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
            let supply_trains_parts =
                if frame.extended.records_any(&extended::supply_trains::TABLES) {
                    SUPPLY_PARTS
                } else {
                    0
                };
            for (part, pending) in supply_trains_pending
                .iter_mut()
                .enumerate()
                .take(supply_trains_parts)
            {
                // Part 0 carries the family's owner selection and store.
                let fingerprints = (part == 0).then(|| {
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { take_slice_store(shared, crate::slicing::SUPPLY_TRAINS) }
                });
                *pending = Some(queue!(acquisition::Job::SupplyTrains(
                    frame.extended.take_supply_trains_request(
                        &country_snapshot,
                        shared.base,
                        state,
                        clock.hour,
                        force_full,
                        part,
                        fingerprints,
                    )
                )));
            }
            // The battles, participants and casualty visitors the map sample
            // installs; joined right before it.
            let progress = frame.extended.take_combat_progress();
            let progress_recorded = frame
                .extended
                .records_any(&extended::combat_progress::TABLES);
            let combat_request =
                frame
                    .map
                    .take_combat_request(shared.base, state, progress, progress_recorded);
            let combat_pending = queue!(acquisition::Job::MapCombats(combat_request));
            // RaidAirScores and the shared rail tables read only the game
            // state and the hour, with the held score table and rail store on
            // loan; the Late job keeps the tables they leave in the packs.
            let scores_rails_pending = queue!(acquisition::Job::ScoresRails({
                // SAFETY: only this callback takes and returns these stores.
                let air_scores = unsafe { &mut *shared.raid_air_scores.get() }
                    .take()
                    .expect("raid air score loan");
                // SAFETY: as above.
                let rail_store = unsafe { (*shared.rail_store.get()).take() };
                frame.extended.take_scores_rails_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    air_scores,
                    rail_store,
                )
            }));
            // Regional Air supply reads only the game state; Ground Crews takes it at the Late job.
            let mut air_supply_pending = if replay_data::cadence::should_walk(
                TableId::AirGroundCrews,
                clock.hour,
                force_full,
            ) && frame.extended.records_any(&[TableId::AirGroundCrews])
            {
                Some(queue!(acquisition::Job::AirSupply(
                    frame
                        .extended
                        .take_air_supply_request(&country_snapshot, shared.base, state)
                )))
            } else {
                None
            };
            release!();
            segment!(0);
            // Readiness, not a fixed join order, advances the early dependency
            // graph. Each handler publishes the same verdicts and returns the
            // same loans before submitting its consumers. Split walks still
            // collect in slot order. Failed stitches retain the pre-Naval
            // admission decision and settle only after Naval and its wings.
            let inline_walk_begin = qpc();
            let inline_wait_begin = shared.acquisition_wait_ns.load(Ordering::Relaxed);
            let inline_dispatch_begin = shared.acquisition_dispatch_ns.load(Ordering::Relaxed);
            let inline_release_begin = shared.game_thread.release_ns.load(Ordering::Relaxed);
            let mut root_pending = Some(root_pending);
            let mut input_pending = Some(input_pending);
            let mut ship_models_pending = Some(ship_models_pending);
            let mut countries_pending = Some(countries_pending);
            let mut air_pending = Some(air_pending);
            let mut combat_pending = Some(combat_pending);
            let mut order_joins: Option<(
                PendingAcquisition,
                [Option<PendingAcquisition>; crate::orders_parts::MAX_PARTS - 1],
            )> = None;
            let mut stitch_pending = None;
            let mut stitch_failure = StitchFailureBarrier::default();
            // Unread bound fronts fail at the plans' capture, reading nothing.
            let fronts_recorded = frame.extended.records_any(&[TableId::Fronts]);
            let mut stitched_orders = None;
            let mut naval_pending = None;
            let mut child_fronts_pending = None;
            let mut fronts_pending = None;
            let mut divisions_pending = None;
            let mut army_icons_pending = None;
            let mut air_missions_pending = None;
            let mut movement_pending = None;
            let mut speed_terms_pending = None;
            let mut intel_pending = None;
            let mut province_unit_order_pending = None;
            let mut model_inputs_pending = None;
            let mut ordinary_pending = None;
            let mut provinces_pending = None;
            let mut states_pending = None;
            let mut weather_pending = None;
            let mut capitulation_pending = None;
            let mut wars_pending = None;
            let mut relations_pending = None;
            let mut character_inputs_pending = None;
            let mut character_portraits_pending = [const { None }; PORTRAIT_PARTS];
            let mut ship_detail_pending = [const { None }; extended::ship_models::MAX_PARTS];
            let mut air_histories_pending = [const { None }; AIR_HISTORY_PARTS];
            let mut late_pending = None;
            let mut inputs_done = false;
            let mut divisions_done = false;
            let mut ships_done = false;
            let mut countries_done = false;
            let mut characters_done = false;
            let mut wings_done = false;
            let mut verdicts_done = false;
            let mut naval_done = false;
            let mut map_done = false;
            let mut child_done = false;
            let mut icons_done = false;
            let mut missions_done = false;
            let mut naval_install_ticks = 0;
            let mut naval_ns = [0; 3];
            let mut orders_ns = 0;
            let mut map_ticks = [0; crate::map_capture::MAP_TABLES];
            let mut inputs = None;
            let mut division_input_failure = None;
            let mut divisions_failure = None;
            let mut air_failure = None;
            let mut naval_failure = None;
            let mut players_failure = None;
            let mut radar_failure = None;
            let mut projects_failure = projects_omitted.then_some(extended::NOT_RECORDED);
            let mut air_stats_failure = air_stats_omitted.then_some(extended::NOT_RECORDED);
            let mut supremacy_failure = None;
            let mut sunk_failure = sunk_omitted.then_some(extended::NOT_RECORDED);
            let mut world_active = Default::default();
            let mut world_report = acquisition::WorldReport::default();
            let mut world_failure = None;
            let mut economy_failure = None;
            let mut air_stats_raw = None;
            // A queued AirBases job always settles a raw payload or a failure.
            let mut air_bases_raw = Some(true);
            let mut characters_failure = [None; CHARACTER_PARTS];
            frame.extended.characters_pilot().raw_active = [false; extended::characters::N];
            macro_rules! join_characters {
                ($part:expr) => {{
                    let part = $part;
                    if let Some(pending) = characters_pending[part].take() {
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        out_of_line(|| {
                            let (request, report, failure) = completed!(pending, Characters);
                            // Return any census loan before the roster job can recycle.
                            drop(request.roles);
                            characters_report[part] = report;
                            let failure = failure.or((report.plan_failed || !report.raw_ok)
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
                        });
                    }
                }};
            }

            let mut intel_residual_pending = Some(intel_residual_pending);
            let mut session_pending = session_pending;
            let mut variants_pending = Some(variants_pending);
            let mut capitulation_failure = None;
            let mut session_failure = session_omitted.then_some(extended::NOT_RECORDED);
            let mut relations_failure = None;
            let mut final_inputs_pending = None;
            let mut final_sources_done = false;
            let mut final_sources_publish = 0;
            let mut final_inputs_armed = false;
            macro_rules! join_final_sources {
                () => {{
                    // Out of line: the job-sized request stays off the hourly body's frame.
                    out_of_line(|| {
                        let (intel_residual_request, intel_residual_report) = collected!(
                            intel_residual_pending
                                .take()
                                .expect("residual Intel producer"),
                            IntelResidual
                        );
                        let (flags, variables) = frame.extended.restore_intel_residual_request(
                            intel_residual_request,
                            intel_residual_report,
                        );
                        if let Some(store) = flags {
                            let bytes = store.allocated_bytes() as u64;
                            let previous = shared
                                .script_flag_store_bytes
                                .swap(bytes, Ordering::Relaxed);
                            shared
                                .pool_bytes
                                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                            // SAFETY: the IntelResidual job has joined; no worker retains this store.
                            unsafe { *shared.script_flag_store.get() = Some(store) };
                        }
                        if let Some(store) = variables {
                            let bytes = store.allocated_bytes() as u64;
                            let previous = shared
                                .script_variable_store_bytes
                                .swap(bytes, Ordering::Relaxed);
                            shared
                                .pool_bytes
                                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                            // SAFETY: the IntelResidual job has joined; no worker retains this store.
                            unsafe { *shared.script_variable_store.get() = Some(store) };
                        }
                    });
                    // Out of line: the job-sized request stays off the hourly body's frame.
                    capitulation_failure = out_of_line(|| {
                        let (request, report, failure) = completed!(
                            capitulation_pending.take().expect("capitulation producer"),
                            Capitulation
                        );
                        let failure = failure.or((!report.raw_ok)
                            .then_some("Capitulation owned acquisition could not complete"));
                        capitulation_report = report;
                        frame.extended.capitulation_pilot().capitulation_raw_mode =
                            report.raw_ok || failure.is_some();
                        if report.raw_ok && failure.is_none() {
                            *capitulation_payload = Some(request.payload);
                        } else {
                            decode::recycle_capitulation_job(request.payload);
                        }
                        failure
                    });

                    if let Some(pending) = session_pending.take() {
                        let (request, report, failure) = completed!(pending, Session);
                        let failure = failure.or((!report.raw_ok)
                            .then_some("Session owned acquisition could not complete"));
                        session_report = report;
                        session_failure = failure;
                        frame.extended.session_pilot().raw_mode =
                            report.raw_ok || failure.is_some();
                        if report.raw_ok && failure.is_none() {
                            *session_payload = Some(request.payload);
                        } else {
                            decode::recycle_session_job(request.payload);
                        }
                    }

                    if let Some(pending) = relations_pending.take() {
                        let (request, report, failure) = completed!(pending, Relations);
                        let failure = failure.or((!report.raw_ok)
                            .then_some("Relations original acquisition could not complete"));
                        let (fingerprints, payload) = frame
                            .extended
                            .relations_pilot()
                            .join_relations(request.payload, report, failure);
                        if fingerprints.is_some() {
                            // SAFETY: only this callback takes and returns the store.
                            unsafe {
                                *shared.slice_fingerprints[crate::slicing::RELATIONS].get() =
                                    fingerprints
                            };
                        }
                        relations_report = report;
                        relations_failure = failure;
                        match payload {
                            Ok(payload) => *relations_payload = Some(payload),
                            Err(payload) => decode::recycle_relations_job(payload),
                        }
                    }
                }};
            }
            while late_pending.is_none() || final_inputs_pending.is_none() {
                if verdicts_done && divisions_done {
                    stitch_failure.reach(&batch);
                }
                if final_sources_done
                    && late_pending.is_some()
                    && !final_inputs_armed
                    && !batch.stopped()
                    && !crate::allocation::failed()
                {
                    // Successful source joins installed the actual packs. The
                    // remaining raw jobs contribute only their owner masks;
                    // failure still settles at their original ordered joins.
                    frame.extended.settle_intel_residual(
                        &cached,
                        extended::intel::CaptureContext {
                            base: shared.base,
                            game_state: state,
                            hour: clock.hour,
                            force_full,
                        },
                    );
                    let mut required = 0;
                    for (part, pending) in characters_pending.iter().enumerate().skip(1) {
                        if pending.is_some() {
                            required |= 1 << part;
                            // Each admitted raw join sets raw_active on success
                            // or failure; only success releases this continuation.
                            for (raw, active) in frame
                                .extended
                                .characters_pilot()
                                .raw_active
                                .iter_mut()
                                .zip(character_part(part))
                            {
                                *raw |= active;
                            }
                        }
                    }
                    out_of_line(|| {
                        final_inputs_dispatch.arm(
                            frame.extended.take_final_inputs_request(
                                &country_snapshot,
                                shared.base,
                                state,
                                clock.hour,
                                force_full,
                            ),
                            required,
                            final_sources_publish,
                        );
                    });
                    final_inputs_armed = true;
                }
                let wait_begin = qpc();
                let mut input_handoff = None;
                let step = batch.wait_ready(|batch| {
                    macro_rules! ready {
                        ($pending:expr) => {
                            $pending
                                .as_ref()
                                .is_some_and(|pending| pending_complete(batch, pending))
                        };
                    }
                    macro_rules! ready_step {
                        ($step:ident, $condition:expr) => {{
                            crate::acquisition_diagnostics::probe_condition();
                            if $condition {
                                return Some(EarlyStep::$step);
                            }
                        }};
                    }
                    let all = |first: &Option<PendingAcquisition>,
                               parts: &[Option<PendingAcquisition>]| {
                        ready!(first)
                            && parts
                                .iter()
                                .flatten()
                                .all(|pending| pending_complete(batch, pending))
                    };
                    // A published input can still have a publisher holding the fan-in lock.
                    ready_step!(
                        Inputs,
                        all(&input_pending, &part_pending) && {
                            input_handoff = presentation_inputs.try_close();
                            input_handoff.is_some()
                        }
                    );
                    ready_step!(Presentation, presentation.ready(batch));
                    ready_step!(Roots, ready!(root_pending));
                    ready_step!(Divisions, ready!(divisions_pending));
                    ready_step!(ArmyIcons, ready!(army_icons_pending));
                    ready_step!(
                        Orders,
                        order_joins
                            .as_ref()
                            .is_some_and(|(orders, parts)| orders_complete(batch, orders, parts))
                    );
                    ready_step!(Ships, ready!(ship_models_pending));
                    ready_step!(Countries, ready!(countries_pending));
                    ready_step!(
                        Characters,
                        !characters_done
                            && characters_pending[0]
                                .as_ref()
                                .is_none_or(|pending| pending_complete(batch, pending))
                    );
                    ready_step!(Wings, all(&air_pending, &air_part_pending));
                    ready_step!(
                        Verdicts,
                        !verdicts_done
                            && inputs_done
                            && ships_done
                            && countries_done
                            && characters_done
                            && wings_done
                            && [
                                radar_pending.as_ref(),
                                projects_pending.as_ref(),
                                Some(&economy_pending),
                                world_pending.as_ref(),
                                Some(&players_pending),
                                air_stats_pending.as_ref(),
                                supremacy_pending.as_ref(),
                                sunk_pending.as_ref(),
                            ]
                            .into_iter()
                            .flatten()
                            .all(|pending| pending_complete(batch, pending))
                    );
                    ready_step!(
                        Stitch,
                        stitch_pending
                            .as_ref()
                            .is_some_and(|pending| stitch_can_advance(batch, pending, false))
                    );
                    ready_step!(
                        Naval,
                        (wings_done && stitched_orders.is_some() && ready!(naval_pending))
                            || stitch_failure.ready(
                                batch,
                                stitch_pending.as_ref(),
                                naval_pending.as_ref(),
                                wings_done,
                            )
                    );
                    ready_step!(
                        Map,
                        verdicts_done
                            && divisions_done
                            && naval_done
                            && !map_done
                            && ready!(combat_pending)
                    );
                    ready_step!(ChildFronts, ready!(child_fronts_pending));
                    ready_step!(
                        Missions,
                        map_done
                            && !missions_done
                            && ready!(air_missions_pending)
                            && pending_complete(batch, &scores_rails_pending)
                            && air_supply_pending
                                .as_ref()
                                .is_none_or(|pending| pending_complete(batch, pending))
                    );
                    ready_step!(
                        FinalSources,
                        !final_sources_done
                            && verdicts_done
                            && icons_done
                            && !batch.stopped()
                            && final_sources_published(
                                batch,
                                [
                                    intel_residual_pending.as_ref(),
                                    capitulation_pending.as_ref(),
                                    session_pending.as_ref(),
                                    relations_pending.as_ref(),
                                ],
                            )
                            .is_some()
                    );
                    ready_step!(
                        Late,
                        late_pending.is_none()
                            && map_done
                            && child_done
                            && icons_done
                            && missions_done
                    );
                    ready_step!(
                        FinalInputs,
                        final_inputs_pending.is_none()
                            && final_inputs_ready(
                                batch,
                                late_pending.is_some(),
                                [
                                    intel_residual_pending.as_ref(),
                                    capitulation_pending.as_ref(),
                                    session_pending.as_ref(),
                                    relations_pending.as_ref(),
                                ]
                                .into_iter()
                                .chain(characters_pending[1..].iter().map(Option::as_ref)),
                            )
                    );
                    None
                });
                shared
                    .acquisition_wait_ns
                    .fetch_add(shared.ns(qpc() - wait_begin), Ordering::Relaxed);
                let _step = EarlyStepTimer::new(
                    shared,
                    &shared.presentation.early_steps[step as usize],
                    step,
                );
                match step {
                    EarlyStep::Presentation => {
                        // SAFETY: readiness proved publication; no preparation wait here.
                        unsafe { presentation.advance(shared, &mut batch) };
                    }
                    EarlyStep::Roots => {
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        let (naval_roots_ticks, mut naval_job) = out_of_line(|| {
                            let (root_request, naval_roots_ticks, root_failure) =
                                completed!(root_pending.take().expect("root producer"), OrderRoots);
                            frame.map.orders = root_request.orders;
                            let mut naval_job = root_request.naval;
                            if let Some(reason) = root_failure {
                                frame.map.orders.fail_roots(reason);
                                naval_job.fail_roots(reason);
                            }
                            (naval_roots_ticks, naval_job)
                        });
                        // Out of line: building the request and its submission closure
                        // must not leave two request-sized values in the hourly body.
                        let (parts, pending) = out_of_line(|| {
                            let mut orders_request = frame.extended.take_orders_request(
                                &country_snapshot,
                                &mut frame.map.orders,
                                shared.base,
                                state,
                            );
                            // The order census and its plans need only the root census: they
                            // leave in parts over ranges of the groups (`orders_parts`): the
                            // Orders job walks part 0, an OrdersPart job each other part. The
                            // Naval copy leaves with them; ChildFronts waits for both.
                            let mut order_part_pending: [Option<PendingAcquisition>;
                                crate::orders_parts::MAX_PARTS - 1] =
                                [const { None }; crate::orders_parts::MAX_PARTS - 1];
                            for index in 1..orders_request.orders.part_count() {
                                if let Some(part) = orders_request.orders.take_part(index) {
                                    order_part_pending[index - 1] =
                                        Some(queue!(acquisition::Job::OrdersPart(
                                            acquisition::OrdersPartRequest {
                                                base: shared.base,
                                                index,
                                                part,
                                                rejected: false,
                                            }
                                        )));
                                }
                            }
                            let orders_pending = queue!(acquisition::Job::Orders(orders_request));
                            (order_part_pending, orders_pending)
                        });
                        // The Naval copy's raw phase reads no wing value: the wing census
                        // is installed when it returns. Behind the wing
                        // join it was the next join the game thread waited for (0.18 ms
                        // per hour at 1943, int_761c274).
                        let naval_ctx = extended::naval_history::NavalCopyContext {
                            base: shared.base,
                            game_state: state,
                            hour: clock.hour,
                            force_full,
                        };
                        naval_install_ticks = naval_roots_ticks;
                        naval_job.defer_wings();
                        naval_pending =
                            Some(queue!(acquisition::Job::Naval(acquisition::RawRequest {
                                countries: Arc::clone(&country_snapshot),
                                context: naval_ctx,
                                payload: naval_job,
                            })));
                        order_joins = Some((pending, parts));
                        // The slack burst: first-burst jobs whose joins come long after
                        // their bodies end leave here, behind the Naval copy and the
                        // Orders parts, in one held burst. In the first burst they took
                        // workers 1.4-2.4 ms into the hour, once the priority-0 jobs were
                        // claimed, and still held them when the Orders parts and the Naval
                        // copy were queued (1.96 and 2.08 ms), which then waited 0.24 and
                        // 0.34 ms for a worker (1943, int_2d1ede901c). Queued with the
                        // Naval copy, waiting as long as any job of that segment waited
                        // for a worker (0.77/0.44 ms at 1943/1936) and running half as
                        // long again as their mean body, each still ends before its join:
                        // the task forces 0.95/0.32 ms before it, provinces 1.11/0.81,
                        // weather, capitulation, states, wars and relations 2.2/1.2 or
                        // more. Only the queue position moves: each job takes the same
                        // request and stores and reads the same sources in the same order,
                        // with the same failures.
                        batch.hold();
                        for (slot, part) in ordinary_parts_pending
                            .iter_mut()
                            .zip(ordinary_slack_parts.iter_mut())
                        {
                            if let Some(part) = part.take() {
                                *slot = Some(queue!(acquisition::Job::OrdinaryPart(part)));
                            }
                        }
                        provinces_pending = Some(queue!(acquisition::Job::Provinces({
                            let mut request = acquisition::ProvincesRequest::take(
                                shared.base,
                                state,
                                &mut frame.provinces,
                            );
                            // SAFETY: only this callback takes and returns the store.
                            if let Some(store) = unsafe { (*shared.area_store.get()).take() } {
                                request.payload.area_store = Some(store);
                            } else if let Some(store) = request.payload.area_store.as_mut() {
                                // A store an unfinished hour left in this frame certifies nothing.
                                store.forget();
                            }
                            if let Some(store) = request.payload.area_store.as_mut() {
                                store.force_full = force_full;
                            }
                            // SAFETY: only this callback takes and returns the store.
                            if let Some(store) = unsafe { (*shared.province_store.get()).take() } {
                                request.payload.province_store = Some(store);
                            } else if let Some(store) = request.payload.province_store.as_mut() {
                                // A store an unfinished hour left in this frame certifies nothing.
                                store.forget();
                            }
                            if let Some(store) = request.payload.province_store.as_mut() {
                                store.force_full = force_full;
                            }
                            request
                        })));
                        states_pending = Some(queue!(acquisition::Job::States({
                            let mut request = frame.take_states_request(shared.base, state);
                            request.countries = Some(Arc::clone(&country_snapshot));
                            // SAFETY: only this callback takes and returns the store.
                            request.store = unsafe { (*shared.state_store.get()).take() };
                            // Without its store the States job reads no occupation.
                            if frame.extended.records_any(&[TableId::StateResistance]) {
                                request.occupation_store =
                                    unsafe { (*shared.occupation_store.get()).take() };
                            }
                            request
                        })));
                        let weather_ctx = extended::weather::WeatherCopyContext {
                            game_state: state,
                            base: shared.base,
                        };
                        weather_pending = Some({
                            let mut payload = decode::acquire_weather_job();
                            // SAFETY: game thread; the previous callback returned it.
                            payload.carry = unsafe { &mut *shared.weather_carry.get() }.take();
                            if let Some(carry) = payload.carry.as_mut() {
                                carry.begin(
                                    clock.hour,
                                    force_full,
                                    crate::country_input_changes::weather_writes(),
                                );
                            }
                            queue!(acquisition::Job::Weather(acquisition::RawRequest {
                                countries: Arc::clone(&country_snapshot),
                                context: weather_ctx,
                                payload,
                            }))
                        });
                        let capitulation_ctx = extended::intel::CapitulationCopyContext {
                            base: shared.base,
                            game_state: state,
                        };
                        capitulation_pending = Some({
                            queue!(acquisition::Job::Capitulation(acquisition::RawRequest {
                                countries: Arc::clone(&country_snapshot),
                                context: capitulation_ctx,
                                payload: decode::acquire_capitulation_job(),
                            }))
                        });
                        wars_pending = Some(queue!(acquisition::Job::Wars({
                            let mut request =
                                frame.take_wars_request(&country_snapshot, shared.base, state);
                            // SAFETY: only this callback takes and returns the store.
                            request.store = unsafe { (*shared.war_store.get()).take() };
                            request
                        })));
                        relations_pending = if relations_refresh {
                            Some(queue!(acquisition::Job::Relations(
                                acquisition::RawRequest {
                                    countries: Arc::clone(&country_snapshot),
                                    context: relations_ctx,
                                    payload: {
                                        let mut payload = decode::acquire_relations_job();
                                        // The hour's owner slice and the family's store go with the
                                        // job and come back at its join. The slice was taken from the
                                        // real Intel pack before IntelResidual borrowed it.
                                        payload.selection = relations_selection.take();
                                        // SAFETY: only this callback takes and returns the store.
                                        payload.fingerprints = Some(unsafe {
                                            take_slice_store(shared, crate::slicing::RELATIONS)
                                        });
                                        payload
                                    },
                                }
                            )))
                        } else {
                            None
                        };
                        release!();
                    }
                    EarlyStep::Inputs => {
                        let mut input_handoff = input_handoff.take().expect("closed input handoff");
                        let (mut input_request, inputs_ok, mut input_failure) = completed!(
                            input_pending.take().expect("input producer"),
                            DivisionInputs
                        );
                        let consumers_begin = qpc();
                        let consumers_wait = shared.acquisition_wait_ns.load(Ordering::Relaxed);
                        // Published tickets no longer borrow the closed fan-in.
                        let mut parts = [const { None }; crate::division_census::MAX_PARTS - 1];
                        for (part, pending) in
                            std::mem::take(&mut part_pending).into_iter().enumerate()
                        {
                            if let Some(pending) = pending {
                                let (request, _, failure) = completed!(pending, DivisionInputParts);
                                parts[part] = Some((request.census, failure));
                            }
                        }
                        let restore_begin = qpc();
                        let (joined, join_failure) =
                            input_handoff.restore_master(&mut input_request.payload.census);
                        shared
                            .presentation
                            .input_restore_ns
                            .fetch_add(shared.ns(qpc() - restore_begin), Ordering::Relaxed);
                        input_request.payload.presentation = None;
                        shared.presentation.store_wait_ns.fetch_add(
                            shared.ns(input_handoff.store_wait_ticks() as i64),
                            Ordering::Relaxed,
                        );
                        input_failure = input_failure.or(join_failure);
                        // The parts join the census in slot order, as one walk would
                        // have left it. The first failure in slot order is the hour's:
                        // the census's own refusal, or a part that did not run.
                        let mut joining = input_failure.is_none();
                        for (part, captured) in parts.into_iter().enumerate() {
                            let Some((mut captured, failure)) = captured else {
                                if joining && part < split {
                                    input_failure = Some("division input part not dispatched");
                                    joining = false;
                                }
                                continue;
                            };
                            input_handoff.restore_part(part, &mut captured);
                            if joining && !joined {
                                let census = &mut input_request.payload.census;
                                let result = match failure {
                                    Some(reason) => Err(reason),
                                    None => {
                                        let begin = qpc();
                                        let result = census.absorb(&mut captured);
                                        crate::acquisition_diagnostics::update(|hour| {
                                            hour.division_merge_ns += shared.ns(qpc() - begin)
                                        });
                                        result
                                    }
                                };
                                if let Err(reason) = result {
                                    input_failure = Some(reason);
                                }
                                joining = input_failure.is_none() && census.joinable();
                            }
                            // SAFETY: the part has joined; only this callback returns it.
                            unsafe { (*shared.division_parts.get())[part] = Some(captured) };
                        }
                        input_request.payload.overflowed |= input_request.payload.census.overflowed;
                        let inputs_ok = inputs_ok && !input_request.payload.overflowed;
                        // SAFETY: only this callback reads and writes the split.
                        unsafe {
                            let (starts, split) = &mut *shared.division_part_starts.get();
                            *split = input_request.payload.census.part_starts(starts);
                            *shared.division_join_capacity.get() =
                                input_request.payload.census.join_capacity();
                        }
                        inputs = if inputs_ok && input_failure.is_none() {
                            input_request.payload.census.unit_inputs().cloned()
                        } else {
                            input_request.payload.inputs_captured = false;
                            None
                        };
                        frame.extended.install_unit_inputs(inputs.clone());
                        shared.presentation.input_consumers_ns.fetch_add(
                            shared.ns(qpc() - consumers_begin).saturating_sub(
                                shared
                                    .acquisition_wait_ns
                                    .load(Ordering::Relaxed)
                                    .saturating_sub(consumers_wait),
                            ),
                            Ordering::Relaxed,
                        );
                        batch.hold();
                        divisions_pending =
                            Some(queue!(acquisition::Job::Divisions(input_request)));
                        if shared.capture_mode == replay_data::scope::CaptureMode::Map {
                            model_inputs_pending =
                                Some(queue!(acquisition::Job::DivisionModelInputs(
                                    frame.extended.take_division_model_inputs_request(
                                        shared.base,
                                        state,
                                        inputs.clone(),
                                    )
                                )));
                        }
                        // Presentation needs only the country snapshot and the unit inputs
                        // installed above; it leaves as soon as those exist, its status
                        // side, movement and speed terms as independent jobs.
                        // Out of line: the job-sized requests stay off the hourly body's frame.
                        let (pending, movement, speed_terms, split, omitted) =
                            out_of_line(|| {
                                let dispatched = input_handoff.take_dispatched();
                                let retained = input_handoff.take_request();
                                drop(input_handoff);
                                if let Some((pending, movement, speed_terms, split)) = dispatched
                                {
                                    assert!(retained.is_none());
                                    let (ticket, parked) = batch.adopt_presentation_ticket(pending);
                                    let pending = if parked {
                                        PendingAcquisition::Rejected(ticket)
                                    } else {
                                        PendingAcquisition::Accepted(ticket)
                                    };
                                    let speed_terms = queue!(
                                        acquisition::Job::PresentationSpeedTerms(speed_terms)
                                    );
                                    let movement =
                                        queue!(acquisition::Job::PresentationMovement(movement));
                                    return (
                                        Some(pending),
                                        movement,
                                        Some(speed_terms),
                                        split,
                                        None,
                                    );
                                }
                                let mut presentation_request =
                                    retained.expect("presentation retained");
                                if status_recorded && presentation_request.army_stats.is_none() {
                                    presentation_request.army_stats = shared.take_army_stat_store();
                                }
                                if presentation_request.army_stats.is_some() {
                                    presentation_request.captured.enable_owned_settlement();
                                }
                                presentation_request.inputs = inputs.clone();
                                presentation_request.captured.status_workers =
                                    acquisition.worker_count();
                                let mut movement_request = presentation_request.take_movement();
                                if !status_recorded {
                                    movement_request.pass.omit_status();
                                    // Held to the final join, which installs the movement pass.
                                    let movement_pending = queue!(
                                        acquisition::Job::PresentationMovement(movement_request)
                                    );
                                    return (
                                        None,
                                        movement_pending,
                                        None,
                                        false,
                                        Some(presentation_request),
                                    );
                                }
                                let speed_terms_request = presentation_request.take_speed_terms();
                                let divisions =
                                    presentation_request.inputs.as_deref().map_or(0, |inputs| {
                                        inputs.outcome().len().min(inputs.seeds().len())
                                    });
                                let split = extended::division_presentation::status_parts_for(
                                    divisions,
                                    acquisition.worker_count(),
                                ) > 1;
                                let presentation_pending = if split {
                                    batch.arm_presentation(&mut presentation_request);
                                    queue!(acquisition::Job::PresentationPrepare(
                                        presentation_request
                                    ))
                                } else {
                                    queue!(acquisition::Job::Presentation(presentation_request))
                                };
                                let speed_terms_pending = queue!(
                                    acquisition::Job::PresentationSpeedTerms(speed_terms_request)
                                );
                                let movement_pending = queue!(
                                    acquisition::Job::PresentationMovement(movement_request)
                                );
                                (
                                    Some(presentation_pending),
                                    movement_pending,
                                    Some(speed_terms_pending),
                                    split,
                                    None,
                                )
                            });
                        intel_pending = intel_recorded.then(|| {
                            queue!(acquisition::Job::Intel(acquisition::IntelRequest {
                                countries: Arc::clone(&country_snapshot),
                                context: intel_ctx,
                                payload: decode::acquire_intel_job(),
                                inputs: inputs.clone(),
                                input_failure: input_failure.or((!inputs_ok)
                                    .then_some("division input acquisition could not complete")),
                            }))
                        });
                        // ProvinceUnitOrder takes its division occupants from the census
                        // the inputs job acquired; without it the walk reads them all.
                        province_unit_order_pending =
                            Some(queue!(acquisition::Job::ProvinceUnitOrder(
                                frame.extended.take_province_unit_order_request(
                                    shared.base,
                                    state,
                                    inputs.clone(),
                                    // SAFETY: only this callback takes and returns the store.
                                    unsafe { (*shared.province_unit_store.get()).take() },
                                )
                            )));
                        release!();
                        if let Some(pending) = pending {
                            if split {
                                presentation.prepare = Some(pending);
                            } else {
                                presentation.pending = Some(pending);
                            }
                        }
                        presentation.omitted = omitted;
                        movement_pending = Some(movement);
                        speed_terms_pending = speed_terms;
                        division_input_failure = input_failure;
                        inputs_done = true;
                    }
                    EarlyStep::Ships => {
                        // Adopt publisher-released details; only an unreleased census uses the fallback.
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        let dispatched = out_of_line(|| {
                            // SAFETY: this thread still owns the live acquisition boundary.
                            let (mut ship_models_request, mut ship_models_report) = unsafe {
                                collect_acquisition(
                                    shared,
                                    &mut batch,
                                    ship_models_pending.take().expect("ship census"),
                                    |job| {
                                        let acquisition::Job::ShipModels(request) = job else {
                                            unreachable!("ShipModels ticket type")
                                        };
                                        request
                                    },
                                    |report| {
                                        let acquisition::Report::ShipModels = report else {
                                            unreachable!("ShipModels report type")
                                        };
                                    },
                                )
                            };
                            let dispatched = ship_models_request.details.take().and_then(
                                |details| match details.take(&mut ship_models_request.captured) {
                                    Ok(pending) => pending,
                                    Err(failure) => {
                                        ship_models_report = Err(failure);
                                        None
                                    }
                                },
                            );
                            frame.map.naval = frame.extended.restore_ship_models_request(
                                ship_models_request,
                                ship_models_report,
                            );
                            dispatched.map(|parts| {
                                parts.map(|pending| {
                                    pending.map(|pending| {
                                        let (ticket, parked) =
                                            batch.adopt_presentation_ticket(pending);
                                        if parked {
                                            PendingAcquisition::Rejected(ticket)
                                        } else {
                                            PendingAcquisition::Accepted(ticket)
                                        }
                                    })
                                })
                            })
                        });
                        // Written after the map-tick copy below, which covers these indices.
                        naval_ns = frame.map.naval.ticks.map(|cost| shared.ns(cost as i64));
                        // Out of line: the parts' requests stay off the hourly body's frame.
                        ship_detail_pending = dispatched.unwrap_or_else(|| {
                            out_of_line(|| {
                                batch.hold();
                                let pending = frame
                                    .extended
                                    .take_ship_model_parts(&country_snapshot, shared.base)
                                    .map(|request| {
                                        request.map(|request| {
                                            queue!(acquisition::Job::ShipModelDetails(request))
                                        })
                                    });
                                release!();
                                pending
                            })
                        });
                        ships_done = true;
                    }
                    EarlyStep::Countries => {
                        // Restore the original appearance buffers and their completed
                        // first-result journal. The Ordinary counter part reads the
                        // reserved naming they hold: it leaves right after this join.
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        out_of_line(|| {
                            let (countries_request, countries_report, countries_failure) = completed!(
                                countries_pending.take().expect("countries producer"),
                                Countries
                            );
                            let appearance_store = frame.extended.restore_countries_request(
                                countries_request,
                                &mut frame.countries,
                                &mut frame.links,
                                countries_failure,
                            );
                            if let Some(store) = appearance_store {
                                let bytes = store.allocated_bytes() as u64;
                                let previous =
                                    shared.appearance_store_bytes.swap(bytes, Ordering::Relaxed);
                                shared
                                    .pool_bytes
                                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                                // SAFETY: the Countries job has joined; no worker retains this store.
                                unsafe { *shared.appearance_store.get() = Some(store) };
                            }
                            frame.countries_result = countries_report.countries;
                            frame.table_ns[2] = shared.ns(countries_report.countries_ticks);
                            frame.links_result = countries_report.links.result;
                            frame.table_ns[3] = shared.ns(countries_report.links_ticks);
                        });
                        let mut ordinary_request = frame.extended.take_ordinary_request(
                            &country_snapshot,
                            shared.base,
                            state,
                            clock.hour,
                        );
                        // SAFETY: only this callback takes and returns the store.
                        ordinary_request.counters = unsafe { (*shared.counter_store.get()).take() };
                        ordinary_pending =
                            Some(queue!(acquisition::Job::Ordinary(ordinary_request)));
                        countries_done = true;
                    }
                    EarlyStep::Characters => {
                        join_characters!(0);
                        for (part, pending) in character_portraits_pending.iter_mut().enumerate() {
                            let mut request = frame.extended.take_character_portraits_request(
                                shared.base,
                                state,
                                part,
                            );
                            request.census = characters_payload[0]
                                .as_ref()
                                .and_then(|job| job.portrait_census());
                            *pending = Some(queue!(acquisition::Job::CharacterPortraits(request)));
                        }
                        // Role jobs and the character inputs borrow the roster's census
                        // like the portrait shares; without one they walk the roster
                        // independently. They read no raw-copy verdict.
                        let role_census = characters_payload[0]
                            .as_ref()
                            .and_then(|job| job.role_census());
                        character_inputs_pending = Some(queue!(acquisition::Job::CharacterInputs(
                            frame.extended.take_character_inputs_request(
                                shared.base,
                                state,
                                clock.hour,
                                force_full,
                                extended::CharacterInputRoles {
                                    census: role_census.clone(),
                                    // SAFETY: only this callback takes and returns the store.
                                    store: unsafe { (*shared.character_role_store.get()).take() },
                                },
                            )
                        )));
                        for (part, pending) in characters_pending.iter_mut().enumerate().skip(1) {
                            let active = character_part(part);
                            if characters_refresh && active.iter().any(|&value| value) {
                                if !character_part_recorded(part) {
                                    // As a failed part: FinalInputs leaves its tables alone.
                                    for (raw, active) in frame
                                        .extended
                                        .characters_pilot()
                                        .raw_active
                                        .iter_mut()
                                        .zip(active)
                                    {
                                        *raw |= active;
                                    }
                                    characters_failure[part] = Some(extended::NOT_RECORDED);
                                    continue;
                                }
                                *pending = Some(queue!(acquisition::Job::Characters(
                                    acquisition::CharactersRequest {
                                        final_inputs: Some(Arc::clone(&final_inputs_dispatch)),
                                        context: extended::characters::CharactersCopyContext {
                                            active,
                                            ..characters_ctx
                                        },
                                        payload: decode::acquire_characters_job(part),
                                        roles: role_census.as_ref().map_or(
                                            extended::characters::Roles::Independent,
                                            |census| {
                                                extended::characters::Roles::Borrow(Arc::clone(
                                                    census,
                                                ))
                                            },
                                        ),
                                    }
                                )));
                            }
                        }
                        drop(role_census);
                        characters_done = true;
                    }
                    EarlyStep::Wings => {
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        air_failure = out_of_line(|| {
                            let (mut air_request, report, air_failure) =
                                completed!(air_pending.take().expect("wing producer"), AirWing);
                            let mut air_failure = air_failure.or((!report.raw_ok)
                                .then_some("AirWing owned acquisition could not complete"));
                            air_wing_report = report;
                            // SAFETY: only this callback takes and returns the parts.
                            let parts = unsafe { &mut *shared.air_wing_parts.get() };
                            let mut joined = 0;
                            for (part, pending) in std::mem::take(&mut air_part_pending)
                                .into_iter()
                                .enumerate()
                            {
                                let Some(pending) = pending else {
                                    continue;
                                };
                                let (request, report, failure) = completed!(pending, AirWing);
                                air_failure =
                                    air_failure.or(failure).or((!report.raw_ok).then_some(
                                        "AirWing owned acquisition part could not complete",
                                    ));
                                air_wing_report.add(&report);
                                parts[part] = request.payload;
                                joined = part + 1;
                            }
                            let active = &mut parts[..joined];
                            if air_failure.is_none() && air_request.payload.split {
                                let begin = qpc();
                                air_failure = air_request.payload.join_parts(active).err();
                                let ticks = elapsed(begin);
                                air_wing_report.copy_ticks[0] += ticks;
                                crate::acquisition_diagnostics::update(|hour| {
                                    hour.wing_merge_ns += shared.ns(ticks as i64)
                                });
                            }
                            if air_failure.is_none()
                                && let Some(split) =
                                    extended::air::part_starts(&air_request.payload, active)
                            {
                                // SAFETY: only this callback reads and writes the split.
                                unsafe { *shared.air_wing_part_starts.get() = split };
                            }
                            // An idle part keeps its store, and its bytes, for when the
                            // split grows again.
                            let mut store = air_request.payload.wing_store.take();
                            let bytes = extended::air::retained_store_bytes(
                                store.as_deref_mut(),
                                parts,
                                air_failure.is_some(),
                            );
                            if store.is_some() {
                                // SAFETY: the AirWing job has joined; no worker retains this store.
                                unsafe { *shared.wing_store.get() = store };
                            }
                            let previous = shared.wing_store_bytes.swap(bytes, Ordering::Relaxed);
                            shared
                                .pool_bytes
                                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                            if air_wing_report.raw_ok && air_failure.is_none() {
                                *air_wing_payload = Some(air_request.payload);
                            } else {
                                decode::recycle_air_wing_job(air_request.payload);
                            }
                            air_failure
                        });
                        batch.hold();
                        // With the owned wing census the registered missions and the
                        // naval-patrol combats need nothing else: they leave now. The air
                        // statistics and base verdicts, which the Late job reads from the
                        // same pack, are installed when it returns.
                        air_missions_pending = out_of_line(|| {
                            air_wing_payload.as_mut().map(|wings| {
                                frame
                                    .extended
                                    .air_wing_pilot()
                                    .install_owned_mission_identities(wings);
                                queue!(acquisition::Job::AirMissions(
                                    frame.extended.take_air_missions_request(
                                        &country_snapshot,
                                        shared.base,
                                        state,
                                        // SAFETY: only this callback takes and returns the store.
                                        unsafe { (*shared.mission_store.get()).take() },
                                    )
                                ))
                            })
                        });
                        release!();
                        wings_done = true;
                    }
                    EarlyStep::Divisions => {
                        if let Some(pending) = divisions_pending.take() {
                            let (mut request, report, failure) = completed!(pending, Divisions);
                            divisions_report = report;
                            divisions_failure =
                                division_input_failure.or(failure).or((!report.raw_ok)
                                    .then_some("division owned acquisition could not complete"));
                            let source_started = qpc();
                            request.payload.supply_sources(
                                &mut frame.extended,
                                division_source_capacity,
                                divisions_failure,
                            );
                            divisions_report.copy_ticks[0] += elapsed(source_started);
                            *divisions_payload = Some(request.payload);
                            frame.extended.set_divisions_raw_mode(true);
                            // ArmyIconInputs reads the division sources and the seeded
                            // census: it leaves now, and the Late job keeps its rows.
                            army_icons_pending = Some(queue!(acquisition::Job::ArmyIcons(
                                frame.extended.take_army_icons_request(shared.base)
                            )));
                            let ns = shared.ns(divisions_report.copy_ticks[0] as i64);
                            frame.table_ns[6] = ns;
                            decode::add_divisions_copy_ns(0, ns);
                            decode::add_divisions_raw_bytes(0, divisions_report.raw_bytes[0]);
                        }
                        divisions_done = true;
                    }
                    EarlyStep::Verdicts => {
                        if let Some(pending) = radar_pending {
                            let (request, report, failure) = completed!(pending, Radar);
                            let failure = raw_copy_join_failure(
                                failure,
                                report.raw_ok,
                                "Radar owned acquisition could not complete",
                            );
                            radar_report = report;
                            radar_failure = failure;
                            if report.raw_ok && failure.is_none() {
                                *radar_payload = Some(request.payload);
                            } else {
                                decode::recycle_radar_job(request.payload);
                            }
                        }

                        if let Some(pending) = projects_pending {
                            let (request, report, failure) = completed!(pending, Projects);
                            let failure = raw_copy_join_failure(
                                failure,
                                report.raw_ok,
                                "Projects owned acquisition could not complete",
                            );
                            projects_report = report;
                            projects_failure = failure;
                            if report.raw_ok && failure.is_none() {
                                *projects_payload = Some(request.payload);
                            } else {
                                decode::recycle_projects_job(request.payload);
                            }
                        }
                        // World (the production and construction rests) and Economy (the
                        // economy rest) join here, off the early segments: the production
                        // and economy rests' verdicts are the final borrowers'
                        // (dispatched later). Joined after the wing and verdict joins,
                        // their daily slots (3 and 8) run beside them instead of holding
                        // the roster's consumers.
                        // An interrupted accepted job is final for this callback and
                        // refuses only its own tables. Capacity admission failures retain
                        // each family's existing typed fallback.
                        let (economy_request, report, failure) =
                            completed!(economy_pending, Economy);
                        economy_rest_report = report;
                        economy_failure = failure;
                        settle_economy(
                            frame.extended.economy_rest_pilot(),
                            economy_request,
                            &mut economy_rest_report,
                            economy_failure,
                            economy_rest_payload,
                        );
                        if let Some(pending) = world_pending {
                            let (world, report, failure) = completed!(pending, World);
                            world_report = report;
                            world_failure = failure;
                            world_active = settle_world(
                                &mut frame.extended,
                                world,
                                &mut world_report,
                                world_failure,
                                production_rest_payload,
                                construction_rest_payload,
                            );
                        } else {
                            frame.extended.production_rest_pilot().rest_raw_mode =
                                world_scheduled.0;
                            world_failure = Some(extended::NOT_RECORDED);
                            world_active = world_scheduled;
                        }
                        production_rest_report = world_report.production;
                        construction_rest_report = world_report.construction;
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        players_failure = out_of_line(|| {
                            let (request, report, failure) = completed!(players_pending, Players);
                            let failure = failure.or((!report.raw_ok)
                                .then_some("Players owned acquisition could not complete"));
                            players_report = report;
                            frame.map.players_raw_mode = report.raw_ok || failure.is_some();
                            if report.raw_ok && failure.is_none() {
                                *players_payload = Some(request.payload);
                            } else {
                                decode::recycle_players_job(request.payload);
                            }
                            failure
                        });

                        if let Some(pending) = air_stats_pending {
                            let (request, report, failure) = completed!(pending, AirStats);
                            let failure = failure.or((!report.raw_ok)
                                .then_some("AirStats owned acquisition could not complete"));
                            air_stats_report = report;
                            air_stats_failure = failure;
                            air_stats_raw = Some(report.raw_ok || failure.is_some());
                            if report.raw_ok && failure.is_none() {
                                *air_stats_payload = Some(request.payload);
                            } else {
                                decode::recycle_air_stats_job(request.payload);
                            }
                        }

                        // Without the AirMissions job the air pack is home: install the
                        // verdicts now, as the typed census below may read them.
                        if air_missions_pending.is_none() {
                            install_air_verdicts(frame, air_stats_raw.take(), air_bases_raw.take());
                        }

                        if let Some(pending) = supremacy_pending {
                            let (request, report, failure) = completed!(pending, Supremacy);
                            let failure = failure.or((!report.raw_ok)
                                .then_some("Supremacy owned acquisition could not complete"));
                            supremacy_report = report;
                            supremacy_failure = failure;
                            frame.extended.supremacy_pilot().supremacy_raw_mode =
                                report.raw_ok || failure.is_some();
                            if report.raw_ok && failure.is_none() {
                                *supremacy_payload = Some(request.payload);
                            } else {
                                decode::recycle_supremacy_job(request.payload);
                            }
                        }

                        if let Some(pending) = sunk_pending {
                            let (request, report, failure) = completed!(pending, Sunk);
                            let failure = failure.or((!report.raw_ok)
                                .then_some("Sunk owned acquisition could not complete"));
                            sunk_report = report;
                            sunk_failure = failure;
                            frame.extended.sunk_pilot().sunk_raw_mode =
                                report.raw_ok || failure.is_some();
                            if report.raw_ok && failure.is_none() {
                                *sunk_payload = Some(request.payload);
                            } else {
                                decode::recycle_sunk_job(request.payload);
                            }
                        }

                        verdicts_done = true;
                    }
                    EarlyStep::Stitch => {
                        out_of_line(|| {
                            let (mut request, report, failure) = completed!(
                                stitch_pending.take().expect("stitch producer"),
                                OrdersStitch
                            );
                            // Only clean stitches bypass Naval's failure barrier.
                            debug_assert!(!report.contained_panic && failure.is_none());
                            // SAFETY: only the callback reads and writes the split.
                            unsafe { end_order_parts(shared, &request.orders) };
                            if fronts_recorded {
                                fronts_pending = Some(queue!(acquisition::Job::Fronts(
                                    acquisition::FrontsRequest {
                                        base: shared.base,
                                        fronts: request.plans.take_fronts(),
                                    }
                                )));
                            }
                            // Keep the large request in the existing executor
                            // storage until the Naval and wing producers return.
                            // Batch also drains this loan on callback unwind.
                            let mut retained = Some(acquisition::Job::ChildFronts(request));
                            stitched_orders =
                                Some(batch.park(&mut retained).expect("stitched order loan slot"));
                        });
                    }
                    EarlyStep::Naval => {
                        // Out of line: the job-sized requests stay off the hourly body's frame.
                        let (failure, child) = out_of_line(|| {
                            // The Naval copy left right after the root census; it still
                            // returns for decode whatever the census did.
                            let (mut naval_request, mut report, naval_failure) =
                                completed!(naval_pending.take().expect("naval producer"), Naval);
                            // The wing census follows the copy, before anything reads it.
                            let (install_ticks, stands) = naval_request
                                .payload
                                .install_wings_after(air_wing_payload.as_mut());
                            naval_install_ticks += install_ticks;
                            if !stands {
                                report = Default::default();
                            }
                            let naval_failure = naval_failure.or((!report.raw_ok)
                                .then_some("Naval owned acquisition could not complete"));
                            naval_report = report;
                            if naval_report.raw_ok {
                                naval_report.copy_ticks[0] += naval_install_ticks;
                            }
                            if let Some(wings) = air_wing_payload.as_mut() {
                                naval_report.copy_ticks[0] += wings.membership_ticks;
                                if !naval_report.raw_ok && naval_failure.is_none() {
                                    frame.extended.naval_pilot().add_wing_source_ticks(
                                        naval_install_ticks.saturating_add(wings.membership_ticks),
                                    );
                                    frame
                                        .extended
                                        .naval_pilot()
                                        .install_owned_wings(&mut naval_request.payload);
                                }
                                air_wing_report.copy_ticks[0] += frame.map.install_owned_air(wings);
                            }
                            frame.extended.naval_pilot().raw_mode =
                                naval_report.raw_ok || naval_failure.is_some();
                            let mut orders_request = if let Some(parked) = stitched_orders.take() {
                                let acquisition::Job::ChildFronts(request) =
                                    batch.unpark(parked).expect("parked stitched orders")
                                else {
                                    unreachable!("stitched orders retain their child-front request")
                                };
                                request
                            } else {
                                // A failed stitch cannot cancel queued work before
                                // the original Naval join and wing installation.
                                let (mut request, report, failure) = completed!(
                                    stitch_pending.take().expect("deferred stitch producer"),
                                    OrdersStitch
                                );
                                settle_stitch(
                                    &batch,
                                    &mut request,
                                    report,
                                    failure,
                                    stitch_failure.admissible.expect("pre-Naval admission"),
                                );
                                // SAFETY: only the callback reads and writes the split.
                                unsafe { end_order_parts(shared, &request.orders) };
                                if fronts_recorded {
                                    fronts_pending = Some(queue!(acquisition::Job::Fronts(
                                        acquisition::FrontsRequest {
                                            base: shared.base,
                                            fronts: request.plans.take_fronts(),
                                        }
                                    )));
                                }
                                request
                            };
                            if naval_report.raw_ok && naval_failure.is_none() {
                                orders_request.group_source = Some(naval_request.payload);
                            } else {
                                decode::recycle_naval_job(naval_request.payload);
                            }
                            // The child fronts' owner operands read this pass's OOB copy.
                            let child_fronts_pending =
                                queue!(acquisition::Job::ChildFronts(orders_request));
                            (naval_failure, child_fronts_pending)
                        });
                        naval_failure = failure;
                        child_fronts_pending = Some(child);
                        naval_done = true;
                    }
                    EarlyStep::Map => {
                        if let Some(reason) = air_failure {
                            frame.map.fail_air_capture(reason);
                        }
                        let (combat_request, combat_report, combat_failure) =
                            completed!(combat_pending.take().expect("combat producer"), MapCombats);
                        let progress = frame.map.restore_combat_request(
                            combat_request,
                            combat_report,
                            combat_failure,
                        );
                        frame
                            .extended
                            .restore_combat_progress(progress, combat_report.progress_ticks);
                        let sample_begin = qpc();
                        map_ticks = frame.map.sample(
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
                        shared
                            .game_thread
                            .map_sample_ns
                            .fetch_add(shared.ns(qpc() - sample_begin), Ordering::Relaxed);
                        // Otherwise the typed visitor supplied them in the map sample.
                        if air_missions_pending.is_none() {
                            air_missions_pending = Some(queue!(acquisition::Job::AirMissions(
                                frame.extended.take_air_missions_request(
                                    &country_snapshot,
                                    shared.base,
                                    state,
                                    // SAFETY: only this callback takes and returns the store.
                                    unsafe { (*shared.mission_store.get()).take() },
                                )
                            )));
                        }
                        map_done = true;
                    }
                    EarlyStep::ChildFronts => {
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        out_of_line(|| {
                            let (mut orders_request, _, child_fronts_failure) = completed!(
                                child_fronts_pending.take().expect("child fronts producer"),
                                ChildFronts
                            );
                            // ChildFronts may fail, unwind or be cancelled without
                            // invalidating the independently completed OOB copy. Always
                            // return it for decode.
                            *naval_payload = orders_request.group_source.take();
                            // Only the child fronts' operands are this job's; the census
                            // and plans the Orders job completed stand.
                            if child_fronts_failure.is_some() {
                                orders_request.plans.fail_child_fronts();
                            }
                            frame.extended.restore_orders_request(
                                orders_request,
                                true,
                                &mut frame.map.orders,
                            );
                        });
                        // Written after the map-tick copy below, which covers this index.
                        orders_ns = shared.ns(frame.map.orders.ticks as i64);
                        child_done = true;
                    }
                    EarlyStep::ArmyIcons => {
                        if let Some(pending) = army_icons_pending.take() {
                            let (request, _, failure) = completed!(pending, ArmyIcons);
                            frame
                                .extended
                                .restore_army_icons_request(request, failure.is_none());
                        }
                        icons_done = true;
                    }
                    EarlyStep::Missions => {
                        // The scores and rails join before the air pack returns: they
                        // install in it below. No dispatch reads this join alone.
                        let (scores_rails_request, _, scores_rails_failure) =
                            completed!(scores_rails_pending, ScoresRails);
                        // The Late job keeps the registrations and their joins; the
                        // history parts need the combats' regions.
                        let (air_missions_request, _, air_missions_failure) = completed!(
                            air_missions_pending.take().expect("air missions producer"),
                            AirMissions
                        );
                        if let Some(store) = frame.extended.restore_air_missions_request(
                            air_missions_request,
                            air_missions_failure.is_none(),
                        ) {
                            let bytes = store.allocated_bytes() as u64;
                            let previous =
                                shared.mission_store_bytes.swap(bytes, Ordering::Relaxed);
                            shared
                                .pool_bytes
                                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                            // SAFETY: the AirMissions job has joined; no worker retains this store.
                            unsafe { *shared.mission_store.get() = Some(store) };
                        }
                        install_air_verdicts(frame, air_stats_raw, air_bases_raw);
                        if let Some(pending) = air_supply_pending.take() {
                            let (request, _, failure) = completed!(pending, AirSupply);
                            frame
                                .extended
                                .restore_air_supply_request(request, failure.is_none());
                        }
                        // The scores and rails install in the packs the Late job loans,
                        // after the map sample and with the air pack back from the
                        // AirMissions job.
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        out_of_line(|| {
                            let (air_scores, rail_store) =
                                frame.extended.restore_scores_rails_request(
                                    scores_rails_request,
                                    scores_rails_failure.is_none(),
                                );
                            let bytes = air_scores.allocated_bytes() as u64;
                            let previous =
                                shared.raid_air_score_bytes.swap(bytes, Ordering::Relaxed);
                            shared
                                .pool_bytes
                                .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                            // SAFETY: the ScoresRails job has joined; no worker retains
                            // these stores.
                            unsafe { *shared.raid_air_scores.get() = Some(air_scores) };
                            if let Some(store) = rail_store {
                                let bytes = store.allocated_bytes() as u64;
                                let previous =
                                    shared.rail_store_bytes.swap(bytes, Ordering::Relaxed);
                                shared
                                    .pool_bytes
                                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                                // SAFETY: as above.
                                unsafe { *shared.rail_store.get() = Some(store) };
                            }
                        });
                        missions_done = true;
                    }
                    EarlyStep::FinalSources => {
                        final_sources_publish = final_sources_published(
                            &batch,
                            [
                                intel_residual_pending.as_ref(),
                                capitulation_pending.as_ref(),
                                session_pending.as_ref(),
                                relations_pending.as_ref(),
                            ],
                        )
                        .expect("successful final input sources");
                        join_final_sources!();
                        final_sources_done = true;
                    }
                    EarlyStep::Late => {
                        // Admit Late and its histories before FinalInputs, without
                        // waiting for their bodies or any assembly-only join.
                        late_pending = Some(queue!(acquisition::Job::Late(
                            frame.extended.take_late_request(
                                &country_snapshot,
                                &mut frame.map.orders,
                                naval_report.raw_ok || naval_failure.is_some(),
                                shared.base,
                                state,
                                clock.hour,
                                force_full,
                            )
                        )));
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
                    }
                    EarlyStep::FinalInputs => {
                        let dispatched = out_of_line(|| {
                            let (dispatched, request) = final_inputs_dispatch.take();
                            if let Some(request) = request {
                                frame.extended.return_final_inputs_request(request);
                            }
                            dispatched
                        });
                        if !final_sources_done {
                            join_final_sources!();
                        }
                        for part in 1..CHARACTER_PARTS {
                            join_characters!(part);
                        }
                        if let Some(pending) = dispatched {
                            let (ticket, parked) = batch.adopt_presentation_ticket(pending);
                            final_inputs_pending = Some(if parked {
                                PendingAcquisition::Rejected(ticket)
                            } else {
                                PendingAcquisition::Accepted(ticket)
                            });
                            continue;
                        }
                        // All producers have published; no callback fallback may repeat
                        // an attempted read. Settle residual Intel before lending its pack.
                        frame.extended.settle_intel_residual(
                            &cached,
                            extended::intel::CaptureContext {
                                base: shared.base,
                                game_state: state,
                                hour: clock.hour,
                                force_full,
                            },
                        );
                        // Out of line: the job-sized request stays off the hourly body's frame.
                        final_inputs_pending = Some(out_of_line(|| {
                            queue!(acquisition::Job::FinalInputs(
                                frame.extended.take_final_inputs_request(
                                    &country_snapshot,
                                    shared.base,
                                    state,
                                    clock.hour,
                                    force_full,
                                )
                            ))
                        }));
                    }
                    EarlyStep::Orders => {
                        let (orders, parts) = order_joins.take().expect("ready order producers");
                        stitch_pending = Some(out_of_line(|| {
                            // SAFETY: the callback still owns the live boundary;
                            // every part published before this slot-order merge.
                            let orders =
                                unsafe { collect_orders(shared, &mut batch, orders, parts) };
                            queue!(acquisition::Job::OrdersStitch(orders))
                        }));
                    }
                }
            }
            segment!(1);
            let late_pending = late_pending.expect("late dispatch");
            let final_pending = final_inputs_pending.expect("final input dispatch");
            let fronts_pending = fronts_recorded.then(|| fronts_pending.expect("fronts dispatch"));
            let movement_pending = movement_pending.expect("movement dispatch");
            let speed_terms_pending =
                status_recorded.then(|| speed_terms_pending.expect("speed terms dispatch"));
            let intel_pending = intel_recorded.then(|| intel_pending.expect("intel dispatch"));
            let province_unit_order_pending =
                province_unit_order_pending.expect("province order dispatch");
            let ordinary_pending = ordinary_pending.expect("ordinary dispatch");
            let provinces_pending = provinces_pending.expect("provinces dispatch");
            let states_pending = states_pending.expect("states dispatch");
            let weather_pending = weather_pending.expect("weather dispatch");
            let wars_pending = wars_pending.expect("wars dispatch");
            let character_inputs_pending =
                character_inputs_pending.expect("character inputs dispatch");
            // Out of line: the job-sized request stays off the hourly body's frame.
            let weather_failure = out_of_line(|| {
                let (mut request, report, failure) = completed!(weather_pending, Weather);
                let failure = failure
                    .or((!report.raw_ok).then_some("Weather owned acquisition could not complete"));
                if let Some(mut carry) = request.payload.carry.take() {
                    if failure.is_some() || request.payload.staging_failed() {
                        carry.forget();
                    }
                    // SAFETY: game thread; the Weather job has joined.
                    unsafe { *shared.weather_carry.get() = Some(carry) };
                }
                weather_report = report;
                frame.extended.weather_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    *weather_payload = Some(request.payload);
                } else {
                    decode::recycle_weather_job(request.payload);
                }
                failure
            });

            let mut lexical_failure = None;
            if let Some(pending) = lexical_pending {
                let (request, report, failure) = completed!(pending, Lexical);
                let failure = failure
                    .or((!report.raw_ok).then_some("Lexical owned acquisition could not complete"));
                lexical_report = report;
                lexical_failure = failure;
                frame.extended.lexical_pilot().raw_mode = report.raw_ok || failure.is_some();
                if report.raw_ok && failure.is_none() {
                    *lexical_payload = Some(request.payload);
                } else {
                    decode::recycle_lexical_job(request.payload);
                }
            }
            let begin = qpc();
            let (mut provinces_request, provinces_report, provinces_failure) =
                completed!(provinces_pending, Provinces);
            if let Some(mut store) = provinces_request.payload.area_store.take() {
                if provinces_failure.is_some() {
                    store.forget();
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared.area_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Provinces job has joined; no worker retains this store.
                unsafe { *shared.area_store.get() = Some(store) };
            }
            if let Some(mut store) = provinces_request.payload.province_store.take() {
                if provinces_failure.is_some() {
                    store.forget();
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared.province_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Provinces job has joined; no worker retains this store.
                unsafe { *shared.province_store.get() = Some(store) };
            }
            provinces_request.restore(&mut frame.provinces);
            frame.provinces_result = provinces_report.result;
            frame.table_ns[4] = shared.ns(qpc() - begin);
            let (mut states_request, states_report, states_failure) =
                completed!(states_pending, States);
            if let Some(mut store) = states_request.store.take() {
                if states_failure.is_some() {
                    store.forget();
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared.state_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the States job has joined; no worker retains this store.
                unsafe { *shared.state_store.get() = Some(store) };
            }
            if let Some(mut store) = states_request.occupation_store.take() {
                if states_failure.is_some() {
                    store.forget();
                }
                let bytes = store.bytes() as u64;
                let previous = shared.occupation_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the States worker has joined; frame snapshots contain no store borrow.
                unsafe { *shared.occupation_store.get() = Some(store) };
            }
            let states_report =
                frame.restore_states_request(states_request, states_report, states_failure);
            frame.table_ns[5] = shared.ns(states_report.ticks);
            // A completed worker needs its own supplying-read witness even
            // when its duration rounds below the clock's resolution.
            frame.table_ns[TableId::StateResistance.index()] = shared
                .ns(states_report.occupation_ticks)
                .max(u64::from(states_failure.is_none()));
            let (mut wars_request, wars_report, wars_failure) = completed!(wars_pending, Wars);
            if let Some(mut store) = wars_request.store.take() {
                if wars_failure.is_some() {
                    store.forget();
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared.war_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Wars job has joined; no worker retains this store.
                unsafe { *shared.war_store.get() = Some(store) };
            }
            let wars_report = frame.restore_wars_request(wars_request, wars_report, wars_failure);
            frame.table_ns[7] = shared.ns(wars_report.table_ticks);
            let (mut factions_request, mut factions_report, factions_failure) =
                completed!(factions_pending, Factions);
            if let Some(reason) = factions_failure {
                factions_report = acquisition_factions::Report::failed(reason);
            }
            if let Some(mut store) = factions_request.goal_store.take() {
                if let Some(reason) = factions_failure {
                    store.invalidate();
                    factions_request.goals.failure = Some(reason);
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared
                    .faction_goal_store_bytes
                    .swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Factions job has joined and returned exclusive ownership.
                unsafe { *shared.faction_goal_store.get() = Some(store) };
            }
            if let Some(mut store) = factions_request.companion_store.take() {
                if let Some(reason) = factions_failure {
                    store.invalidate();
                    factions_request.companions.refuse(reason);
                }
                let bytes = store.allocated_bytes() as u64;
                let previous = shared
                    .faction_companion_store_bytes
                    .swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Factions job joined and returned exclusive ownership.
                unsafe { *shared.faction_companion_store.get() = Some(store) };
            }
            frame.faction_companions = std::mem::take(&mut factions_request.companions);
            frame.table_ns[TableId::FactionRules.index()] =
                shared.ns(factions_report.companion_ticks as i64);
            frame.faction_goals = std::mem::take(&mut factions_request.goals);
            // Both tables share one acquisition; charge its work once.
            frame.table_ns[TableId::FactionGoalStatus.index()] = 0;
            frame.table_ns[TableId::FactionGoalEntries.index()] =
                shared.ns(factions_report.goal_ticks as i64);
            (frame.faction_names_len, frame.faction_names_required) = factions_request.restore(
                &mut frame.factions,
                &mut frame.faction_names,
                &mut frame.faction_members,
            );
            frame.factions_result = factions_report.factions.result;
            frame.table_ns[8] = shared.ns(factions_report.ticks[0] as i64);
            frame.faction_members_result = factions_report.members.result;
            frame.table_ns[9] = shared.ns(factions_report.ticks[1] as i64);
            // Readiness moved dispatches and releases into this span. They are
            // coordination costs, not inline walks, and have their own counters.
            inline_walk_ns = shared
                .ns(qpc() - inline_walk_begin)
                .saturating_sub(
                    shared
                        .acquisition_wait_ns
                        .load(Ordering::Relaxed)
                        .saturating_sub(inline_wait_begin),
                )
                .saturating_sub(
                    shared
                        .acquisition_dispatch_ns
                        .load(Ordering::Relaxed)
                        .saturating_sub(inline_dispatch_begin),
                )
                .saturating_sub(
                    shared
                        .game_thread
                        .release_ns
                        .load(Ordering::Relaxed)
                        .saturating_sub(inline_release_begin),
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
                let (request, report) = collected!(pending, SupplyTrains);
                if let Some(store) = frame
                    .extended
                    .restore_supply_trains_request(request, report)
                {
                    // SAFETY: only this callback takes and returns the store.
                    unsafe { return_slice_store(shared, crate::slicing::SUPPLY_TRAINS, store) };
                }
            }
            for pending in character_portraits_pending.into_iter().flatten() {
                let (request, report) = collected!(pending, CharacterPortraits);
                frame
                    .extended
                    .restore_character_portraits_request(request, report);
            }
            let (province_order_request, province_order_report) =
                collected!(province_unit_order_pending, ProvinceUnitOrder);
            if let Some(pending) = model_inputs_pending {
                let (request, _, failure) = completed!(pending, DivisionModelInputs);
                if let Some(training) = frame
                    .extended
                    .restore_division_model_inputs_request(request, failure.is_none())
                {
                    shared.model_inputs.record(training);
                }
            }
            let province_unit_store = frame
                .extended
                .restore_province_unit_order_request(province_order_request, province_order_report);
            if let Some(store) = province_unit_store {
                let bytes = store.allocated_bytes() as u64;
                let previous = shared
                    .province_unit_store_bytes
                    .swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the ProvinceUnitOrder job has joined; no worker
                // retains this store, and only this callback returns it.
                unsafe { *shared.province_unit_store.get() = Some(store) };
            }
            // The template dictionary's job: take its ticks (no read) and fold
            // its model failures. Out of line: the job-sized request stays off
            // the hourly body's frame.
            out_of_line(|| {
                let (templates_request, _, templates_failure) =
                    completed!(templates_pending, Templates);
                frame
                    .extended
                    .restore_templates_request(templates_request, templates_failure);
            });
            frame
                .extended
                .take_preacquired_templates(shared.base, state, clock.hour, force_full);
            // The game thread never waits for the status thread's copy.
            let failures = frame.extended.template_model_failures();
            if failures.rows != 0 {
                match shared.template_models.try_lock() {
                    Ok(mut stats) => stats.fold(failures),
                    Err(_) => {
                        shared
                            .template_models_missed
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            let (slow_request, _, slow_failure) = completed!(slow_pending, Slow);
            let stores = frame
                .extended
                .restore_slow_request(slow_request, slow_failure.is_none());
            // SAFETY: only this callback takes and returns the stores.
            unsafe {
                if let Some(store) = stores.available {
                    *shared.slice_fingerprints[crate::slicing::AVAILABLE_ADVISORS].get() =
                        Some(store);
                }
            };
            if let Some(store) = stores.advisors {
                let bytes = store.allocated_bytes() as u64;
                let previous = shared.advisor_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Slow job has joined; no worker retains this store.
                unsafe { *shared.advisor_store.get() = Some(store) };
            }
            if let Some(store) = stores.local_subunits {
                let bytes = store.allocated_bytes() as u64;
                let previous = shared
                    .local_subunit_store_bytes
                    .swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the Slow job has joined; no worker retains this store.
                unsafe { *shared.local_subunit_store.get() = Some(store) };
            }
            if let Some(store) = stores.doctrines {
                let bytes = stores.doctrine_bytes;
                let previous = shared.doctrine_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the existing Slow job has joined and returned its loan.
                unsafe { *shared.doctrine_store.get() = Some(store) };
            }
            if let Some(pending) = war_overview_pending {
                let (war_overview_request, _, war_overview_failure) =
                    completed!(pending, WarOverview);
                frame.extended.restore_war_overview_request(
                    war_overview_request,
                    war_overview_failure.is_none(),
                );
            }
            if let Some(pending) = stability_pending {
                let (stability_request, _, stability_failure) = completed!(pending, Stability);
                let stability_store = frame
                    .extended
                    .restore_stability_request(stability_request, stability_failure.is_none());
                if let Some(store) = stability_store {
                    // SAFETY: only this callback takes and returns the store.
                    unsafe {
                        *shared.slice_fingerprints[crate::slicing::STABILITY].get() = Some(store)
                    };
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
            if let Some(pending) = raid_targets_pending {
                let (raid_targets_request, _, raid_targets_failure) =
                    completed!(pending, RaidTargets);
                frame.extended.restore_raid_targets_request(
                    raid_targets_request,
                    raid_targets_failure.is_none(),
                );
            }
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
                return_slice_store(shared, crate::slicing::TRADE_ROUTES, state_stores.routes);
            };
            let (mut gated_request, _, gated_failure) = completed!(gated_pending, Gated);
            let (projects_request, _, gated_projects_failure) =
                completed!(gated_projects_pending, GatedProjects);
            gated_request.restore_projects(projects_request);
            let project_store = frame.extended.restore_gated_request(
                gated_request,
                gated_failure.is_none() && gated_projects_failure.is_none(),
            );
            // SAFETY: the Gated job has joined; no worker retains this store.
            unsafe { return_slice_store(shared, crate::slicing::PROJECTS, project_store) };
            let (character_inputs_request, character_inputs_report) =
                collected!(character_inputs_pending, CharacterInputs);
            if let Some(store) = frame.extended.restore_character_inputs_request(
                character_inputs_request,
                character_inputs_report.ok(),
            ) {
                let bytes = store.allocated_bytes() as u64;
                let previous = shared
                    .character_role_store_bytes
                    .swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                // SAFETY: the CharacterInputs job has joined; no worker retains this store.
                unsafe { *shared.character_role_store.get() = Some(store) };
            }
            // Out of line: the job-sized request stays off the hourly body's frame.
            out_of_line(|| {
                let (governance_request, governance_report) =
                    collected!(governance_pending, Governance);
                let (technologies, focus, building_store, leader_store, party_store) = frame
                    .extended
                    .restore_governance_request(governance_request, governance_report);
                // SAFETY: only this callback takes and returns the stores.
                unsafe {
                    *shared.slice_fingerprints[crate::slicing::TECHNOLOGIES].get() =
                        Some(technologies);
                    *shared.slice_fingerprints[crate::slicing::FOCUS].get() = Some(focus);
                };
                if let Some(store) = building_store {
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.building_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the Governance job has joined; no worker retains this store.
                    unsafe { *shared.building_store.get() = Some(store) };
                }
                if let Some(store) = leader_store {
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.leader_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the Governance job has joined; no worker retains this store.
                    unsafe { *shared.leader_store.get() = Some(store) };
                }
                if let Some(store) = party_store {
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.party_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the Governance job has joined; no worker retains this store.
                    unsafe { *shared.party_store.get() = Some(store) };
                }
            });
            // The event feed seals once the Governance census has joined, so
            // the census markers of this hour publish in this hour's row. A
            // worker seals it: the seal may wait for the shared store.
            let event_feed_seal_pending = queue!(acquisition::Job::EventFeedSeal(
                frame.extended.take_event_feed_seal()
            ));
            // Research returns into the restored family and charges its costs there.
            if let Some(pending) = research_pending {
                let (research_request, research_report) = collected!(pending, Research);
                let fingerprints = frame
                    .extended
                    .restore_research_request(research_request, research_report);
                // SAFETY: only this callback takes and returns the store.
                unsafe {
                    *shared.slice_fingerprints[crate::slicing::RESEARCH].get() = Some(fingerprints)
                };
            } else {
                frame
                    .extended
                    .omit_research(shared.base, state, clock.hour, force_full);
            }
            // No later census can repair the detail parts' first result.
            // Join the last-dispatched parts first: any still queued then run
            // inline here instead of parking. Part 0, the family's own capture,
            // is restored first.
            // Out of line: the job-sized request stays off the hourly body's frame.
            out_of_line(|| {
                let mut ship_detail_completed: [Option<(
                    acquisition::ShipModelDetailsRequest,
                    Result<extended::ship_models::CaptureReport, acquisition::Failure>,
                )>;
                    extended::ship_models::MAX_PARTS] =
                    [const { None }; extended::ship_models::MAX_PARTS];
                for (pending, completed) in ship_detail_pending
                    .into_iter()
                    .zip(ship_detail_completed.iter_mut())
                    .rev()
                {
                    if let Some(pending) = pending {
                        *completed = Some(collected!(pending, ShipModelDetails));
                    }
                }
                for completed in &mut ship_detail_completed {
                    if let Some((request, report)) = completed.take() {
                        frame.extended.restore_ship_model_part(request, report);
                    }
                }
                frame.extended.finish_ship_model_parts();
            });
            // The Ordinary family joins together, after the map sample (whose
            // typed air fallback must not see its score pack) and before
            // capture_tables and the Late join's land certificate, which read
            // the packs it returns.
            let naval_missions = out_of_line(|| {
                let parts = ordinary_parts_pending.map(|pending| {
                    // Queued in the first burst or the slack burst.
                    let pending = pending.expect("every Ordinary part is queued");
                    let (request, _, failure) = completed!(pending, OrdinaryPart);
                    (request, failure)
                });
                let (mut ordinary_request, _, ordinary_failure) =
                    completed!(ordinary_pending, Ordinary);
                if let Some(mut store) = ordinary_request.counters.take() {
                    if ordinary_failure.is_some() {
                        store.forget();
                    }
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.counter_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the Ordinary job has joined; no worker retains this store.
                    unsafe { *shared.counter_store.get() = Some(store) };
                }
                frame
                    .extended
                    .restore_ordinary_request(ordinary_request, ordinary_failure, parts)
            });
            // SAFETY: every terminal Ordinary outcome returns its original
            // store after invalidating unpublished/failed sliced observations.
            unsafe {
                if let Some(store) = naval_missions.fingerprints {
                    *shared.slice_fingerprints[crate::slicing::NAVAL_MISSIONS].get() = Some(store);
                }
            }
            // The event feed seal joins before capture_tables, which reads
            // the frame's event feed. A seal that did not run refuses its
            // prepared hour with the job's failure.
            // Out of line: the job-sized request stays off the hourly body's frame.
            out_of_line(|| {
                // SAFETY: this thread still owns the live acquisition boundary.
                let (feed, report) = unsafe {
                    collect_acquisition(
                        shared,
                        &mut batch,
                        event_feed_seal_pending,
                        |job| {
                            let acquisition::Job::EventFeedSeal(feed) = job else {
                                unreachable!("event feed seal ticket type")
                            };
                            feed
                        },
                        |report| {
                            let acquisition::Report::EventFeedSeal = report else {
                                unreachable!("event feed seal report type")
                            };
                        },
                    )
                };
                frame
                    .extended
                    .restore_event_feed_seal(feed, report.err().map(acquisition_failure_reason));
            });
            // The daily production slot's independent walks still join last
            // before the assembly that takes their ticks.
            if let Some(pending) = production_daily_pending {
                let (mut request, _, failure) = completed!(pending, ProductionDaily);
                if let Some(pending) = industrial_organisations_pending {
                    let (mio, report, mio_failure) = completed!(pending, IndustrialOrganisations);
                    frame.extended.restore_industrial_organisations_request(
                        mio,
                        mio_failure.is_none(),
                        report.ticks,
                        &mut request.packs,
                    );
                }
                frame
                    .extended
                    .restore_production_daily_request(request, failure.is_none());
            }
            for pending in production_part_pending.into_iter().flatten() {
                let (request, report, failure) = completed!(pending, ProductionModifierPart);
                frame.extended.restore_production_modifier_part(
                    request,
                    failure.is_none(),
                    report.ticks,
                );
            }
            if production_daily_due {
                // SAFETY: only this callback reads and writes the split.
                unsafe {
                    let starts = &mut *shared.production_part_starts.get();
                    *starts = frame.extended.production_modifier_starts(*starts);
                }
            }
            out_of_line(|| {
                let pending = variants_pending
                    .take()
                    .expect("independent variant producer");
                let (request, report) = collected!(pending, Variants);
                frame.extended.restore_variants_request(request, report);
            });
            segment!(9);
            let assembly_begin = qpc();
            let mut ticks =
                frame
                    .extended
                    .capture_tables(&cached, shared.base, state, clock.hour, force_full);
            let mut assembly_ticks = qpc() - assembly_begin;
            segment!(10);
            let (late_request, _, late_failure) = completed!(late_pending, Late);
            frame.extended.restore_late_request(
                late_request,
                late_failure.is_none(),
                &mut frame.map.orders,
            );
            // The fronts join the plans the Late job returned.
            if let Some(pending) = fronts_pending {
                let (fronts_request, _, fronts_failure) = completed!(pending, Fronts);
                frame
                    .extended
                    .install_fronts(fronts_request.fronts, fronts_failure.is_none());
            }
            // Out of line: the job-sized request stays off the hourly body's frame.
            out_of_line(|| {
                // Only this original Presentation join may wait solely for preparation.
                // SAFETY: the callback retains the live boundary through every part.
                let (mut presentation_request, presentation_report) =
                    unsafe { presentation.finish(shared, &mut batch) };
                let (movement_request, movement_report) =
                    collected!(movement_pending, PresentationMovement);
                let presentation_report = presentation_request.restore_movement(
                    movement_request,
                    movement_report,
                    presentation_report,
                );
                let presentation_report = match speed_terms_pending {
                    Some(pending) => {
                        let (speed_terms_request, speed_terms_report) =
                            collected!(pending, PresentationSpeedTerms);
                        presentation_request.restore_speed_terms(
                            speed_terms_request,
                            speed_terms_report,
                            presentation_report,
                        )
                    }
                    None => presentation_report,
                };
                let army_stats = frame
                    .extended
                    .restore_presentation_request(presentation_request, presentation_report);
                if let Some(army_stats) = army_stats {
                    shared.return_army_stat_store(army_stats);
                }
            });
            frame.extended.install_order_transport();
            let assembly_begin = qpc();
            let border_ticks = frame.extended.capture_border_inputs(
                &cached,
                shared.base,
                state,
                (&frame.provinces.rows, frame.provinces_result),
                (&mut frame.countries, &mut frame.countries_result),
            );
            assembly_ticks += qpc() - assembly_begin;
            ticks[replay_data::rows::TableId::DivisionStatus.index()] += border_ticks[0];
            frame.table_ns[replay_data::rows::TableId::Countries.index()] +=
                shared.ns(border_ticks[1] as i64);
            segment!(11);
            for pending in air_histories_pending.into_iter().flatten() {
                let (request, _, failure) = completed!(pending, AirHistories);
                frame
                    .extended
                    .restore_air_histories_request(request, failure.is_none());
            }
            frame.extended.settle_air_histories();
            // Out of line: the job-sized request stays off the hourly body's frame.
            out_of_line(|| {
                let (final_request, final_ticks, final_failure) =
                    completed!(final_pending, FinalInputs);
                frame.extended.restore_final_inputs_request(
                    final_request,
                    final_ticks,
                    final_failure,
                );
            });
            let assembly_begin = qpc();
            frame.extended.capture_tables_late(
                &cached,
                shared.base,
                state,
                clock.hour,
                force_full,
                &mut frame.map.orders,
                &mut ticks,
            );
            assembly_ticks += qpc() - assembly_begin;
            segment!(12);
            // UnitIntel joins here, at its first reader: nothing before reads
            // its rows, report or failure, and its worker finished long ago.
            // Out of line: the job-sized request stays off the hourly body's frame.
            let intel_failure = match intel_pending {
                None => Some(extended::NOT_RECORDED),
                Some(intel_pending) => out_of_line(|| {
                    let (request, report, failure) = completed!(intel_pending, Intel);
                    let failure = failure
                        .or((!report.raw_ok)
                            .then_some("Intel owned acquisition could not complete"));
                    intel_report = report;
                    if report.raw_ok && failure.is_none() {
                        *intel_payload = Some(request.payload);
                    } else {
                        decode::recycle_intel_job(request.payload);
                    }

                    // Release the shared seed leases before the Census can be
                    // cleared on the decode worker during payload retirement.
                    drop(request.inputs);
                    failure
                }),
            };
            // AirBases joins here, at its first reader: nothing before reads
            // its payload, report, failure or registration store, and its
            // verdicts were set before the Gated dispatch.
            // Out of line: the job-sized request stays off the hourly body's frame.
            let air_bases_failure = out_of_line(|| {
                let (mut request, report, failure) = completed!(air_bases_pending, AirBases);
                let failure = air_bases_join_failure(failure, report.raw_ok);
                if let Some(mut store) = request.payload.registration_store.take() {
                    if failure.is_some() {
                        store.forget();
                    }
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.air_base_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the AirBases job has joined; no worker retains this store.
                    unsafe { *shared.air_base_store.get() = Some(store) };
                }
                air_bases_report = report;
                if report.raw_ok && failure.is_none() {
                    *air_bases_payload = Some(request.payload);
                } else {
                    decode::recycle_air_bases_job(request.payload);
                }
                failure
            });
            shared.record_presentation(frame.extended.presentation_phase_ticks());
            refuse_world_rest(
                &mut frame.extended,
                world_active,
                &world_report,
                world_failure,
                &mut ticks,
            );
            refuse_economy_rest(
                frame.extended.economy_rest_pilot(),
                &economy_rest_report,
                economy_failure,
                &mut ticks,
            );
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
                frame.extended.install_reserved_land_certificate();
            }
            if let Some(reason) = divisions_failure {
                fail_divisions_capture(frame, reason, divisions_ctx.details_active);
            }
            // Restore the final independent request before releasing the lease.
            segment!(13);
            let ships = replay_data::rows::TableId::ShipModels.index();
            let assembly_begin = qpc();
            ticks[ships] = frame.extended.capture_ship_models(&cached, shared.base);
            assembly_ticks += qpc() - assembly_begin;
            shared
                .game_thread
                .table_assembly_ns
                .fetch_add(shared.ns(assembly_ticks), Ordering::Relaxed);
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
            for table in extended::PACK_TABLES {
                if table == TableId::StateResistance {
                    continue;
                }
                let index = table.index();
                let walk = replay_data::cadence::should_walk(table, clock.hour, force_full);
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
            if air_wing_report.raw_ok {
                let index = TableId::AirWings.index();
                frame.table_ns[index] = frame.table_ns[index]
                    .saturating_add(shared.ns(air_wing_report.transport_ticks as i64));
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
                    if *table == TableId::StateResistance {
                        continue;
                    }
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
                    engine_native::Memory::authenticated_countries(&cached, shared.base, state),
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
            shared.sample_roster(&memory, frame.clock.as_ref().map(Clock::borrowed));
        }
        frame.table_ns[0] = shared.ns(clock_ticks);
        crate::read_accounting::snapshot_into(&mut frame.table_reads);
        if frame.world_generation != crate::world_lifetime::generation() {
            frame.extended.discard_event_feed();
            lost_observer_interval();
            frame.set_clock(None);
            shared.force_next.store(true, Ordering::Release);
        }
        // Keep the producer's loss witnesses until ownership is published.
        // A contained panic before enqueue must not consume earlier gaps.
        unsafe {
            std::ptr::swap(&mut frame.dropped_before, shared.pending.get());
        }
        // Restamped by the decode worker with this frame's own growth measured.
        frame.pool_bytes = shared.pool_total();
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
        let diagnostics = crate::acquisition_diagnostics::take(
            engine_allocation::growths().saturating_sub(growths_before),
        );
        #[cfg(feature = "callback_profile")]
        crate::callback_profile::retain_diagnostics(diagnostics);
        let game_elapsed = (qpc() - dispatched).max(1);
        let setup_ns = shared.ns(ts_setup - dispatched);
        let parallel_ns = shared.ns(ts_parallel - ts_setup);
        let residual_ns = shared.ns(dispatched + game_elapsed - ts_parallel);
        // Disjoint block phases: setup+snapshot, parallel (dispatch/inline/joins),
        // then residual work and enqueue.
        shared.phase_setup_ns.fetch_add(setup_ns, Ordering::Relaxed);
        shared
            .phase_parallel_ns
            .fetch_add(parallel_ns, Ordering::Relaxed);
        shared
            .phase_residual_ns
            .fetch_add(residual_ns, Ordering::Relaxed);
        let joins = take_hour_joins();
        frame.hour_cost = if frame.clock.is_some() {
            HourCost {
                entry_ns: shared.ns(entry_ticks),
                buffer_wait_ns,
                setup_ns,
                parallel_ns,
                residual_ns,
                segments_ns,
                slowest_join: joins.slowest,
                collect_stalls_over_1ms: joins.collect_stalls_over_1ms,
                growths: diagnostics
                    .callback_growths
                    .saturating_add(diagnostics.jobs.worker_growths),
                diagnostics,
                idle: crate::acquisition::idle_diagnostics::take(),
                variants: frame.clock.as_ref().and_then(|clock| {
                    crate::design_changes::profile::latest(clock.hour, dispatched)
                }),
                #[cfg(feature = "callback_profile")]
                join_timeline: HOUR_JOINS.get().and_then(|tally| tally.profile_span),
                first_since_attach,
                continuous,
                force_full,
                recovery,
                refresh,
            }
        } else {
            HourCost::default()
        };
        if let Some(diagnostics) = attach_diagnostics {
            diagnostics.finish(frame, shared.ns(game_elapsed));
        }
        shared
            .phase_inline_walk_ns
            .fetch_add(inline_walk_ns, Ordering::Relaxed);
        shared.phase_join_ns.fetch_add(join_ns, Ordering::Relaxed);
        shared.phase_calls.fetch_add(1, Ordering::Relaxed);
        shared.game_thread.hours.fetch_add(1, Ordering::Relaxed);
        frame.capture_seq = decode::next_seq();
        // Every field is named: a new one cannot keep a previous hour's value.
        let decode::DecodeJob {
            queued_qpc: _,
            index: job_index,
            seq,
            hour,
            relations_refresh,
            characters_refresh,
            projects_refresh,
            lexical_refresh,
            divisions_refresh,
            air_wing_refresh,
            air_stats_refresh,
            naval_refresh,
            players_refresh,
            session_refresh,
            production_rest_refresh,
            construction_rest_refresh,
            economy_rest_refresh,
            air_bases_refresh,
            supremacy_refresh,
            sunk_refresh,
            radar_refresh,
            intel_refresh,
            weather_refresh,
            capitulation_refresh,
            force_full,
            game_elapsed: job_elapsed,
            relations_payload: _,
            characters_payload: _,
            projects_payload: _,
            lexical_payload: _,
            divisions_payload: _,
            air_wing_payload: _,
            air_stats_payload: _,
            naval_payload: _,
            players_payload: _,
            session_payload: _,
            production_rest_payload: _,
            construction_rest_payload: _,
            economy_rest_payload: _,
            air_bases_payload: _,
            supremacy_payload: _,
            sunk_payload: _,
            radar_payload: _,
            intel_payload: _,
            weather_payload: _,
            capitulation_payload: _,
        } = decode_slot.job();
        *job_index = index;
        *seq = frame.capture_seq;
        *hour = decode_hour;
        *relations_refresh = relations_report.raw_ok;
        *characters_refresh = characters_report.map(|report| report.raw_ok);
        *projects_refresh = projects_report.raw_ok;
        *lexical_refresh = lexical_report.raw_ok;
        *divisions_refresh = divisions_report.raw_ok;
        *air_wing_refresh = air_wing_report.raw_ok;
        *air_stats_refresh = air_stats_report.raw_ok;
        *naval_refresh = naval_report.raw_ok;
        *players_refresh = players_report.raw_ok;
        *session_refresh = session_report.raw_ok;
        *production_rest_refresh = production_rest_report.raw_ok;
        *construction_rest_refresh = construction_rest_report.raw_ok;
        *economy_rest_refresh = economy_rest_report.raw_ok;
        *air_bases_refresh = air_bases_report.raw_ok;
        *supremacy_refresh = supremacy_report.raw_ok;
        *sunk_refresh = sunk_report.raw_ok;
        *radar_refresh = radar_report.raw_ok;
        *intel_refresh = intel_report.raw_ok;
        *weather_refresh = weather_report.raw_ok;
        *capitulation_refresh = capitulation_report.raw_ok;
        *force_full = decode_force_full;
        *job_elapsed = game_elapsed;
        if decode::enqueue_reserved(shared, decode_slot) {
            unsafe { (&mut *shared.pending.get()).clear() }
        } else {
            // Existing whole-frame drop plus gap path, unchanged
            // counters and no new loss reasons.
            drop_owned_frame(shared, frame, index);
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

/// The AirBases join's failure: the job's own, else an incomplete raw
/// acquisition. Every join therefore has a raw payload or a failure, so the
/// registrations and bases verdicts are true before the join.
fn air_bases_join_failure(failure: Option<&'static str>, raw_ok: bool) -> Option<&'static str> {
    failure.or((!raw_ok).then_some("AirBases owned acquisition could not complete"))
}

/// A raw-copy join's failure: the job's own, else an incomplete raw
/// acquisition (`incomplete`). Every join therefore has a raw payload or a
/// failure, so the verdict the Gated packs read is true before the join.
fn raw_copy_join_failure(
    failure: Option<&'static str>,
    raw_ok: bool,
    incomplete: &'static str,
) -> Option<&'static str> {
    failure.or((!raw_ok).then_some(incomplete))
}

/// Install the air pack's statistics and base verdicts of this hour (`None`:
/// unchanged). The AirMissions job owns the pack between their joins and its
/// own; neither verdict is read before the Late job.
fn install_air_verdicts(frame: &mut Captured, stats: Option<bool>, bases: Option<bool>) {
    if let Some(raw) = stats {
        frame.extended.air_stats_pilot().stats_raw_mode = raw;
    }
    if let Some(raw) = bases {
        frame.extended.air_bases_pilot().bases_raw_mode = raw;
    }
}

/// Runs `work` in its own frame: values it holds, such as a join's job-sized
/// request, never occupy the hourly body's frame.
#[inline(never)]
fn out_of_line<R>(work: impl FnOnce() -> R) -> R {
    work()
}

/// One submitted request's join. A rejected request is parked in the
/// acquisition pool (`Batch::park`) rather than held here, so the hourly body
/// keeps a ticket per pending join, not a job-sized request.
#[derive(Clone, Copy)]
enum PendingAcquisition {
    Accepted(acquisition::Ticket),
    Rejected(acquisition::Ticket),
}

/// Builds one request and submits it out of line: the job-sized request lives
/// only in this frame, never in the hourly body's.
///
/// # Safety
/// As `submit_acquisition`.
#[inline(never)]
unsafe fn queue_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    request: impl FnOnce() -> acquisition::Job,
) -> PendingAcquisition {
    let mut retained = Some(request());
    // SAFETY: forwarded from the caller.
    unsafe { submit_retained(shared, batch, &mut retained) }
}

/// Caller retains the game boundary until this guard and all returned jobs finish.
unsafe fn submit_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    request: acquisition::Job,
) -> PendingAcquisition {
    // SAFETY: forwarded from the caller.
    unsafe { submit_retained(shared, batch, &mut Some(request)) }
}

unsafe fn submit_retained(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    retained: &mut Option<acquisition::Job>,
) -> PendingAcquisition {
    let begin = qpc();
    // SAFETY: the caller's batch owns this callback's complete live lease.
    let pending = match unsafe { batch.submit(retained) } {
        Ok(ticket) => PendingAcquisition::Accepted(ticket),
        // MAX_JOBS bounds accepted and retained requests together.
        Err(_) => PendingAcquisition::Rejected(
            batch
                .park(retained)
                .expect("acquisition retains at most MAX_JOBS requests"),
        ),
    };
    shared
        .acquisition_dispatch_ns
        .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
    shared
        .game_thread
        .dispatched_jobs
        .fetch_add(1, Ordering::Relaxed);
    pending
}

/// Collects one join out of line: the completion, with its job-sized request
/// and read counts, lives only in this frame. The hourly body receives the
/// variant's request and report.
///
/// # Safety
/// As `finish_acquisition_into`.
#[inline(never)]
unsafe fn collect_acquisition<T, R>(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    pending: PendingAcquisition,
    request: impl FnOnce(&acquisition::Job) -> &T,
    report: impl FnOnce(acquisition::Report) -> R,
) -> (T, Result<R, acquisition::Failure>) {
    // Never dropped: the job's variant is its only owned value and moves out
    // once below, straight from this storage (a panic leaks, never frees).
    let mut completed = std::mem::ManuallyDrop::new(None);
    // SAFETY: forwarded from the caller.
    unsafe { finish_acquisition_into(shared, batch, pending, &mut completed) };
    let completed = completed.as_ref().expect("every join yields a completion");
    // SAFETY: `completed` is never dropped, so the variant is read exactly once.
    let variant = unsafe { std::ptr::read(request(&completed.request)) };
    (variant, completed.report.map(report))
}

#[cfg(test)]
unsafe fn finish_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    pending: PendingAcquisition,
) -> acquisition::Completion<acquisition::Job, acquisition::Report> {
    let mut completed = None;
    // SAFETY: forwarded from the caller.
    unsafe { finish_acquisition_into(shared, batch, pending, &mut completed) };
    completed.expect("every join yields a completion")
}

/// Accepted work is recovered once; only rejected submissions may execute
/// inline. The completion moves into the caller's empty `out`.
unsafe fn finish_acquisition_into(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    pending: PendingAcquisition,
    out: &mut Option<acquisition::Completion<acquisition::Job, acquisition::Report>>,
) {
    match pending {
        PendingAcquisition::Accepted(ticket) => {
            let begin = qpc();
            assert!(batch.finish_into(ticket, out), "submitted acquisition job");
            let result = out.as_ref().expect("submitted acquisition job");
            let end = qpc();
            use crate::acquisition::idle_diagnostics::{self, Family};
            match (&result.request, &result.report) {
                (acquisition::Job::Governance(_), Ok(acquisition::Report::Governance(report))) => {
                    idle_diagnostics::update(|hour| {
                        hour.governance = Some(Family::new(
                            extended::governance::TABLES
                                .into_iter()
                                .zip(report.governance.ticks),
                        ));
                        hour.buildings_ticks = Some(report.buildings);
                    });
                }
                (
                    acquisition::Job::GatedProjects(request),
                    Ok(acquisition::Report::GatedProjects(_)),
                ) => {
                    idle_diagnostics::update(|hour| {
                        hour.projects = Some(Family::new(
                            extended::projects::TABLES.into_iter().zip(request.ticks),
                        ))
                    });
                }
                _ => {}
            }
            let wait = shared.ns(end - begin);
            let stall_ns = shared.join_costs[result.request.role() as usize].record(
                &result.timeline,
                begin,
                |ticks| shared.ns(ticks),
            );
            record_hour_join(
                result.request.role() as usize,
                wait,
                stall_ns,
                #[cfg(feature = "callback_profile")]
                result.profile,
                #[cfg(feature = "callback_profile")]
                Some(crate::acquisition_diagnostics::JoinSpan {
                    job: crate::acquisition_diagnostics::Job {
                        identity: result.request.diagnostic_identity(),
                        ticket,
                        timeline: result.timeline,
                    },
                    profile: result.profile,
                    begin_qpc: begin,
                    end_qpc: end,
                }),
            );
            #[cfg(feature = "callback_profile")]
            idle_diagnostics::update(|hour| match result.request.role() {
                acquisition::Role::Variants => {
                    hour.variants_cycles = result.profile.runner_thread_cycles
                }
                acquisition::Role::Governance => {
                    hour.governance_cycles = result.profile.runner_thread_cycles
                }
                acquisition::Role::GatedProjects => {
                    hour.projects_cycles = result.profile.runner_thread_cycles
                }
                _ => {}
            });
            shared
                .acquisition_wait_ns
                .fetch_add(wait, Ordering::Relaxed);
            shared.join_wait[result.request.role() as usize].fetch_add(wait, Ordering::Relaxed);
            shared.role_timeline[result.request.role() as usize].record(
                shared.acquisition_entry_qpc.load(Ordering::Relaxed),
                &result.timeline,
                begin,
                end,
                wait,
            );
        }
        // SAFETY: forwarded from the caller.
        PendingAcquisition::Rejected(ticket) => unsafe { finish_rejected(batch, ticket, out) },
    }
}

/// Early dependency barriers. Handlers install producer outputs and adopt
/// worker continuations; readiness never borrows a request or a live source.
#[derive(Clone, Copy, Debug)]
pub(crate) enum EarlyStep {
    Presentation,
    Roots,
    Inputs,
    Ships,
    Countries,
    Characters,
    Wings,
    Orders,
    Divisions,
    Verdicts,
    Stitch,
    Naval,
    Map,
    ChildFronts,
    ArmyIcons,
    Missions,
    Late,
    FinalSources,
    FinalInputs,
}
impl EarlyStep {
    pub(crate) const ALL: [Self; 19] = [
        Self::Presentation,
        Self::Roots,
        Self::Inputs,
        Self::Ships,
        Self::Countries,
        Self::Characters,
        Self::Wings,
        Self::Orders,
        Self::Divisions,
        Self::Verdicts,
        Self::Stitch,
        Self::Naval,
        Self::Map,
        Self::ChildFronts,
        Self::ArmyIcons,
        Self::Missions,
        Self::Late,
        Self::FinalSources,
        Self::FinalInputs,
    ];
}

fn pending_complete(batch: &acquisition::Batch<'_>, pending: &PendingAcquisition) -> bool {
    match pending {
        PendingAcquisition::Accepted(ticket) => batch.is_complete(*ticket),
        PendingAcquisition::Rejected(_) => true,
    }
}

/// Only successful final-source joins move ahead of the admission fence.
/// Failed, rejected, cancelled and incomplete captures keep ordered settlement.
fn final_sources_published(
    batch: &acquisition::Batch<'_>,
    producers: [Option<&PendingAcquisition>; 4],
) -> Option<i64> {
    producers
        .into_iter()
        .enumerate()
        .try_fold(0, |last, (index, pending)| {
            let Some(pending) = pending else {
                return matches!(index, 2 | 3).then_some(last);
            };
            let PendingAcquisition::Accepted(ticket) = pending else {
                return None;
            };
            let (Ok(report), at) = batch.completed_state(*ticket)? else {
                return None;
            };
            let complete = match (index, report) {
                (0, acquisition::Report::IntelResidual(_)) => true,
                (1, acquisition::Report::Capitulation(report)) => report.raw_ok,
                (2, acquisition::Report::Session(report)) => report.raw_ok,
                (3, acquisition::Report::Relations(report)) => report.raw_ok,
                _ => false,
            };
            complete.then_some(last.max(at))
        })
}

/// FinalInputs loans only returned packs and settled raw verdicts. Late/history
/// admission preserves the last-dispatch failure barrier, not a body dependency.
/// Optional producers skipped by cadence need no ticket; rejected ones are
/// published terminal outcomes and still return their original loans.
fn final_inputs_ready<'a>(
    batch: &acquisition::Batch<'_>,
    earlier_dispatches_done: bool,
    producers: impl IntoIterator<Item = Option<&'a PendingAcquisition>>,
) -> bool {
    earlier_dispatches_done
        && producers
            .into_iter()
            .flatten()
            .all(|pending| pending_complete(batch, pending))
}

/// Tickets only: pool-owned continuations retain preparation and its ranges.
/// Readiness adopts their tickets and preserves the callback fallback.
#[derive(Default)]
struct PresentationPending {
    prepare: Option<PendingAcquisition>,
    parts: [Option<PendingAcquisition>; extended::division_presentation::MAX_STATUS_PARTS],
    prepared: Option<acquisition::Ticket>,
    failure: Option<acquisition::Failure>,
    pending: Option<PendingAcquisition>,
    /// The request whose status side the capture mode leaves out, held to
    /// the final join.
    omitted: Option<acquisition::PresentationRequest>,
}

impl PresentationPending {
    fn waiting(&self) -> bool {
        self.prepare.is_some()
            || (self.prepared.is_some()
                && (self.failure.is_none() || self.parts.iter().any(Option::is_some)))
    }

    fn ready(&self, batch: &acquisition::Batch<'_>) -> bool {
        if let Some(prepare) = &self.prepare {
            pending_complete(batch, prepare)
        } else {
            self.prepared.is_some()
                && (self.failure.is_none() || self.parts.iter().any(Option::is_some))
                && self
                    .parts
                    .iter()
                    .flatten()
                    .all(|part| pending_complete(batch, part))
        }
    }

    /// Await an existing join, servicing published presentation edges meanwhile.
    /// A ready unrelated join wins over an unfinished preparation. Only `None`,
    /// at the original Presentation join, may wait for this chain alone.
    ///
    /// # Safety
    /// The callback retains its live acquisition boundary through the batch.
    unsafe fn wait_for(
        &mut self,
        shared: &Shared,
        batch: &mut acquisition::Batch<'_>,
        pending: Option<&PendingAcquisition>,
    ) {
        while self.waiting() {
            let begin = qpc();
            let advance = batch.wait_ready(|batch| {
                if self.ready(batch) {
                    return Some(true);
                }
                crate::acquisition_diagnostics::probe_condition();
                pending
                    .is_some_and(|pending| pending_complete(batch, pending))
                    .then_some(false)
            });
            shared
                .acquisition_wait_ns
                .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
            if !advance {
                break;
            }
            let _advance = EarlyStepTimer::new(
                shared,
                &shared.presentation.wait_advance,
                EarlyStep::Presentation,
            );
            // SAFETY: publication is proved above; the same boundary is held.
            unsafe { self.advance(shared, batch) };
        }
    }

    /// Collect only published producers, dispatching their consumer immediately.
    /// # Safety
    /// As `wait_for`; `ready` must hold.
    #[inline(never)]
    unsafe fn advance(&mut self, shared: &Shared, batch: &mut acquisition::Batch<'_>) {
        assert!(self.ready(batch), "presentation producer published");
        if let Some(pending) = self.prepare.take() {
            // SAFETY: forwarded from the caller.
            unsafe { self.dispatch_ranges(shared, batch, pending) };
        } else {
            // SAFETY: forwarded from the caller.
            unsafe { self.dispatch_stitch(shared, batch) };
        }
    }

    #[inline(never)]
    fn park(
        &mut self,
        batch: &mut acquisition::Batch<'_>,
        request: acquisition::PresentationRequest,
    ) {
        self.prepared = Some(
            batch
                .park(&mut Some(acquisition::Job::Presentation(request)))
                .expect("presentation preparation slot"),
        );
    }

    #[inline(never)]
    fn unpark(&mut self, batch: &mut acquisition::Batch<'_>) -> acquisition::PresentationRequest {
        let acquisition::Job::Presentation(request) = batch
            .unpark(
                self.prepared
                    .take()
                    .expect("presentation preparation parked"),
            )
            .expect("presentation preparation retained")
        else {
            unreachable!("presentation preparation type")
        };
        request
    }

    #[inline(never)]
    unsafe fn dispatch_ranges(
        &mut self,
        shared: &Shared,
        batch: &mut acquisition::Batch<'_>,
        pending: PendingAcquisition,
    ) {
        // SAFETY: this published preparation belongs to the callback's batch.
        let (mut request, report) = unsafe {
            collect_acquisition(
                shared,
                batch,
                pending,
                |job| {
                    let (acquisition::Job::PresentationPrepare(request)
                    | acquisition::Job::PresentationSettle(request)) = job
                    else {
                        unreachable!("presentation preparation type")
                    };
                    request
                },
                |report| {
                    let acquisition::Report::PresentationPrepare(report) = report else {
                        unreachable!("presentation preparation report")
                    };
                    report
                },
            )
        };
        self.failure = report.err();
        if request.dispatch.dispatched() {
            let count = request.dispatch.count;
            for index in 0..count {
                self.parts[index] =
                    batch
                        .adopt_presentation(&mut request, index)
                        .map(|(ticket, parked)| {
                            if parked {
                                PendingAcquisition::Rejected(ticket)
                            } else {
                                PendingAcquisition::Accepted(ticket)
                            }
                        });
            }
            shared
                .presentation
                .range_dispatch_ns
                .fetch_add(shared.ns(request.dispatch.ticks as i64), Ordering::Relaxed);
        } else if self.failure.is_none() {
            batch.hold();
            for (index, pending) in self
                .parts
                .iter_mut()
                .enumerate()
                .take(request.captured.status_part_count())
            {
                // SAFETY: the batch drains every range before releasing the boundary.
                *pending = Some(unsafe {
                    queue_acquisition(shared, batch, || {
                        let part = request.captured.take_status_part(index);
                        acquisition::Job::PresentationStatus(
                            acquisition::PresentationStatusRequest {
                                countries: Arc::clone(&request.countries),
                                context: request.context,
                                inputs: request.inputs.clone(),
                                preparation: matches!(
                                    &part,
                                    extended::division_presentation::StatusWork::Preparation { .. }
                                ),
                                part: Some(part),
                                stitch: None,
                            },
                        )
                    })
                });
            }
            let begin = qpc();
            let jobs = batch.release();
            shared.game_thread.release(jobs, shared.ns(qpc() - begin));
        }
        self.park(batch, request);
    }

    #[inline(never)]
    unsafe fn dispatch_stitch(&mut self, shared: &Shared, batch: &mut acquisition::Batch<'_>) {
        let mut request = self.unpark(batch);
        let worker_stitch = request.dispatch.continuation();
        for (index, pending) in self.parts.iter_mut().enumerate() {
            let Some(pending) = pending.take() else {
                continue;
            };
            // Separate frame: only the master request lives in this one.
            out_of_line(|| {
                // SAFETY: every range published before this ordered collection.
                let (mut part, report) = unsafe {
                    collect_acquisition(
                        shared,
                        batch,
                        pending,
                        |job| {
                            let acquisition::Job::PresentationStatus(request) = job else {
                                unreachable!("presentation status part type")
                            };
                            request
                        },
                        |report| {
                            let acquisition::Report::PresentationStatus(report) = report else {
                                unreachable!("presentation status part report")
                            };
                            report
                        },
                    )
                };
                if let Err(failure) = report {
                    self.failure = self.failure.or(Some(failure));
                }
                if let Some(stitch) = &worker_stitch {
                    if part.part.is_some() {
                        stitch.store_part(
                            index,
                            part.part.take(),
                            report.map(acquisition::Report::PresentationStatus),
                        );
                    }
                    return;
                }
                let mut work = part.part.take().expect("presentation range retained");
                if report.is_err() {
                    work.fail("division presentation part acquisition failed");
                }
                request
                    .captured
                    .restore_status_part(index, work, report.unwrap_or_default());
                // The preparation Arc lease drops before the stitch can mutate it.
            });
        }
        if let Some(stitch) = worker_stitch {
            let (pending, retained, failure) = stitch.take();
            self.failure = self.failure.or(failure);
            if let Some(pending) = pending {
                let (ticket, parked) = batch.adopt_presentation_ticket(pending);
                let pending = Some(if parked {
                    PendingAcquisition::Rejected(ticket)
                } else {
                    PendingAcquisition::Accepted(ticket)
                });
                if request.dispatch.final_ranges() {
                    self.pending = pending;
                } else {
                    self.prepare = pending;
                }
                return;
            }
            request = retained.expect("presentation stitch retained");
        }
        request
            .captured
            .restore_preparation_store(request.army_stats.as_deref_mut());
        if self.failure.is_none() {
            if request.captured.preparation_pending() {
                // The worker releases final ranges at its completed-admission
                // barrier, even while this callback installs unrelated results.
                batch.arm_presentation(&mut request);
                self.prepare = Some(unsafe {
                    submit_acquisition(shared, batch, acquisition::Job::PresentationSettle(request))
                });
            } else {
                // SAFETY: all shared leases were dropped above; the batch owns the stitch.
                self.pending = Some(unsafe {
                    submit_acquisition(shared, batch, acquisition::Job::Presentation(request))
                });
            }
        } else {
            // Retain the failed request for its original final join, without retry.
            self.park(batch, request);
        }
    }

    /// # Safety
    /// As `wait_for`; called only at the original Presentation join.
    #[inline(never)]
    unsafe fn finish(
        &mut self,
        shared: &Shared,
        batch: &mut acquisition::Batch<'_>,
    ) -> (
        acquisition::PresentationRequest,
        Result<extended::division_presentation::CaptureReport, acquisition::Failure>,
    ) {
        if let Some(mut request) = self.omitted.take() {
            let report = request.captured.omit_status(extended::NOT_RECORDED);
            return (request, Ok(report));
        }
        // SAFETY: forwarded from the caller.
        unsafe { self.wait_for(shared, batch, None) };
        if let Some(pending) = self.pending.take() {
            // SAFETY: as above.
            unsafe {
                collect_acquisition(
                    shared,
                    batch,
                    pending,
                    |job| {
                        let acquisition::Job::Presentation(request) = job else {
                            unreachable!("presentation stitch type")
                        };
                        request
                    },
                    |report| {
                        let acquisition::Report::Presentation(report) = report else {
                            unreachable!("presentation stitch report")
                        };
                        report
                    },
                )
            }
        } else {
            let mut request = self.unpark(batch);
            request
                .captured
                .restore_preparation_store(request.army_stats.as_deref_mut());
            (request, Err(self.failure.expect("presentation failed")))
        }
    }
}

/// Failure settlement follows the original Naval join, but admission was
/// decided before that join. Naval can itself stop the pool in between.
#[derive(Default)]
struct StitchFailureBarrier {
    admissible: Option<bool>,
}
impl StitchFailureBarrier {
    fn reach(&mut self, batch: &acquisition::Batch<'_>) {
        self.admissible.get_or_insert_with(|| !batch.stopped());
    }
    fn ready(
        &self,
        batch: &acquisition::Batch<'_>,
        stitch: Option<&PendingAcquisition>,
        naval: Option<&PendingAcquisition>,
        wings_done: bool,
    ) -> bool {
        self.admissible.is_some()
            && wings_done
            && stitch.is_some_and(|pending| pending_complete(batch, pending))
            && naval.is_some_and(|pending| pending_complete(batch, pending))
    }
}

/// Successful stitching releases independent fronts at publication. A
/// contained panic or failed/rejected stitch keeps the original settlement
/// barrier, so it cannot stop admission before the earlier dispatches.
fn stitch_can_advance(
    batch: &acquisition::Batch<'_>,
    pending: &PendingAcquisition,
    settle_failure: bool,
) -> bool {
    if !pending_complete(batch, pending) {
        return false;
    }
    settle_failure
        || matches!(pending,
            PendingAcquisition::Accepted(ticket)
                if matches!(batch.completed_report(*ticket),
                    Some(Ok(acquisition::Report::OrdersStitch(acquisition::StitchReport {
                        contained_panic: false,
                    }))))
        )
}

/// Whether the Orders job and every part have completed (a refused one is
/// complete: it never runs), so that joining them neither waits nor runs one
/// on the game thread.
fn orders_complete(
    batch: &acquisition::Batch<'_>,
    orders: &PendingAcquisition,
    parts: &[Option<PendingAcquisition>],
) -> bool {
    std::iter::once(orders)
        .chain(parts.iter().flatten())
        .all(|pending| pending_complete(batch, pending))
}

/// Join the Orders job and its parts into the census the stitch takes. A
/// failed Orders job fails the census; each part returns, and the first that
/// did not run fails the census with the reason the single job would have
/// had, unless the census already failed.
///
/// # Safety
/// The caller still owns the live acquisition boundary.
unsafe fn collect_orders(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    orders: PendingAcquisition,
    parts: [Option<PendingAcquisition>; crate::orders_parts::MAX_PARTS - 1],
) -> acquisition::OrdersRequest {
    // SAFETY: forwarded from the caller.
    let (mut orders_request, report) = unsafe {
        collect_acquisition(
            shared,
            batch,
            orders,
            |job| {
                let acquisition::Job::Orders(request) = job else {
                    unreachable!("acquisition ticket retains its submitted job type")
                };
                request
            },
            |report| {
                let acquisition::Report::Orders(report) = report else {
                    unreachable!("acquisition report retains its submitted job type")
                };
                report
            },
        )
    };
    if report.is_err() {
        orders_request.fail("army order acquisition failed");
    }
    for pending in parts.into_iter().flatten() {
        // SAFETY: as above.
        let (request, report) = unsafe {
            collect_acquisition(
                shared,
                batch,
                pending,
                |job| {
                    let acquisition::Job::OrdersPart(request) = job else {
                        unreachable!("acquisition ticket retains its submitted job type")
                    };
                    request
                },
                |report| {
                    let acquisition::Report::OrdersPart(report) = report else {
                        unreachable!("acquisition report retains its submitted job type")
                    };
                    report
                },
            )
        };
        let reason = request.failure(report.is_err());
        orders_request
            .orders
            .return_part(request.index, request.part);
        if let Some(reason) = reason
            && !orders_request.orders.failed()
        {
            orders_request.fail(reason);
        }
    }
    orders_request
}

/// The OrdersStitch job's outcome at its join, as if it had been submitted
/// where `stitch_admissible` was taken: after every job the census does not
/// gate. The stitch leaves earlier, once the Orders job and its parts are
/// joined, so neither direction of failure may differ from that placement:
/// - Its own panic, contained in its body, stops the pool only here: no job
///   dispatched before this join is cancelled or refused for it, as none
///   would have been had the stitch been submitted later. It is counted as
///   the runner counts a panic.
/// - A pool already stopped where it would have been submitted would have
///   refused it: a stitch that did not complete then takes the admission
///   reason a refused stitch takes, whether a stop cancelled it while queued
///   or it panicked. One that completed before the stop keeps its census,
///   which is never harsher.
/// - Otherwise a failed, cancelled or panicked stitch fails the census with
///   the acquisition reason, as a failure after its submission did.
///
/// A census an earlier job already failed keeps that reason.
fn settle_stitch(
    batch: &acquisition::Batch<'_>,
    orders: &mut acquisition::OrdersRequest,
    report: acquisition::StitchReport,
    failure: Option<&'static str>,
    stitch_admissible: bool,
) {
    if report.contained_panic && stitch_admissible {
        batch.stop_for_contained_panic(acquisition::Role::OrdersStitch);
    }
    if (failure.is_none() && !report.contained_panic) || orders.orders.failed() {
        return;
    }
    orders.fail(if stitch_admissible {
        "army order acquisition failed"
    } else {
        "army order worker admission failed"
    });
}

/// Before this hour's root census job: its order census parts follow the
/// split the last stitched hour planned, whichever pooled frame either hour
/// uses. Returns the parts' storage before the job prepares them
/// ([`charge_order_parts`]).
///
/// # Safety
/// Only the hourly callback reads and writes `shared`'s order split.
unsafe fn begin_order_parts(shared: &Shared, orders: &mut crate::map_capture::Orders) {
    // SAFETY: forwarded from the caller.
    let split = unsafe { *shared.order_split.get() };
    orders.begin_parts(split, shared.frequency);
}

/// After this hour's stitch joins: the split it planned for the next hour,
/// or one part when it planned none.
///
/// # Safety
/// As [`begin_order_parts`].
unsafe fn end_order_parts(shared: &Shared, orders: &crate::map_capture::Orders) {
    // SAFETY: forwarded from the caller.
    unsafe { *shared.order_split.get() = orders.next_split() };
}

/// A rejected request's join, out of line from the accepted path.
///
/// # Safety
/// As `finish_acquisition_into`.
#[inline(never)]
unsafe fn finish_rejected(
    batch: &mut acquisition::Batch<'_>,
    ticket: acquisition::Ticket,
    out: &mut Option<acquisition::Completion<acquisition::Job, acquisition::Report>>,
) {
    let completed = batch.finish_parked_into(ticket, out, |request| {
        let report = match request {
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
            // A census an earlier job already failed keeps that reason: a failed
            // job stops the pool, which then refuses this one.
            acquisition::Job::OrdersStitch(orders) => {
                if !orders.orders.failed() {
                    orders.fail("army order worker admission failed");
                }
                acquisition::Report::OrdersStitch(acquisition::StitchReport::default())
            }
            // The part returns unrun; the join fails the census with the
            // admission reason a refused Orders job has.
            acquisition::Job::OrdersPart(part) => {
                part.rejected = true;
                acquisition::Report::OrdersPart(acquisition::OrdersReport)
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
            acquisition::Job::OrdinaryPart(part) => {
                let report = part.fail("ordinary worker admission failed");
                acquisition::Report::OrdinaryPart(report)
            }
            acquisition::Job::Countries(countries) if countries.has_naming() => {
                let report = countries.fail_naming("country naming worker admission failed");
                acquisition::Report::Countries(report)
            }
            acquisition::Job::Templates(templates) => {
                let report = templates.fail("division template worker admission failed");
                acquisition::Report::Templates(report)
            }
            _ if request.worker_only() => return Some(Err(acquisition::Failure::Stopped)),
            _ => return None,
        };
        Some(Ok(report))
    });
    if !completed {
        // SAFETY: the unhandled request remains parked and is eligible inline.
        unsafe { finish_rejected_inline(batch, ticket, out) };
    }
}

/// Eligible inline work retains its separate completion frame. Worker-only
/// rejections never reach this path or allocate new payload storage.
#[inline(never)]
unsafe fn finish_rejected_inline(
    batch: &mut acquisition::Batch<'_>,
    ticket: acquisition::Ticket,
    out: &mut Option<acquisition::Completion<acquisition::Job, acquisition::Report>>,
) {
    let request = batch
        .unpark(ticket)
        .expect("parked inline acquisition request");
    // SAFETY: this request was rejected before acquiring any live state.
    *out = Some(unsafe { batch.run_inline(ticket, request) });
}

pub(crate) fn acquisition_failure_reason(failure: acquisition::Failure) -> &'static str {
    match failure {
        acquisition::Failure::Panicked => "acquisition worker panicked",
        acquisition::Failure::Stopped => "acquisition worker stopped before completion",
    }
}

/// The World job's scheduled parts: production and construction rests, each
/// owning a payload only in its slot. The economy rest is the Economy job's
/// ([`acquire_economy_request`]).
pub(crate) fn acquire_world_request(
    countries: &Arc<Cache>,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
) -> acquisition::WorldRequest {
    let (production, construction) = world_schedule(hour, force_full);
    acquisition::WorldRequest {
        countries: Arc::clone(countries),
        production: production.then(|| acquisition::WorldPart {
            context: extended::production::ProductionRestCopyContext { base, game_state },
            payload: decode::acquire_production_rest_job(),
        }),
        construction: construction.then(|| acquisition::WorldPart {
            context: extended::construction::ConstructionRestCopyContext { base, game_state },
            payload: decode::acquire_construction_rest_job(),
        }),
    }
}

/// Whether the World job's production and construction rests are scheduled.
fn world_schedule(hour: i32, force_full: bool) -> (bool, bool) {
    (
        replay_data::cadence::should_walk(TableId::ProductionLines, hour, force_full),
        replay_data::cadence::should_walk(TableId::StateBaseResources, hour, force_full),
    )
}

/// The Economy job: the economy rest runs every hour (province supply, the
/// resource census and its consumers), its other tables in their daily slot.
pub(crate) fn acquire_economy_request(
    countries: &Arc<Cache>,
    base: usize,
    game_state: usize,
    economy: &mut extended::economy::Captured,
    hour: i32,
    force_full: bool,
) -> acquisition::EconomyRequest {
    acquisition::EconomyRequest {
        countries: Arc::clone(countries),
        context: economy.resource_copy_context(base, game_state, hour, force_full),
        payload: decode::acquire_economy_rest_job(),
    }
}

/// The World join. A scheduled part's completed raw copy goes to its decode
/// payload and anything else is recycled; the part is then installed from
/// the payload (`rest_raw_mode`: production's here, construction's, which
/// the Gated packs hold, before their dispatch from the scheduled part). A
/// failed job lowers only its own parts' `raw_ok`; [`refuse_world_rest`]
/// refuses their tables after the assembly. Returns whether production and
/// construction were scheduled.
pub(crate) fn settle_world(
    extended: &mut extended::Captured,
    world: acquisition::WorldRequest,
    report: &mut acquisition::WorldReport,
    failure: Option<&'static str>,
    production_payload: &mut Option<extended::production::ProductionRestJob>,
    construction_payload: &mut Option<extended::construction::ConstructionRestJob>,
) -> (bool, bool) {
    let production_active = world.production.is_some();
    let construction_active = world.construction.is_some();
    let terminal = failure.is_some();
    extended.production_rest_pilot().rest_raw_mode = production_active;
    if let Some(part) = world.production {
        if report.production.raw_ok && !terminal && !report.production.plan_failed {
            *production_payload = Some(part.payload);
        } else {
            decode::recycle_production_rest_job(part.payload);
        }
    }
    if let Some(part) = world.construction {
        if report.construction.raw_ok && !terminal {
            *construction_payload = Some(part.payload);
        } else {
            decode::recycle_construction_rest_job(part.payload);
        }
    }
    if terminal {
        report.production.raw_ok = false;
        report.construction.raw_ok = false;
    }
    (production_active, construction_active)
}

/// The Economy join: a completed raw copy goes to the decode payload and
/// anything else is recycled. A failed job lowers only the economy rest's
/// `raw_ok`; [`refuse_economy_rest`] refuses its tables after the assembly.
pub(crate) fn settle_economy(
    economy: &mut extended::economy::Captured,
    request: acquisition::EconomyRequest,
    report: &mut extended::economy::EconomyRestCopyReport,
    failure: Option<&'static str>,
    payload: &mut Option<extended::economy::EconomyRestJob>,
) {
    economy.rest_raw_mode = true;
    if report.raw_ok && failure.is_none() {
        *payload = Some(request.payload);
    } else {
        decode::recycle_economy_rest_job(request.payload);
    }
    if failure.is_some() {
        report.raw_ok = false;
    }
}

/// After the assembly: a scheduled World part whose raw copy did not complete
/// is refused, and a failed World job refuses both scheduled parts with its
/// reason, charging their copy ticks. The economy tables are not touched.
pub(crate) fn refuse_world_rest(
    extended: &mut extended::Captured,
    (production_active, construction_active): (bool, bool),
    report: &acquisition::WorldReport,
    failure: Option<&'static str>,
    ticks: &mut [u64; TABLE_COUNT],
) {
    if production_active && (report.production.plan_failed || !report.production.raw_ok) {
        extended
            .production_rest_pilot()
            .fail_production_rest_capture("production acquisition plan could not complete safely");
        for (table, cost) in decode::PRODUCTION_REST
            .into_iter()
            .zip(report.production.copy_ticks)
        {
            ticks[table.index()] = cost;
        }
    }
    if construction_active && !report.construction.raw_ok && failure.is_none() {
        extended
            .construction_rest_pilot()
            .fail_construction_rest_capture("construction original acquisition could not complete");
    }
    if let Some(reason) = failure {
        if production_active {
            extended
                .production_rest_pilot()
                .fail_production_rest_capture(reason);
        }
        if construction_active {
            extended
                .construction_rest_pilot()
                .fail_construction_rest_capture(reason);
        }
        for (table, cost) in decode::PRODUCTION_REST
            .into_iter()
            .zip(report.production.copy_ticks)
            .chain(
                decode::CONSTRUCTION_REST
                    .into_iter()
                    .zip(report.construction.copy_ticks),
            )
        {
            ticks[table.index()] = cost;
        }
    }
}

/// After the assembly: an economy rest whose raw copy did not complete is
/// refused, and a failed Economy job refuses it with its reason, charging its
/// copy ticks. The production and construction tables are not touched.
pub(crate) fn refuse_economy_rest(
    economy: &mut extended::economy::Captured,
    report: &extended::economy::EconomyRestCopyReport,
    failure: Option<&'static str>,
    ticks: &mut [u64; TABLE_COUNT],
) {
    if let Some(reason) = failure {
        economy.fail_economy_rest_capture(reason);
        for (table, cost) in decode::ECONOMY_REST.into_iter().zip(report.copy_ticks) {
            ticks[table.index()] = cost;
        }
    } else if !report.raw_ok {
        economy.fail_economy_rest_capture("economy original acquisition could not complete");
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
fn modifier_table_moved(shared: &Shared, memory: &impl engine_native::Memory) -> bool {
    let observed = engine_native::modifier_definition::table(memory, shared.base)
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
        engine_sites::at(shared.base, engine_native::cship::NEXT_ID_RVA)
            .and_then(|at| engine_native::read::<i32>(memory, at)),
        engine_sites::at(
            shared.base,
            engine_native::creference_registry::ID_COUNTER_RVA,
        )
        .and_then(|at| engine_native::read::<u32>(memory, at)),
    )
}

/// The game-thread ticks a frame records for an extended table: a skipped
/// table keeps zero cost, except CountryIntel, whose walk moves to a later
/// hour when its shared buffer was held (`daily_intel::Loan::borrow`). That
/// retried walk is a real read, so it keeps its ticks: the oracle tells a
/// table read at an hour by its measured cost.
fn exported_table_ticks(table: replay_data::rows::TableId, walk: bool, ticks: u64) -> Option<u64> {
    (walk || (table == replay_data::rows::TableId::CountryIntel && ticks != 0)).then_some(ticks)
}

#[cfg(test)]
mod exported_ticks_tests;

#[cfg(test)]
mod role_timeline_tests {
    use super::*;

    /// Offsets are taken from the acquisition entry; a job whose body never
    /// ran adds to the queue and join means but not to the body means.
    #[test]
    fn role_timeline_means_offsets_from_the_acquisition_entry() {
        let timeline = RoleTimeline::new();
        timeline.record(
            1_000,
            &crate::acquisition::Timeline {
                queued_qpc: 1_010,
                body_begin_qpc: 1_100,
                body_end_qpc: 1_500,
                ..Default::default()
            },
            1_200,
            1_510,
            0,
        );
        timeline.record(
            2_000,
            &crate::acquisition::Timeline {
                queued_qpc: 2_030,
                ..Default::default()
            },
            2_050,
            2_060,
            0,
        );
        assert_eq!(timeline.means(), (2, [20, 100, 500, 125, 285]));
        assert_eq!(timeline.slow_joins(), [0; 3]);
    }

    /// Each join counts once, in the highest threshold its wait reached;
    /// the reported counts are the joins at or over each threshold.
    #[test]
    fn role_timeline_counts_slow_joins_by_threshold() {
        let timeline = RoleTimeline::new();
        let record =
            |wait_ns| timeline.record(0, &crate::acquisition::Timeline::default(), 0, 0, wait_ns);
        for wait_ns in [
            0, 249_999, 250_000, 999_999, 1_000_000, 3_999_999, 4_000_000, 90_000_000,
        ] {
            record(wait_ns);
        }
        assert_eq!(timeline.slow_joins(), [6, 4, 2]);
        assert_eq!(
            timeline
                .slow
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            [2, 2, 2]
        );
    }

    #[test]
    fn readiness_waits_compete_with_joins_without_claiming_collection_stalls() {
        reset_hour_joins();
        record_hour_join(
            1,
            10,
            4,
            #[cfg(feature = "callback_profile")]
            Default::default(),
            #[cfg(feature = "callback_profile")]
            None,
        );
        let completion = crate::acquisition_diagnostics::Completion {
            identity: crate::acquisition_diagnostics::Identity {
                role: 2,
                ..Default::default()
            },
            ticket: crate::acquisition::Ticket::diagnostic(1, 3, 4),
        };
        record_readiness_wait(100, Default::default(), Some(completion), false);
        record_hour_join(
            3,
            100,
            8,
            #[cfg(feature = "callback_profile")]
            Default::default(),
            #[cfg(feature = "callback_profile")]
            None,
        );
        let join = take_hour_joins().slowest.unwrap();
        assert_eq!(join.role, Some(2));
        assert!(join.readiness);
        assert_eq!(join.wait_ns, 100);
        assert_eq!(join.collect_stall_ns, 0);
        reset_hour_joins();
        assert!(take_hour_joins().slowest.is_none());
    }

    /// A job published before its join stalls from the join's begin to its
    /// collection; one published during it, from its publication. The first
    /// finish-lock wait and the worker's wait to publish add up apart, and a
    /// job never published (rejected or inline work) is not counted.
    #[test]
    fn join_costs_split_collection_from_the_unfinished_body() {
        let costs = JoinCosts::new();
        let ns = |ticks: i64| ticks as u64 * 100;
        // Ready before its join at 2_000; collected 3_000 ticks later.
        costs.record(
            &crate::acquisition::Timeline {
                queued_qpc: 100,
                body_begin_qpc: 200,
                body_end_qpc: 1_000,
                published_qpc: 1_040,
                collected_qpc: 5_000,
                finish_lock_ticks: 2_900,
                growths: 0,
                caller: false,
                thread_work: None,
            },
            2_000,
            ns,
        );
        // Published at 6_500 during its join from 6_000; collected at 6_510.
        costs.record(
            &crate::acquisition::Timeline {
                queued_qpc: 100,
                body_begin_qpc: 300,
                body_end_qpc: 6_490,
                published_qpc: 6_500,
                collected_qpc: 6_510,
                finish_lock_ticks: 1,
                growths: 0,
                caller: false,
                thread_work: None,
            },
            6_000,
            ns,
        );
        costs.record(&crate::acquisition::Timeline::default(), 7_000, ns);
        assert_eq!(costs.totals(), (2, 1, [2_901, 3_000 + 10, 40 + 10], [1, 0]));
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod capture_scope_test_kit;
