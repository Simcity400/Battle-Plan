use std::sync::Arc;
mod divisions;
use divisions::fail_divisions_capture;
pub use divisions::{DivisionsCopyContext, DivisionsJob, decode_divisions_payload};
pub(crate) use divisions::{
    DivisionsCopyReport, copy_divisions_core_refresh, copy_divisions_inputs_refresh,
};
#[cfg(any(test, feature = "test-support"))]
pub use divisions::{copy_divisions_refresh, divisions_test_kit};

use crate::acquisition_factions;
use crate::acquisition_jobs as acquisition;
use crate::decode;
use crate::extended;
pub use collector_extended_support::hourly::BUFFERS;
use collector_extended_support::hourly::TICKING;
use collector_map_capture::map_capture::MapCapacities;
use collector_map_capture::map_capture::MapCaptured;
use collector_map_capture::map_capture::ShipCensus;
use collector_metadata::roster::Roster;
use collector_primitives::clock::qpc;
use collector_primitives::table_capture::grow_capacity;
use collector_states::{acquisition_states, acquisition_wars};
#[cfg(test)]
use engine_native::cunit;
#[cfg(any(test, feature = "test-support"))]
use engine_native_memory::memory::CountedMemory;
#[cfg(test)]
use engine_native_memory::memory::ReadCounts;
use engine_native_memory::{
    Memory,
    capture_text::Owned,
    memory::LiveMemory,
    save_allocation_counters::{self, AllocationCounters},
    walk::Outcome,
};
use engine_native::{
    carmy,
    ccountry,
    cfaction::{self, system as cfaction_system},
    cgame_state::{self, Clock},
    cprovince,
    cstate,
    cwar_relation,
};
use engine_native_intelligence::country_memory::{Cache, SeedStatus};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use std::{
    cell::UnsafeCell,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

use collector_workers::SEGMENTS;

const SUPPLY_PARTS: usize = collector_supply_trains::supply_trains::PARTS;
const CHARACTER_PARTS: usize = collector_characters::extended::characters::PARTS;
const AIR_RELATION_PARTS: usize = collector_air_relations::air_relations::PARTS;
const AIR_HISTORY_PARTS: usize = collector_air_histories::air_histories::PARTS;
const PORTRAIT_PARTS: usize = collector_characters::extended::character_portraits::PARTS;
pub(crate) const INITIAL_CAPACITIES: [usize; 12] = [0; 12];

pub(crate) fn raise_for_countries(core: &mut [usize; 12], slots: usize) {
    let rows = collector_primitives::country_slots::rows(slots, 1);
    for index in [0, 2, 7] {
        collector_primitives::country_slots::raise(&mut core[index], rows);
    }
}
pub const SAMPLE_BOUNDARY: replay_data::identity::SampleBoundary =
    replay_data::identity::SampleBoundary::AfterHourlyUpdate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    Held,
    Beside(Option<u32>),
}
const LOSS_RUNS: usize = 64;
pub static SHARED: SharedSlot = SharedSlot(AtomicPtr::new(ptr::null_mut()));

#[derive(Default)]
pub struct SharedSlot(AtomicPtr<Shared>);

impl SharedSlot {
    pub fn is_published(&self) -> bool {
        !self.load(Ordering::Acquire).is_null()
    }

    #[doc = "# Safety"]
    #[doc = "Keep the shared state alive until the slot is cleared and all users have quiesced."]
    pub unsafe fn publish(&self, shared: &Shared) {
        self.0
            .store(std::ptr::from_ref(shared).cast_mut(), Ordering::Release);
    }

    /// # Safety
    /// Only the owning runtime may unpublish the live slot, after restoring hooks and waiting for
    /// every callback and worker to stop borrowing Shared.
    pub unsafe fn clear(&self) {
        self.0.store(ptr::null_mut(), Ordering::Release);
    }

    #[doc = "# Safety"]
    #[doc = "The shared state must remain alive for the lifetime of the returned reference."]
    pub unsafe fn current(&self) -> Option<&Shared> {
        unsafe { self.load(Ordering::Acquire).as_ref() }
    }

    pub(crate) fn load(&self, order: Ordering) -> *mut Shared {
        self.0.load(order)
    }

    #[cfg(test)]
    pub(crate) fn store(&self, shared: *mut Shared, order: Ordering) {
        self.0.store(shared, order);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_for_test() -> Self {
        Self(AtomicPtr::new(
            std::ptr::NonNull::<Shared>::dangling().as_ptr(),
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn identity_for_test(&self) -> SharedIdentity {
        SharedIdentity(self.load(Ordering::Acquire))
    }
}
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Eq, PartialEq)]
pub struct SharedIdentity(*mut Shared);

#[cfg(any(test, feature = "test-support"))]
impl SharedIdentity {
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

pub(crate) static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
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
            && self.prev_world == collector_lifetime::world_lifetime::generation()
    }
    #[cfg(test)]
    fn force_full(&mut self, clock: &Clock<&[u8]>) -> bool {
        self.full_reasons(clock) != 0
    }

    fn full_reasons(&mut self, clock: &Clock<&[u8]>) -> u8 {
        if !self.initialized {
            self.initialized = true;
            self.prev_hour = clock.hour;
            self.prev_signal = clock.play_session_started_at;
            self.prev_campaign
                .copy_prepared(clock.campaign_id)
                .expect("campaign prepared before callback");
            self.prev_world = collector_lifetime::world_lifetime::generation();
            return FULL_FIRST;
        }
        let force = u8::from(self.prev_signal != clock.play_session_started_at) * FULL_SIGNAL
            | u8::from(self.prev_campaign.as_bytes() != clock.campaign_id) * FULL_CAMPAIGN
            | u8::from(self.prev_hour.checked_add(1) != Some(clock.hour)) * FULL_GAP
            | u8::from(self.prev_world != collector_lifetime::world_lifetime::generation())
                * FULL_WORLD;
        self.prev_hour = clock.hour;
        self.prev_signal = clock.play_session_started_at;
        self.prev_campaign
            .copy_prepared(clock.campaign_id)
            .expect("campaign prepared before callback");
        self.prev_world = collector_lifetime::world_lifetime::generation();
        force
    }
}

pub const FULL_FIRST: u8 = 1;
pub const FULL_SIGNAL: u8 = 2;
pub const FULL_CAMPAIGN: u8 = 4;
pub const FULL_GAP: u8 = 8;
pub const FULL_WORLD: u8 = 16;
pub const FULL_RECOVERY: u8 = 32;
pub const FULL_REFRESH: u8 = 64;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct HourPhases {
    pub entry_ns: u64,
    pub frame_grow_ns: u64,
    pub free_frames: u32,
    pub setup_ns: u64,
    pub parallel_ns: u64,
    pub residual_ns: u64,
    pub segments_ns: [u64; SEGMENTS],
    pub slowest_join: Option<(usize, u64)>,
    pub slowest_ready: Option<(EarlyStep, u64)>,
    pub slowest_step: Option<(EarlyStep, u64)>,
    pub heap_calls: u64,
    pub heap_ns: u64,
    pub page_calls: u64,
    pub page_ns: u64,
    pub force_full: bool,
    pub full_reasons: u8,
    pub retry_cause: u16,
    pub holders: Option<Holders>,
    pub cycles: Option<u64>,
    pub variant_drain_ns: u64,
    pub variant: collector_production::production::design_changes::DrainProfile,
}

#[derive(Clone, Copy, Debug)]
pub struct Holders {
    pub stages: [(u32, u64); crate::frame_pool::FramePool::STAGES.len()],
    pub spool: collector_workers::spool_phases::Phase,
    pub helper: collector_workers::spool_phases::Phase,
    pub sizing: collector_workers::spool_phases::Phase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildAhead {
    Starting,
    Free,
    Parking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStage {
    Free,
    Callback,
    Queued,
    Decoding,
    Ready,
    Spool,
    Recycle,
}

fn holders(shared: &Shared) -> Holders {
    Holders {
        stages: shared
            .frames
            .stages(qpc() as u64)
            .map(|(frames, ticks)| (frames, shared.ns(ticks as i64))),
        spool: collector_workers::spool_phases::SPOOL.current(),
        helper: collector_workers::spool_phases::HELPER.current(),
        sizing: collector_workers::spool_phases::SIZING.current(),
    }
}

thread_local! {
    static SLOWEST_JOIN: std::cell::Cell<Option<(usize, u64)>> = const { std::cell::Cell::new(None) };
}

pub struct Captured {
    pub world_generation: u64,
    pub fixed_layout: Option<collector_metadata::fixed_layout::Layout>,
    pub capture_seq: u64,
    pub map: MapCaptured,
    pub extended: extended::Captured,
    pub country_cache_status: SeedStatus,
    pub(crate) country_cache_entries: usize,
    pub(crate) country_cache_ns: u64,
    pub clock: Option<Clock>,
    clock_spare: Owned,
    pub counters: Option<AllocationCounters>,
    pub countries: Vec<ccountry::Row>,
    pub provinces: cprovince::Capture,
    pub states: Vec<cstate::Row>,
    pub state_inputs: acquisition_states::Inputs,
    pub state_names: Vec<u8>,
    pub state_names_len: usize,
    pub(crate) state_names_required: usize,
    pub divisions: Vec<carmy::Row>,
    pub wars: Vec<cwar_relation::Row>,
    pub(crate) wars_snapshot: cwar_relation::OwnedSnapshotScratch,
    pub(crate) wars_snapshot_sampled: bool,
    pub factions: Vec<cfaction::Row>,
    pub faction_names: Vec<u8>,
    pub faction_goals: collector_factions::faction_goals::Captured,
    pub faction_companions: collector_factions::faction_companions::Captured,
    pub faction_names_len: usize,
    pub(crate) faction_names_required: usize,
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
    pub phases: HourPhases,
    #[cfg(feature = "acquisition_profile")]
    pub table_reads: [engine_native_memory::memory::ReadCounts; TABLE_COUNT],
    pub total_ns: u64,
    pub force_full: bool,
    pub qpc: i64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
    pub dropped_before: LossBatch,
    verdicts_reset: bool,
}

impl Captured {
    pub(crate) fn reset_verdicts(&mut self) {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.table_ns = [0; TABLE_COUNT];
        self.force_full = false;
        #[cfg(feature = "acquisition_profile")]
        {
            self.table_reads = collector_memory::read_accounting::ZERO;
        }
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
        self.faction_goals = Default::default();
        self.faction_companions = Default::default();
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
        self.fixed_layout = None;
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Allocate only in the session's attach, sizing or capture sequence, or in an isolated
    /// fixture. Keep live observer stores active and exclude conflicting sizing or teardown while
    /// creating packs.
    pub unsafe fn new() -> Self {
        let packs = extended::Capacities::default();
        let mut frame =
            // SAFETY: The session or isolated fixture owns this frame's observer allocations and
            // excludes teardown.
            unsafe { Self::with_capacities(&INITIAL_CAPACITIES, &MapCapacities::default(), &packs) }.unwrap();
        frame
            .extended
            .share_homes(&extended::Homes::new(&packs).unwrap());
        frame
    }

    /// # Safety
    /// Allocate only in the session's attach, sizing or capture sequence, or in an isolated
    /// fixture. Keep live observer stores active and exclude conflicting sizing or teardown while
    /// creating packs.
    pub unsafe fn with_capacities(
        capacities: &[usize; 12],
        map_capacities: &MapCapacities,
        extended_capacities: &extended::Capacities,
    ) -> std::io::Result<Self> {
        // SAFETY: Caller guarantees live observer storage and exclusive frame preparation.
        unsafe { Self::with_capacities_mode(capacities, map_capacities, extended_capacities, false) }
    }

    /// # Safety
    /// Allocate only in the session's attach, sizing or capture sequence, or in an isolated
    /// fixture. Keep live observer stores active and exclude conflicting sizing or teardown while
    /// creating packs.
    pub unsafe fn with_capacities_mode(
        capacities: &[usize; 12],
        map_capacities: &MapCapacities,
        extended_capacities: &extended::Capacities,
        parked: bool,
    ) -> std::io::Result<Self> {
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        let extended = if parked {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { extended::Captured::frame_parked(extended_capacities) }
        } else {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { extended::Captured::frame(extended_capacities) }
        };
        let frame = Self {
            world_generation: collector_lifetime::world_lifetime::generation(),
            capture_seq: 0,
            fixed_layout: None,
            map:
            // SAFETY: These are session-measured map capacities or capacities owned by an isolated
            // fixture.
            unsafe { MapCaptured::with_capacities(map_capacities) },
            extended,
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
            phases: HourPhases::default(),
            #[cfg(feature = "acquisition_profile")]
            table_reads: collector_memory::read_accounting::ZERO,
            total_ns: 0,
            force_full: false,
            qpc: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            dropped_before: LossBatch::default(),
            verdicts_reset: false,
        };
        if collector_memory::terminal::failed() {
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
    fn clock_bytes(&self) -> usize {
        self.clock
            .as_ref()
            .map_or(0, |clock| clock.campaign_id.allocated_bytes())
            + self.clock_spare.allocated_bytes()
            + self.dropped_before.allocated_bytes()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.core_allocated_bytes() + self.map.allocated_bytes() + self.extended.allocated_bytes()
    }

    pub(crate) fn core_allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.clock_bytes()
            + self.wars_snapshot.allocated_bytes()
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

    pub fn take_states_request(
        &mut self,
        base: usize,
        game_state: usize,
    ) -> acquisition_states::Request {
        acquisition_states::Request {
            census: collector_states::acquisition_gather::Request::take(
                base,
                game_state,
                &mut self.states,
            ),
            names: acquisition_states::Names::take(&mut self.state_names),
            visitor: self.extended.take_state_visitor(),
            inputs: std::mem::take(&mut self.state_inputs),
            store: None,
            countries: None,
            occupation_store: None,
        }
    }

    pub fn restore_states_request(
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
        countries: &engine_allocation::SharedBox<Cache>,
        base: usize,
        game_state: usize,
    ) -> acquisition_wars::Request {
        acquisition_wars::Request {
            countries: engine_allocation::SharedBox::clone(countries),
            table: collector_states::acquisition_gather::Request::take(
                base,
                game_state,
                &mut self.wars,
            ),
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

    pub fn grow(&mut self, capacities: &mut [usize; 12]) {
        engine_allocation::advisory(|| self.grow_hints(capacities));
    }

    fn grow_hints(&mut self, capacities: &mut [usize; 12]) {
        fn resize<T: Default + Clone>(rows: &mut Vec<T>, outcome: Outcome, capacity: &mut usize) {
            match outcome {
                Outcome::Rows { len } => grow_capacity(capacity, len),
                Outcome::Truncated { len, unread_rows } => {
                    grow_capacity(capacity, len.saturating_add(unread_rows as usize));
                }
                Outcome::Unavailable { .. } => {}
            }
            if rows.len() < *capacity {
                collector_primitives::table_capture::grow_vec(rows, *capacity);
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
            collector_primitives::table_capture::grow_vec(
                &mut self.provinces.groups,
                capacities[10],
            );
        }
        resize(&mut self.links, self.links_result, &mut capacities[2]);
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
        resize(&mut self.wars, self.wars_result, &mut capacities[5]);
        resize(&mut self.factions, self.factions_result, &mut capacities[6]);
        resize(
            &mut self.faction_members,
            self.faction_members_result,
            &mut capacities[7],
        );
        grow_capacity(&mut capacities[8], self.faction_names_required);
        collector_primitives::table_capture::grow_vec(&mut self.faction_names, capacities[8]);
        grow_capacity(&mut capacities[11], self.state_names_required);
        collector_primitives::table_capture::grow_vec(&mut self.state_names, capacities[11]);
    }
}

#[cfg(feature = "callback_profile")]
pub struct ProfilePhases {
    pub frame_grow_ns: AtomicU64,
    pub setup_ns: AtomicU64,
    pub parallel_ns: AtomicU64,
    pub residual_ns: AtomicU64,
    pub calls: AtomicU64,
    pub(crate) dispatch_ns: AtomicU64,
    pub segments: [AtomicU64; SEGMENTS],
    pub(crate) join_wait: [AtomicU64; collector_workers::roles::ROLE_COUNT],
}

#[cfg(feature = "callback_profile")]
impl ProfilePhases {
    fn new() -> Self {
        Self {
            frame_grow_ns: AtomicU64::new(0),
            setup_ns: AtomicU64::new(0),
            parallel_ns: AtomicU64::new(0),
            residual_ns: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            dispatch_ns: AtomicU64::new(0),
            segments: std::array::from_fn(|_| AtomicU64::new(0)),
            join_wait: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FrameCapacities {
    pub core: [usize; 12],
    pub map: MapCapacities,
    pub extended: extended::Capacities,
}

struct WaitCounter {
    calls: AtomicU64,
    ns: AtomicU64,
    max_ns: AtomicU64,
}

impl WaitCounter {
    pub const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    pub fn add(&self, ns: u64) {
        if ns == 0 {
            return;
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    pub fn read(&self) -> [u64; 3] {
        [&self.calls, &self.ns, &self.max_ns].map(|value| value.load(Ordering::Relaxed))
    }
}

pub struct Shared {
    pub(crate) acquisition: OnceLock<acquisition::Pool>,
    pub(crate) acquisition_panics: AtomicU64,
    #[cfg(feature = "callback_profile")]
    pub(crate) profile: ProfilePhases,
    pub(crate) native_tick_ns: AtomicU64,
    pub(crate) native_tick_calls: AtomicU64,
    pub base: usize,
    trampoline: usize,
    pub frequency: u64,
    pub game_score_enabled: bool,
    pub capture_mode: replay_data::scope::CaptureMode,
    pub(crate) tick_idle: Option<collector_workers::event::Event>,
    pub(crate) frames: crate::frame_pool::FramePool,
    pub(crate) frame_capacities: engine_allocation::SharedBox<crate::frame_capacities::Capacities>,
    pub(crate) history_frames: collector_workers::frame_fence::Frames,
    heap_waits: WaitCounter,
    pub(crate) frame_priority:
        engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline>,
    pub(crate) dropped: AtomicU64,
    pub(crate) pool_bytes: engine_allocation::SharedBox<AtomicU64>,
    pub(crate) recording: AtomicBool,
    pub(crate) stop_spool: AtomicBool,
    pub(crate) spool_waker: OnceLock<std::thread::Thread>,
    pub(crate) decode_waker: OnceLock<std::thread::Thread>,
    pub(crate) roster: UnsafeCell<Option<Box<Roster>>>,
    pub(crate) roster_ready: AtomicBool,
    roster_sampling: AtomicBool,
    pub(crate) pending: UnsafeCell<LossBatch>,
    clock_capture: UnsafeCell<ClockCapture>,
    clock_required: AtomicUsize,
    clock_frames: AtomicUsize,
    cadence: UnsafeCell<CadenceState>,
    slice_fingerprints: [UnsafeCell<
        Option<Box<collector_certificates::slicing::SliceFingerprints>>,
    >; collector_certificates::slicing::FAMILIES.len()],
    country_cache: UnsafeCell<engine_allocation::SharedBox<Cache>>,
    weather_carry: UnsafeCell<Option<Box<collector_weather::weather::carry::WeatherCarry>>>,
    raid_air_scores: UnsafeCell<Option<Box<engine_raid_inputs::raid_air_scores::Held>>>,
    raid_air_score_bytes: AtomicU64,
    character_role_store:
        UnsafeCell<Option<Box<collector_characters::extended::character_inputs::RoleStore>>>,
    character_role_store_bytes: AtomicU64,
    script_flag_store: UnsafeCell<Option<Box<collector_intel::intel::FlagStore>>>,
    script_flag_store_bytes: AtomicU64,
    script_variable_store: UnsafeCell<Option<Box<collector_intel::intel::VariableStore>>>,
    script_variable_store_bytes: AtomicU64,
    faction_goal_store: UnsafeCell<Option<Box<collector_factions::faction_goals::Store>>>,
    faction_goal_store_bytes: AtomicU64,
    faction_companion_store: UnsafeCell<Option<Box<collector_factions::faction_companions::Store>>>,
    faction_companion_store_bytes: AtomicU64,
    building_store: UnsafeCell<Option<Box<collector_construction::construction::BuildingStore>>>,
    building_store_bytes: AtomicU64,
    leader_store: UnsafeCell<Option<Box<collector_governance::governance::LeaderStore>>>,
    leader_store_bytes: AtomicU64,
    party_store: UnsafeCell<Option<Box<collector_governance::governance::PartyStore>>>,
    party_store_bytes: AtomicU64,
    wing_store: UnsafeCell<Option<Box<collector_air::extended::air::WingStore>>>,
    wing_store_bytes: AtomicU64,
    air_wing_parts: UnsafeCell<
        [collector_air::extended::air::AirWingJob; collector_air::extended::air::WING_PARTS - 1],
    >,
    air_wing_part_starts: UnsafeCell<([u32; collector_air::extended::air::WING_PARTS - 1], usize)>,
    production_part_starts: UnsafeCell<(
        [u32; collector_production_daily::production_modifiers::MAX_PARTS - 1],
        usize,
    )>,
    air_base_store: UnsafeCell<Option<Box<collector_air::extended::air::RegistrationStore>>>,
    air_base_store_bytes: AtomicU64,
    mission_store: UnsafeCell<Option<Box<collector_air::extended::air::MissionStore>>>,
    mission_store_bytes: AtomicU64,
    rail_store:
        UnsafeCell<Option<Box<collector_naval_history::extended::naval_history::RailStore>>>,
    rail_store_bytes: AtomicU64,
    state_store: UnsafeCell<Option<Box<acquisition_states::StateStore>>>,
    state_store_bytes: AtomicU64,
    occupation_store: UnsafeCell<Option<Box<acquisition_states::OccupationStore>>>,
    occupation_store_bytes: AtomicU64,
    war_store: UnsafeCell<Option<Box<acquisition_wars::WarStore>>>,
    war_store_bytes: AtomicU64,
    army_stat_store: Option<collector_divisions::extended::division_presentation::held_store::Slot>,
    army_stat_store_bytes: AtomicU64,
    division_parts: UnsafeCell<
        [Option<Box<collector_divisions::division_census::Census>>;
            collector_divisions::division_census::MAX_PARTS - 1],
    >,
    division_part_starts: UnsafeCell<(
        [u32; collector_divisions::division_census::MAX_PARTS - 1],
        usize,
    )>,
    order_split: UnsafeCell<collector_divisions::orders_parts::Split>,
    division_join_capacity: UnsafeCell<collector_divisions::division_census::JoinCapacity>,
    province_unit_store: UnsafeCell<
        Option<Box<collector_divisions::extended::province_unit_order::ProvinceUnitStore>>,
    >,
    province_unit_store_bytes: AtomicU64,
    area_store: UnsafeCell<Option<Box<engine_native::cprovince::controller_areas::Store>>>,
    area_store_bytes: AtomicU64,
    province_store: UnsafeCell<Option<Box<engine_native::cprovince::carry::RowStore>>>,
    province_store_bytes: AtomicU64,
    advisor_store: UnsafeCell<Option<Box<collector_advisors::hired_advisors::AdvisorStore>>>,
    advisor_store_bytes: AtomicU64,
    local_subunit_store: UnsafeCell<Option<Box<collector_model_inputs::model_inputs::LocalStore>>>,
    local_subunit_store_bytes: AtomicU64,
    doctrine_store: engine_allocation::SharedBox<Mutex<collector_slow_tables::doctrines::Store>>,
    doctrine_store_bytes: AtomicU64,
    counter_store:
        UnsafeCell<Option<Box<collector_country_counters::country_counters::CounterStore>>>,
    counter_store_bytes: AtomicU64,
    appearance_store:
        UnsafeCell<Option<Box<collector_country_counters::country_appearance::AppearanceStore>>>,
    appearance_store_bytes: AtomicU64,
    pub(crate) force_next: AtomicBool,
    pub(crate) refresh_next: AtomicBool,
    pub(crate) forget_families: std::sync::atomic::AtomicU32,
    pub(crate) lexical_max: AtomicI32,
    pub(crate) fixed_layout: OnceLock<engine_fixed_layout::fixed_layout::Report>,
    pub(crate) fixed_layout_buffer: UnsafeCell<Vec<u8>>,
    pub(crate) modifier_table: [AtomicU64; 2],
    pub(crate) division_mark: AtomicU64,
    pub(crate) country_cache_swap: Mutex<CountryCacheSwap>,
    pub(crate) country_cache_cells: AtomicUsize,
    pub(crate) country_intel: collector_intel::daily_intel::Home,
    pub(crate) template_home: collector_division_templates::extended::division_templates::Home,
    pub(crate) naval_combat_home: collector_naval_details::naval_combat_sources::Home,
    pub(crate) weather_temperature: Mutex<collector_weather::weather::temperature::Tracker>,
    parked_packs: Mutex<Vec<Box<extended::ParkedPacks>>>,
    parked_packs_bytes: AtomicU64,
    growing_parked_bytes: AtomicU64,
    retired_parked: Mutex<Vec<Box<extended::ParkedPacks>>>,
    retired_parked_bytes: AtomicU64,
    retired_parked_pending: AtomicBool,
    pub(crate) builder_waker: OnceLock<std::thread::Thread>,
    #[cfg(test)]
    pub(crate) clock_frame_seam: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

#[derive(Default)]
pub struct CountryCacheSwap {
    pub next: Option<engine_allocation::SharedBox<Cache>>,
    pub retired: Option<engine_allocation::SharedBox<Cache>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Return {
    Newest,
    Kept,
    Idle,
}

unsafe impl Sync for Shared {}

impl Shared {
    pub fn heap_waits(&self) -> [u64; 3] {
        self.heap_waits.read()
    }

    /// # Safety
    /// Only attach may install the pool, before publication. Only the runtime and its admitted
    /// capture workers may use the installed pool, and only until it has joined at detach.
    pub unsafe fn acquisition(&self) -> &OnceLock<acquisition::Pool> {
        &self.acquisition
    }

    /// # Safety
    /// Only attach may set the idle event, before publishing the shared state.
    pub unsafe fn tick_idle_mut(&mut self) -> &mut Option<collector_workers::event::Event> {
        &mut self.tick_idle
    }

    /// # Safety
    /// Use the live pool only within this session's runtime, acquire, decode and spool sequences.
    /// Respect slot ownership for every frame operation and keep the pool live through detach.
    pub unsafe fn frames(&self) -> &crate::frame_pool::FramePool {
        &self.frames
    }

    /// # Safety
    /// Only the session sizing owner may mutate these capacities. Publish and read them only for
    /// this session's own frames.
    pub unsafe fn frame_capacities(
        &self,
    ) -> &engine_allocation::SharedBox<crate::frame_capacities::Capacities> {
        &self.frame_capacities
    }

    /// # Safety
    /// Use only this session's fence with this session's frame slots. The capture sequence
    /// publishes it and the spool alone checks it for history requests.
    pub unsafe fn history_frames(&self) -> &collector_workers::frame_fence::Frames {
        &self.history_frames
    }

    /// # Safety
    /// Use only the session's priority pipeline, registering only its owned worker threads and
    /// keeping the pipeline live until they join.
    pub unsafe fn frame_priority(
        &self,
    ) -> &engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline> {
        &self.frame_priority
    }

    /// # Safety
    /// Only the capture path may account dropped frames. Runtime and spool status may read this
    /// counter for the active session.
    pub unsafe fn dropped(&self) -> &AtomicU64 {
        &self.dropped
    }

    /// # Safety
    /// Use only to account memory for owners in this session and while the session is live.
    /// Runtime and spool status may read the current total.
    pub unsafe fn pool_bytes(&self) -> &engine_allocation::SharedBox<AtomicU64> {
        &self.pool_bytes
    }

    /// # Safety
    /// Only the runtime or the originating spool may stop recording, in response to terminal
    /// failure or as part of detach.
    pub unsafe fn recording(&self) -> &AtomicBool {
        &self.recording
    }

    /// # Safety
    /// Only the runtime may set this flag, after quiescing capture and arranging for the spool to
    /// drain. Only its spool reads it to decide when to exit.
    pub unsafe fn stop_spool(&self) -> &AtomicBool {
        &self.stop_spool
    }

    /// # Safety
    /// Only the session's spool thread may register itself, once before consuming frames.
    pub unsafe fn spool_waker(&self) -> &OnceLock<std::thread::Thread> {
        &self.spool_waker
    }

    /// # Safety
    /// Only the originating spool or detached fixture may consume the roster, after observing its
    /// ready flag with acquire ordering and only once.
    pub unsafe fn roster(&self) -> &UnsafeCell<Option<Box<Roster>>> {
        &self.roster
    }

    /// # Safety
    /// Use only with the roster from this same session. Only the roster producer may publish
    /// readiness, and the spool must observe readiness before taking the roster.
    pub unsafe fn roster_ready(&self) -> &AtomicBool {
        &self.roster_ready
    }

    /// # Safety
    /// Only the sole spool consumer may take this loss batch, after acquisition is quiescent and
    /// before disposing of capture state.
    pub unsafe fn pending(&self) -> &UnsafeCell<LossBatch> {
        &self.pending
    }

    /// # Safety
    /// Only the isolated retry fixture may inspect or change this flag directly outside capture.
    pub unsafe fn force_next(&self) -> &AtomicBool {
        &self.force_next
    }

    /// # Safety
    /// Only an isolated retry fixture may change this flag directly outside the session capture
    /// and spool.
    pub unsafe fn refresh_next(&self) -> &AtomicBool {
        &self.refresh_next
    }

    /// # Safety
    /// Only the session's spool may change this set, using the families from a failed publication
    /// before capture takes the recovery request.
    pub unsafe fn forget_families(&self) -> &std::sync::atomic::AtomicU32 {
        &self.forget_families
    }

    /// # Safety
    /// Use this home for the originating session's sizing and capture loans. Return each loan
    /// before its frame or session is retired.
    pub unsafe fn country_intel(&self) -> &collector_intel::daily_intel::Home {
        &self.country_intel
    }

    /// # Safety
    /// Use this home only for the originating session's sizing and capture, with no competing
    /// teardown.
    pub unsafe fn template_home(
        &self,
    ) -> &collector_division_templates::extended::division_templates::Home {
        &self.template_home
    }

    /// # Safety
    /// Use this home only for the originating session's sizing and capture, with no competing
    /// teardown.
    pub unsafe fn naval_combat_home(&self) -> &collector_naval_details::naval_combat_sources::Home {
        &self.naval_combat_home
    }

    /// # Safety
    /// Only the native callback may advance this counter. The status writer may read it, and
    /// isolated fixtures may seed it before publication.
    pub unsafe fn native_tick_calls(&self) -> &AtomicU64 {
        &self.native_tick_calls
    }

    /// # Safety
    /// Only the native callback may advance this counter. The status writer may read it for the
    /// active session.
    pub unsafe fn native_tick_ns(&self) -> &AtomicU64 {
        &self.native_tick_ns
    }

    /// # Safety
    /// Only the originating spool may request a refresh after unpublished output, while the
    /// session's capture state is live.
    pub unsafe fn request_retry_full(&self) {
        self.refresh_next.store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// The attaching runtime or isolated fixture must have exclusive use of the global observer
    /// bindings. Keep Shared alive through hook restoration and callback quiescence once its
    /// doctrine store is bound.
    pub unsafe fn new(base: usize, trampoline: usize, frequency: u64) -> Self {
        // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
        // boundary or detach.
        unsafe {
            Self::with_fixture_sizing(
                base,
                trampoline,
                frequency,
                &crate::country_scale::Sizing::for_slots(0),
                0,
            )
        }
        .expect("synthetic capture buffers")
    }

    /// # Safety
    /// The attaching runtime or isolated fixture must have exclusive use of the global observer
    /// bindings. Keep Shared alive through hook restoration and callback quiescence once its
    /// doctrine store is bound.
    pub unsafe fn with_sizing(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
    ) -> std::io::Result<Self> {
        let shared =
        // SAFETY: This sequence prepares its own attachment with no competing observer boundary.
        unsafe { Self::without_frames(base, trampoline, frequency, sizing) }?;
        shared.publish_steady_frames()?;
        shared.bind_doctrine_observer();
        Ok(shared)
    }

    fn bind_doctrine_observer(&self) {
        // SAFETY: `with_sizing` binds the store and all three handlers before any hook is
        // published. Shared keeps one owner of the store for its whole life, and a slow request
        // only holds another, so no request dropped on any path frees it; Shared itself is dropped
        // only after hook restoration and native callback quiescence.
        unsafe {
            collector_observers::doctrine_changes::bind(
                engine_allocation::SharedBox::as_ptr(&self.doctrine_store) as usize,
                collector_slow_tables::doctrines::observe,
            );
            collector_observers::doctrine_changes::bind_cost_consumption(
                collector_slow_tables::doctrines::observe_cost_consumption,
            );
            collector_observers::doctrine_changes::bind_gain(
                collector_slow_tables::doctrines::observe_gain,
            );
        }
    }

    pub(crate) fn publish_steady_frames(&self) -> std::io::Result<()> {
        for index in 0..crate::frame_pool::STEADY {
            let frame = if index + 1 < crate::frame_pool::STEADY {
                self.new_frame_parked()?
            } else {
                self.new_frame()?
            };
            self.publish_frame(frame, false)?;
        }
        Ok(())
    }

    /// # Safety
    /// Only the attaching runtime or an isolated collector fixture may prime the live observers.
    /// Exclude other observer boundaries, capture and detach from construction.
    pub unsafe fn without_frames(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
    ) -> std::io::Result<Self> {
        let fixed_layout_bytes = engine_fixed_layout::fixed_layout::longest_name();
        // SAFETY: The caller excludes competing observer boundaries, capture and detach.
        unsafe { Self::allocate(base, trampoline, frequency, sizing, fixed_layout_bytes) }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// The fixture must have exclusive use of the global observer bindings. Keep Shared alive
    /// through callback quiescence once its doctrine store is bound.
    pub unsafe fn with_fixture_sizing(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
        fixed_layout_bytes: usize,
    ) -> std::io::Result<Self> {
        // SAFETY: The isolated fixture owns these observer bindings.
        let shared = unsafe {
            Self::allocate(base, trampoline, frequency, sizing, fixed_layout_bytes)
        }?;
        shared.publish_steady_frames()?;
        shared.bind_doctrine_observer();
        Ok(shared)
    }

    unsafe fn allocate(
        base: usize,
        trampoline: usize,
        frequency: u64,
        sizing: &crate::country_scale::Sizing,
        fixed_layout_bytes: usize,
    ) -> std::io::Result<Self> {
        let fixed_layout_buffer = engine_allocation::default_vec::<u8>(fixed_layout_bytes)?;
        let roster = engine_allocation::boxed(Roster::with_counts(sizing.content))?;
        let country_cache =
            engine_allocation::shared(Cache::try_with_capacity(sizing.country_cells)?)?;
        let country_cells = country_cache.capacity();
        let extended::Homes {
            country_intel,
            templates: template_home,
            naval_combat: naval_combat_home,
        } = extended::Homes::new(&sizing.extended)?;
        let air = &sizing.extended.air;
        let mut raid_air_scores =
            engine_allocation::boxed(engine_raid_inputs::raid_air_scores::Held::default())?;
        raid_air_scores.grow(air.scores.regions, air.scores.countries, air.scores.rows)?;
        let raid_air_score_bytes = raid_air_scores.allocated_bytes();
        let pool_bytes = roster.allocated_bytes()
            + country_cache.allocated_bytes()
            + country_intel.allocated_bytes()
            + raid_air_score_bytes
            + fixed_layout_buffer.capacity();
        fn store<T>(enabled: bool, new: impl FnOnce() -> T) -> std::io::Result<Option<Box<T>>> {
            enabled.then(|| engine_allocation::boxed(new())).transpose()
        }
        let character_role_store = store(
            cfg!(feature = "character_changes"),
            collector_characters::extended::character_inputs::RoleStore::new,
        )?;
        let script_flag_store = store(
            cfg!(feature = "flag_changes"),
            collector_intel::intel::FlagStore::new,
        )?;
        let script_variable_store = store(
            cfg!(feature = "variable_changes"),
            collector_intel::intel::VariableStore::new,
        )?;
        let faction_goal_store = store(
            cfg!(feature = "faction_changes"),
            collector_factions::faction_goals::Store::default,
        )?;
        let faction_companion_store = store(
            cfg!(feature = "faction_changes"),
            collector_factions::faction_companions::Store::default,
        )?;
        let building_store = store(
            cfg!(feature = "building_changes"),
            collector_construction::construction::BuildingStore::new,
        )?;
        let leader_store = store(
            cfg!(feature = "party_leader_changes"),
            collector_governance::governance::LeaderStore::new,
        )?;
        let party_store = store(
            cfg!(feature = "party_changes"),
            collector_governance::governance::PartyStore::new,
        )?;
        let wing_store = store(
            cfg!(feature = "wing_changes"),
            collector_air::extended::air::WingStore::new,
        )?;
        let air_base_store = store(
            cfg!(feature = "air_base_carry_changes"),
            collector_air::extended::air::RegistrationStore::new,
        )?;
        let mission_store = store(
            cfg!(feature = "mission_changes"),
            collector_air::extended::air::MissionStore::new,
        )?;
        let rail_store = store(
            cfg!(feature = "rail_changes"),
            collector_naval_history::extended::naval_history::RailStore::new,
        )?;
        let state_store = store(cfg!(feature = "territory_changes"), || {
            acquisition_states::StateStore::new(&collector_observers::territory_changes::Observer)
        })?;
        let occupation_store =
            engine_allocation::boxed(acquisition_states::OccupationStore::new()?)?;
        let war_store = store(
            cfg!(feature = "war_changes"),
            acquisition_wars::WarStore::default,
        )?;
        let advisor_store = store(
            cfg!(feature = "advisor_pool_changes"),
            collector_advisors::hired_advisors::AdvisorStore::new,
        )?;
        let local_subunit_store = store(
            cfg!(feature = "local_subunit_changes"),
            collector_model_inputs::model_inputs::LocalStore::default,
        )?;
        let doctrine_store = engine_allocation::shared(Mutex::new(
            collector_slow_tables::doctrines::Store::default(),
        ))?;
        let counter_store = store(
            cfg!(feature = "counter_changes"),
            collector_country_counters::country_counters::CounterStore::observed,
        )?;
        let appearance_store = store(
            cfg!(feature = "appearance_changes"),
            collector_country_counters::country_appearance::AppearanceStore::default,
        )?;
        let frame_priority =
            engine_allocation::shared(collector_workers::frame_priority::Pipeline::new())?;
        let army_stat_store = cfg!(feature = "army_stat_changes")
            .then(|| {
                collector_divisions::extended::division_presentation::held_store::Slot::new(
                    engine_allocation::boxed(
                        collector_division_parts::attack_stats::ArmyStatStore::new(),
                    )?,
                )
            })
            .transpose()?;
        let province_unit_store = store(collector_divisions::province_unit_changes::CARRY, || {
            collector_divisions::extended::province_unit_order::ProvinceUnitStore::new(
                &collector_divisions::province_unit_changes::Observer,
            )
        })?;
        let area_store = store(
            cfg!(feature = "area_changes"),
            engine_native::cprovince::controller_areas::Store::new,
        )?;
        let province_store = store(
            cfg!(feature = "province_changes"),
            engine_native::cprovince::carry::RowStore::new,
        )?;
        let weather_carry =
            engine_allocation::boxed(collector_weather::weather::carry::WeatherCarry::default())?;
        let mut slice_fingerprints = std::array::from_fn(|_| UnsafeCell::new(None));
        for (family, cell) in slice_fingerprints.iter_mut().enumerate() {
            if !matches!(
                family,
                collector_certificates::slicing::AIR_RELATIONS
                    | collector_certificates::slicing::AIR_HISTORIES
            ) {
                *cell.get_mut() = Some(engine_allocation::boxed(
                    collector_certificates::slicing::SliceFingerprints::new(),
                )?);
            }
        }
        let mut division_parts: [_; collector_divisions::division_census::MAX_PARTS - 1] =
            std::array::from_fn(|_| None);
        for part in &mut division_parts {
            *part = Some(engine_allocation::boxed(
                collector_divisions::division_census::Census::new_worker(),
            )?);
        }
        let frame_capacities =
            engine_allocation::shared(crate::frame_capacities::Capacities::new(FrameCapacities {
                core: sizing.core,
                map: sizing.map,
                extended: sizing.extended,
            }))?;
        let pool_bytes = engine_allocation::shared(AtomicU64::new(
            (pool_bytes
                + engine_allocation::SharedBox::<AtomicU64>::BYTES
                + engine_allocation::SharedBox::<collector_workers::frame_priority::Pipeline>::BYTES
                + engine_allocation::SharedBox::<crate::frame_capacities::Capacities>::BYTES)
                as u64,
        ))?;
        let air_wing_parts =
            std::array::from_fn(|_| collector_air::extended::air::AirWingJob::part());
        // SAFETY: `Shared::without_frames` runs during attach or an isolated fixture, priming
        // faction watches before capture or concurrent boundaries can observe them.
        unsafe {
            collector_factions::faction_goal_changes::prime();
            collector_factions::faction_companion_changes::prime();
        }
        if collector_memory::terminal::failed() {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        let shared = Self {
            acquisition: OnceLock::new(),
            acquisition_panics: AtomicU64::new(0),
            #[cfg(feature = "callback_profile")]
            profile: ProfilePhases::new(),
            native_tick_ns: AtomicU64::new(0),
            native_tick_calls: AtomicU64::new(0),
            base,
            trampoline,
            frequency,
            game_score_enabled: sizing.extended.game_score.enabled,
            capture_mode: replay_data::scope::CaptureMode::Full,
            tick_idle: None,
            frames: crate::frame_pool::FramePool::new(),
            frame_capacities,
            history_frames: collector_workers::frame_fence::Frames::default(),
            heap_waits: WaitCounter::new(),
            frame_priority,
            dropped: AtomicU64::new(0),
            pool_bytes,
            recording: AtomicBool::new(true),
            stop_spool: AtomicBool::new(false),
            spool_waker: OnceLock::new(),
            decode_waker: OnceLock::new(),
            roster: UnsafeCell::new(Some(roster)),
            roster_ready: AtomicBool::new(false),
            roster_sampling: AtomicBool::new(false),
            pending: UnsafeCell::new(LossBatch::default()),
            clock_capture: UnsafeCell::new(ClockCapture::default()),
            clock_required: AtomicUsize::new(0),
            clock_frames: AtomicUsize::new(0),
            cadence: UnsafeCell::new(CadenceState::default()),
            slice_fingerprints,
            country_cache: UnsafeCell::new(country_cache),
            raid_air_scores: UnsafeCell::new(Some(raid_air_scores)),
            weather_carry: UnsafeCell::new(Some(weather_carry)),
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
            air_wing_parts: UnsafeCell::new(air_wing_parts),
            air_wing_part_starts: UnsafeCell::new((
                [0; collector_air::extended::air::WING_PARTS - 1],
                0,
            )),
            production_part_starts: UnsafeCell::new((
                [0; collector_production_daily::production_modifiers::MAX_PARTS - 1],
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
            occupation_store: UnsafeCell::new(Some(occupation_store)),
            occupation_store_bytes: AtomicU64::new(0),
            war_store: UnsafeCell::new(war_store),
            war_store_bytes: AtomicU64::new(0),
            army_stat_store,
            army_stat_store_bytes: AtomicU64::new(0),
            division_parts: UnsafeCell::new(division_parts),
            division_part_starts: UnsafeCell::new((
                [0; collector_divisions::division_census::MAX_PARTS - 1],
                0,
            )),
            order_split: UnsafeCell::new(collector_divisions::orders_parts::Split::default()),
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
            doctrine_store,
            doctrine_store_bytes: AtomicU64::new(0),
            counter_store: UnsafeCell::new(counter_store),
            counter_store_bytes: AtomicU64::new(0),
            appearance_store: UnsafeCell::new(appearance_store),
            appearance_store_bytes: AtomicU64::new(0),
            force_next: AtomicBool::new(false),
            refresh_next: AtomicBool::new(false),
            forget_families: std::sync::atomic::AtomicU32::new(0),
            lexical_max: AtomicI32::new(-1),
            fixed_layout: OnceLock::new(),
            fixed_layout_buffer: UnsafeCell::new(fixed_layout_buffer),
            modifier_table: [AtomicU64::new(u64::MAX), AtomicU64::new(u64::MAX)],
            division_mark: AtomicU64::new(collector_new_divisions::new_divisions::UNKNOWN),
            country_cache_swap: Mutex::new(CountryCacheSwap::default()),
            country_cache_cells: AtomicUsize::new(country_cells),
            country_intel,
            template_home,
            naval_combat_home,
            weather_temperature: Mutex::new(Default::default()),
            parked_packs: Mutex::new(Vec::new()),
            parked_packs_bytes: AtomicU64::new(0),
            growing_parked_bytes: AtomicU64::new(0),
            retired_parked: Mutex::new(Vec::new()),
            retired_parked_bytes: AtomicU64::new(0),
            retired_parked_pending: AtomicBool::new(false),
            builder_waker: OnceLock::new(),
                    #[cfg(test)]
            clock_frame_seam: OnceLock::new(),
                };
        Ok(shared)
    }

    /// # Safety
    /// Only the originating session's sizing pass may replace the pending cache. Keep the session
    /// live and exclude competing presizers and teardown.
    pub unsafe fn offer_country_cache(&self, cells: usize) {
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
            let Ok(next) = Cache::try_with_capacity(cells).and_then(engine_allocation::shared)
            else {
                return;
            };
            self.pool_bytes
                .fetch_add(next.allocated_bytes() as u64, Ordering::Relaxed);
            swap.next = Some(next);
        }
    }

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
        let current = unsafe { &mut *self.country_cache.get() };
        swap.retired = Some(std::mem::replace(current, next));
        self.country_cache_cells.store(cells, Ordering::Release);
    }

    fn publish_frame(&self, frame: Box<Captured>, ahead: bool) -> std::io::Result<()> {
        let index = self.frames.vacant()?;
        self.frames.install(index, frame);
        if ahead {
            self.frames.built_ahead(index);
        }
        self.frame_priority.publish();
        let published = self.frames.push(index);
        debug_assert!(published.is_ok());
        Ok(())
    }

    fn publish_frame_claimed(
        &self,
        frame: Box<Captured>,
        ahead: bool,
        claim: crate::frame_pool::GrowthClaim<'_>,
    ) -> std::io::Result<()> {
        let index = self.frames.vacant()?;
        self.frames.install_claimed(index, frame, claim);
        if ahead {
            self.frames.built_ahead(index);
        }
        self.frame_priority.publish();
        let published = self.frames.push(index);
        debug_assert!(published.is_ok());
        Ok(())
    }

    fn parked_and_retired_bytes(&self) -> u64 {
        self.parked_packs_bytes
            .load(Ordering::Relaxed)
            .saturating_add(self.growing_parked_bytes.load(Ordering::Relaxed))
            .saturating_add(self.retired_parked_bytes.load(Ordering::Relaxed))
    }

    pub fn parked_bytes(&self) -> u64 {
        self.parked_and_retired_bytes()
    }

    fn at_frame_limit(&self) -> bool {
        self.frames.at_limit_with_extra(
            crate::frame_pool::MAX_LIVE_FRAMES,
            crate::frame_pool::MAX_LIVE_BYTES,
            self.parked_and_retired_bytes(),
        )
    }

    fn try_claim_frame_growth(&self) -> Option<crate::frame_pool::GrowthClaim<'_>> {
        self.frames.try_claim_growth(
            crate::frame_pool::MAX_LIVE_FRAMES,
            crate::frame_pool::MAX_LIVE_BYTES,
            self.parked_and_retired_bytes(),
        )
    }

    #[inline(never)]
    pub(crate) fn new_frame(&self) -> std::io::Result<Box<Captured>> {
        self.frame_for(&self.frame_capacities.published(), false)
    }

    #[inline(never)]
    pub(crate) fn new_frame_parked(&self) -> std::io::Result<Box<Captured>> {
        self.frame_for(&self.frame_capacities.published(), true)
    }

    fn frame_for(&self, capacities: &FrameCapacities, parked: bool) -> std::io::Result<Box<Captured>> {
        let mut frame = engine_allocation::boxed(
            // SAFETY: The session or isolated fixture owns this frame's observer allocations and
            // excludes teardown.
            unsafe {
                Captured::with_capacities_mode(
                    &capacities.core,
                    &capacities.map,
                    &capacities.extended,
                    parked,
                )
            }?,
        )?;
        if !parked {
            frame.extended.share_country_intel(&self.country_intel);
            frame.extended.share_template_home(&self.template_home);
        }
        frame
            .extended
            .share_naval_combat_home(&self.naval_combat_home);
        frame.prepare_clock(self.clock_required())?;
        if collector_memory::terminal::failed() {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        Ok(frame)
    }

    /// # Safety
    /// Only the session frame builder or isolated fixture may prepare a spare. Ensure the observer
    /// stores remain live throughout its preparation.
    pub unsafe fn build_frame_ahead(&self) -> std::io::Result<()> {
        if self.frames.spool_stalled() {
            return Ok(());
        }
        let Some(claim) = self.try_claim_frame_growth() else {
            return Ok(());
        };
        let frame = self.new_frame_parked()?;
        self.publish_frame_claimed(frame, true, claim)
    }

    /// # Safety
    /// Only the designated builder may register itself once on this session's Shared. Keep it and
    /// its observer stores alive until the builder is stopped and joined.
    pub unsafe fn build_ahead_until(
        &self,
        stop: &AtomicBool,
        follow: impl FnMut(),
    ) -> std::io::Result<()> {
        // SAFETY: The caller owns the builder and the observer storage outlives it.
        unsafe { self.build_ahead_until_observing(stop, follow, |_| {}) }
    }

    /// # Safety
    /// The ownership and lifetime requirements of build_ahead_until apply.
    pub unsafe fn build_ahead_until_observing(
        &self,
        stop: &AtomicBool,
        mut follow: impl FnMut(),
        mut observe: impl FnMut(BuildAhead),
    ) -> std::io::Result<()> {
        let _ = self.builder_waker.set(std::thread::current());
        loop {
            follow();
            if stop.load(Ordering::Acquire) || self.acquisition_failed() {
                return Ok(());
            }
            if crate::frame_pool::builds_ahead(
                self.frames.len(),
                self.frames.spool_stalled() || self.at_frame_limit(),
            ) {
                observe(BuildAhead::Starting);
                // SAFETY: The dedicated builder or owning fixture prepares a spare while observers
                // remain live.
                unsafe { self.build_frame_ahead() }?;
                observe(BuildAhead::Free);
                continue;
            }
            observe(BuildAhead::Parking);
            std::thread::park();
        }
    }

    /// # Safety
    /// Take an hour's frame only during admitted capture or an isolated fixture. Settle the unique
    /// slot claim before another owner can recycle it.
    pub unsafe fn take_frame(&self, grow_ns: &mut u64) -> Option<usize> {
        *grow_ns = 0;
        if self.acquisition_failed() {
            return None;
        }
        use crate::frame_pool::HourFrame;
        match crate::frame_pool::hour_frame(
            // SAFETY: This consumer owns the queue removal for the named pool.
            unsafe { self.frames.pop() },
            self.frames.spool_stalled(),
            self.at_frame_limit(),
        ) {
            HourFrame::Free(index) => {
                self.frames.taken(index);
                self.frame_priority.claim();
                // SAFETY: This stage owns the installed frame slot.
                unsafe { self.frame_stage(index, FrameStage::Callback) };
                if self.frames.len() < crate::frame_pool::AHEAD && !self.at_frame_limit() {
                    self.wake_builder();
                }
                if !self.hold_clock(index) {
                    self.return_frame(index, Return::Idle);
                    return None;
                }
                self.ensure_parked_packs(index);
                return Some(index);
            }
            HourFrame::DropStalled => {
                self.frames.drop_stalled();
                return None;
            }
            HourFrame::DropLimit => {
                self.frames.drop_limit();
                return None;
            }
            HourFrame::Grow => {}
        }
        let Some(claim) = self.try_claim_frame_growth() else {
            self.frames.drop_limit();
            return None;
        };
        self.wake_builder();
        let started = qpc();
        let index = self.frames.vacant().ok()?;
        match self.new_frame_parked() {
            Ok(frame) => self.frames.install_claimed(index, frame, claim),
            Err(_) => {
                self.frames.abandon(index);
                return None;
            }
        }
        let elapsed = self.ns(qpc() - started);
        self.frames.grew(elapsed);
        #[cfg(feature = "callback_profile")]
        self.profile
            .frame_grow_ns
            .fetch_add(elapsed, Ordering::Relaxed);
        *grow_ns = elapsed;
        // SAFETY: This stage owns the installed frame slot.
        unsafe { self.frame_stage(index, FrameStage::Callback) };
        self.ensure_parked_packs(index);
        self.frame_priority.refresh();
        Some(index)
    }

    fn set_parked_packs_bytes(&self, bytes: u64) {
        let previous = self.parked_packs_bytes.swap(bytes, Ordering::Relaxed);
        self.pool_bytes
            .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
    }

    fn park_pack(&self, parked: Box<extended::ParkedPacks>) {
        let retired = if let Ok(mut slots) = self.parked_packs.lock() {
            if slots.is_empty() && engine_allocation::reserve(&mut slots, 1).is_ok() {
                let bytes = parked.allocated_bytes() as u64;
                slots.push(parked);
                self.set_parked_packs_bytes(bytes);
                None
            } else {
                let current: usize = slots.iter().map(|entry| entry.allocated_bytes()).sum();
                self.set_parked_packs_bytes(current as u64);
                Some(parked)
            }
        } else {
            Some(parked)
        };
        if let Some(retired) = retired {
            self.retire_parked_pack(retired);
        }
    }

    fn retire_parked_pack(&self, retired: Box<extended::ParkedPacks>) {
        const MAX_RETIRED_PARKED: usize = 1;
        let bytes = retired.allocated_bytes() as u64;
        let overflow = if let Ok(mut queue) = self.retired_parked.lock() {
            if queue.len() < MAX_RETIRED_PARKED && engine_allocation::reserve(&mut queue, 1).is_ok()
            {
                queue.push(retired);
                self.retired_parked_bytes.fetch_add(bytes, Ordering::Relaxed);
                self.pool_bytes.fetch_add(bytes, Ordering::Relaxed);
                self.retired_parked_pending.store(true, Ordering::Release);
                None
            } else {
                Some(retired)
            }
        } else {
            Some(retired)
        };
        drop(overflow);
    }

    pub fn has_retired_parked(&self) -> bool {
        self.retired_parked_pending.load(Ordering::Acquire)
    }

    pub fn drain_retired_parked(&self) -> bool {
        let drained = match self.retired_parked.lock() {
            Ok(mut queue) => {
                self.retired_parked_pending.store(false, Ordering::Release);
                std::mem::take(&mut *queue)
            }
            Err(_) => return false,
        };
        if drained.is_empty() {
            return false;
        }
        let bytes: u64 = drained
            .iter()
            .map(|entry| entry.allocated_bytes() as u64)
            .sum();
        drop(drained);
        self.retired_parked_bytes.fetch_sub(bytes, Ordering::Relaxed);
        self.pool_bytes.fetch_sub(bytes, Ordering::Relaxed);
        true
    }

    fn ensure_parked_packs(&self, index: usize) {
        // SAFETY: caller exclusively owns the frame slot just taken for capture.
        let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
        if !frame.extended.is_parked() {
            return;
        }
        let taken = self.parked_packs.lock().ok().and_then(|mut slots| {
            let parked = slots.pop()?;
            let remaining: usize = slots.iter().map(|entry| entry.allocated_bytes()).sum();
            self.set_parked_packs_bytes(remaining as u64);
            Some(parked)
        });
        if let Some(parked) = taken {
            frame.extended.install_parked_packs(parked);
        } else {
            let capacities = self.frame_capacities.published();
            frame.extended.hydrate_parked_packs(&capacities.extended);
            frame.extended.share_country_intel(&self.country_intel);
            frame.extended.share_template_home(&self.template_home);
        }
        self.set_frame_bytes(index, frame.allocated_bytes() as u64);
    }

    pub fn grow_parked_packs(&self, capacities: &mut extended::Capacities) {
        struct GrowingGuard<'a> {
            shared: &'a Shared,
            bytes: u64,
        }
        impl Drop for GrowingGuard<'_> {
            fn drop(&mut self) {
                self.shared
                    .growing_parked_bytes
                    .fetch_sub(self.bytes, Ordering::Relaxed);
                self.shared
                    .pool_bytes
                    .fetch_sub(self.bytes, Ordering::Relaxed);
            }
        }
        let mut grown = 0usize;
        loop {
            let (mut parked, guard) = match self.parked_packs.lock() {
                Ok(mut slots) => {
                    if grown >= slots.len() {
                        break;
                    }
                    let parked = slots.remove(0);
                    let bytes = parked.allocated_bytes() as u64;
                    self.growing_parked_bytes
                        .fetch_add(bytes, Ordering::Relaxed);
                    self.pool_bytes.fetch_add(bytes, Ordering::Relaxed);
                    let remaining: usize = slots.iter().map(|entry| entry.allocated_bytes()).sum();
                    self.set_parked_packs_bytes(remaining as u64);
                    (
                        parked,
                        GrowingGuard {
                            shared: self,
                            bytes,
                        },
                    )
                }
                Err(_) => break,
            };
            parked.grow(capacities);
            self.park_pack(parked);
            drop(guard);
            grown += 1;
        }
    }

    /// # Safety
    /// Only this frame pool's spool consumer may publish its stall status and wake the builder.
    pub unsafe fn spool_stalled(&self, stalled: bool) {
        self.frames.set_spool_stalled(stalled);
        if !stalled {
            self.wake_builder();
        }
    }

    fn wake_builder(&self) {
        if let Some(builder) = self.builder_waker.get() {
            builder.unpark();
        }
    }

    fn hold_clock(&self, index: usize) -> bool {
        let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
        let before = frame.clock_bytes();
        let held = frame.prepare_clock(self.clock_required()).is_ok();
        if frame.clock_bytes() != before {
            self.set_frame_bytes(index, frame.allocated_bytes() as u64);
        }
        held
    }

    /// # Safety
    /// Update the stage only for the installed frame that the current pipeline stage owns.
    pub unsafe fn frame_stage(&self, index: usize, stage: FrameStage) {
        self.frames.stage(index, stage, qpc() as u64);
    }

    /// # Safety
    /// Only the active sizing owner may take an idle frame to grow it. Return the slot exactly once
    /// before releasing the sizing pass.
    pub unsafe fn take_idle_frame_for_growth(&self, index: usize) -> bool {
        if !self.frames.take(index, crate::frame_pool::AHEAD) {
            return false;
        }
        self.frame_priority.claim();
        true
    }

    pub fn wake_spool(&self) {
        if let Some(spool) = self.spool_waker.get() {
            spool.unpark();
        }
    }

    pub fn buffers_backlogged(&self) -> bool {
        self.frames.len() <= 1
    }

    pub fn acquisition_failed(&self) -> bool {
        collector_memory::terminal::failed() || self.acquisition_panics.load(Ordering::Acquire) != 0
    }

    pub fn set_frame_bytes(&self, index: usize, bytes: u64) {
        self.frames.set_slot_bytes(index, bytes);
    }

    pub fn pool_owned(&self) -> u64 {
        self.frames
            .slots()
            .map(|slot| slot.bytes.load(Ordering::Relaxed))
            .fold(
                self.pool_bytes
                    .load(Ordering::Relaxed)
                    .wrapping_add(self.frames.slot_bytes()),
                u64::wrapping_add,
            )
    }

    pub fn pool_total(&self) -> u64 {
        self.pool_owned()
            .wrapping_add(collector_air_histories::air_histories::retained_bytes() as u64)
            .wrapping_add(
                self.army_stat_store
                    .as_ref()
                    .map_or(0, |slot| slot.spare_bytes()),
            )
    }

    /// # Safety
    /// The caller must exclusively own the installed slot at index, with no outstanding decode,
    /// spool, sizing or helper borrow, and give up that claim on return to the pool.
    pub unsafe fn prepare_frame_for_reuse(&self, index: usize) {
        let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
        let before = frame.allocated_bytes();
        frame.reset_verdicts();
        frame.verdicts_reset = true;
        frame.extended.release_transient();
        if let Some(parked) = frame.extended.take_parked_packs() {
            self.park_pack(parked);
        }
        let result = frame.prepare_clock(self.clock_required());
        let after = frame.allocated_bytes();
        if after != before
            || self.frames.slot(index).bytes.load(Ordering::Relaxed) == 0
        {
            self.set_frame_bytes(index, after as u64);
        }
        collector_workers::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
        collector_workers::private_heap::PRIVATE_HEAP.trim_large_if_needed();
        if result.is_err() {
            self.recording.store(false, Ordering::Release);
        }
    }

    fn recycle_frame_kept(&self, index: usize) {
        self.return_frame(index, Return::Kept);
    }

    /// # Safety
    /// The caller must exclusively own the installed slot at index, with no outstanding decode,
    /// spool, sizing or helper borrow, and give up that claim on return to the pool.
    pub unsafe fn recycle_frame_prepared(&self, index: usize) {
        // SAFETY: This owner has finished using the frame and no other stage or helper borrows it.
        unsafe { self.prepare_frame_for_reuse(index) };
        // SAFETY: This is the last owner of the installed slot and all other uses have ended.
        unsafe { self.recycle_frame(index) };
    }

    /// # Safety
    /// The caller must exclusively own the installed slot at index, with no outstanding decode,
    /// spool, sizing or helper borrow, and give up that claim on return to the pool.
    pub unsafe fn restore_idle_frame_prepared(&self, index: usize) {
        // SAFETY: This owner has finished using the frame and no other stage or helper borrows it.
        unsafe { self.prepare_frame_for_reuse(index) };
        self.return_frame(index, Return::Idle);
    }

    /// # Safety
    /// The caller must exclusively own the installed slot at index, with no outstanding decode,
    /// spool, sizing or helper borrow, and give up that claim on return to the pool.
    pub unsafe fn recycle_frame(&self, index: usize) {
        // SAFETY: The frame was removed from its slot after every previous use ended.
        unsafe {
            self.free_surplus(
                // SAFETY: This is the last owner of the installed slot and all other uses have ended.
                self.recycle_frame_unfreed(index),
            )
        };
    }

    #[must_use]
    /// # Safety
    /// The caller must exclusively own the installed slot at index, with no outstanding decode,
    /// spool, sizing or helper borrow, and give up that claim on return to the pool.
    pub unsafe fn recycle_frame_unfreed(&self, index: usize) -> Option<Box<Captured>> {
        self.return_frame(index, Return::Newest)
    }

    /// # Safety
    /// Drop only this session's retired frame outside its owning slot, after any users of that
    /// frame have joined.
    pub unsafe fn free_surplus(&self, frame: Option<Box<Captured>>) {
        drop(frame);
    }

    fn return_frame(&self, index: usize, how: Return) -> Option<Box<Captured>> {
        // SAFETY: The spool or refusing capture is the last owner of this frame slot.
        unsafe { self.history_frames.recycle(&self.frames.slot(index).fence) };
        unsafe {
            (&mut *self.frames[index].load(Ordering::Acquire))
                .dropped_before
                .clear();
        }
        // SAFETY: This stage owns the installed frame slot.
        unsafe { self.frame_stage(index, FrameStage::Free) };
        if how == Return::Newest && self.frames.surplus() {
            return Some(
                // SAFETY: The caller owns the installed slot and all other frame users are finished.
                unsafe { self.frames.vacate(index) },
            );
        }
        self.frame_priority.publish();
        let published = if how == Return::Idle {
            self.frames.restore(index)
        } else {
            self.frames.push(index)
        };
        debug_assert!(published.is_ok());
        self.frame_priority.refresh();
        None
    }

    fn take_army_stat_store(
        &self,
    ) -> Option<collector_divisions::extended::division_presentation::held_store::Loan> {
        self.army_stat_store.as_ref().and_then(|slot| slot.take())
    }

    pub(crate) fn return_army_stat_store(
        &self,
        mut store: collector_divisions::extended::division_presentation::held_store::Loan,
    ) {
        store.prepare_return();
        let bytes = store.allocated_bytes() as u64;
        let previous = self.army_stat_store_bytes.swap(bytes, Ordering::Relaxed);
        self.pool_bytes
            .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
        drop(store);
    }

    pub fn ns(&self, ticks: i64) -> u64 {
        ((ticks.max(0) as u128 * 1_000_000_000) / self.frequency as u128) as u64
    }

    /// # Safety
    /// Prepare only unpublished attach state or isolated fixture state using its own image, before
    /// publishing a callback that can use these buffers.
    pub unsafe fn prepare_clock_capture(&mut self, memory: &impl engine_native_memory::Memory) {
        let native = cgame_state::campaign_id_len(memory, self.base).unwrap_or(0);
        self.prepare_clock_len(native);
    }

    pub(crate) fn prepare_clock_len(&mut self, len: usize) {
        let capture = self.clock_capture.get_mut() as *mut ClockCapture;
        // SAFETY: `&mut self`: no hook runs and nothing else holds the clock buffers.
        let _ = engine_allocation::advisory(|| unsafe { self.grow_own_clock(&mut *capture, len) });
        self.grow_free_clocks(len, 0);
        *self.clock_frames.get_mut() = len;
    }

    /// # Safety
    /// Only the spool's exclusive sizing pass may prepare this session's frames while respecting
    /// every slot claim.
    pub unsafe fn prepare_frame_clocks(&self, memory: &impl engine_native_memory::Memory) {
        let native = cgame_state::campaign_id_len(memory, self.base).unwrap_or(0);
        let required = native.max(self.clock_required());
        if required > self.clock_frames.load(Ordering::Acquire) {
            self.grow_free_clocks(required, crate::frame_pool::AHEAD);
            self.clock_frames.store(required, Ordering::Release);
        }
    }

    fn grow_free_clocks(&self, required: usize, keep: usize) {
        for index in 0..self.frames.made() {
            if !self.frames.take(index, keep) {
                continue;
            }
            self.frame_priority.claim();
            #[cfg(test)]
            if let Some(seam) = self.clock_frame_seam.get() {
                seam();
            }
            let frame = unsafe { &mut *self.frames[index].load(Ordering::Acquire) };
            let before = frame.clock_bytes();
            let _ = engine_allocation::advisory(|| frame.prepare_clock(required));
            if frame.clock_bytes() != before {
                self.set_frame_bytes(index, frame.allocated_bytes() as u64);
            }
            self.return_frame(index, Return::Idle);
        }
    }

    /// The capture's own clock buffers grown to hold a `required`-byte campaign id. Each
    /// buffer is checked rather than a remembered length: the loss witnesses trade places with
    /// the frames' as hours pass.
    ///
    /// # Safety
    /// The caller is the running capture, or holds `&mut self`.
    unsafe fn grow_own_clock(
        &self,
        capture: &mut ClockCapture,
        required: usize,
    ) -> std::io::Result<()> {
        let cadence = unsafe { &mut *self.cadence.get() };
        let pending = unsafe { &mut *self.pending.get() };
        let bytes = |capture: &ClockCapture, cadence: &CadenceState, pending: &LossBatch| {
            capture.allocated_bytes()
                + cadence.prev_campaign.allocated_bytes()
                + pending.allocated_bytes()
        };
        let before = bytes(capture, cadence, pending);
        let result = (|| {
            capture.entry.prepare(required)?;
            capture.previous.prepare(required)?;
            cadence.prev_campaign.prepare(required)?;
            pending.prepare(required)
        })();
        self.pool_bytes.fetch_add(
            (bytes(capture, cadence, pending) - before) as u64,
            Ordering::Relaxed,
        );
        result?;
        capture.prepared = capture.prepared.max(required);
        Ok(())
    }

    /// The capture's clock destination, its own buffers grown first when the game holds a
    /// longer campaign id than they do. It takes no lock and waits for nothing: these buffers
    /// are the running capture's alone, and the frame it takes is grown by [`Self::take_frame`]. A
    /// refused growth is the collector's allocation failure; the hour's clock read then fails.
    ///
    /// # Safety
    /// The running capture, which makes no other borrow of the clock buffers while this one lives;
    /// the borrow ends with the hour.
    #[allow(
        clippy::mut_from_ref,
        reason = "the running capture owns the clock buffers alone"
    )]
    unsafe fn prepared_clock_capture(
        &self,
        memory: &impl engine_native_memory::Memory,
    ) -> &mut ClockCapture {
        let required = cgame_state::campaign_id_len(memory, self.base).unwrap_or(0);
        self.clock_required.fetch_max(required, Ordering::Release);
        let capture = unsafe { &mut *self.clock_capture.get() };
        if !collector_memory::terminal::failed() {
            let _ = unsafe { self.grow_own_clock(capture, required) };
        }
        capture
    }

    pub(crate) fn clock_required(&self) -> usize {
        self.clock_required.load(Ordering::Acquire)
    }

    /// # Safety
    /// Prepare only unpublished attach state or isolated fixture state using its own image, before
    /// publishing a callback that can use these buffers.
    pub unsafe fn prepare_roster(&mut self, memory: &impl engine_native_memory::Memory) {
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

    /// # Safety
    /// Captures only: the capture gate keeps observed command-batch writers out of the roster
    /// `memory` reads; after an unproven hook publication, a batch already running then is not
    /// observed. Portrait writers may still run, so a portrait read here would need the capture's
    /// pre-images.
    unsafe fn sample_roster(
        &self,
        memory: &impl engine_native_memory::Memory,
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
            let _reads =
                collector_memory::read_accounting::table(replay_data::rows::TableId::Players);
            let roster = unsafe { (&mut *self.roster.get()).as_mut().unwrap() };
            roster.sample(memory, self.base, hour);
            self.roster_ready.store(roster.sampled(), Ordering::Release);
        }
        self.roster_sampling.store(false, Ordering::Release);
        self.wake_spool();
    }
}

pub fn completed_callback_ticks(base: &AtomicI64, tail: &AtomicI64) -> Option<i64> {
    let base = base.load(Ordering::Acquire);
    let tail = tail.load(Ordering::Acquire);
    (base > 0 && tail > 0).then(|| base.saturating_add(tail))
}

fn forget_unpublished_families(shared: &Shared) {
    let families = shared.forget_families.swap(0, Ordering::AcqRel);
    if families & (1 << collector_certificates::slicing::COMBAT_LOG) != 0 {
        collector_combat_log::combat_log::forget_reads();
    }
    for (family, store) in shared.slice_fingerprints.iter().enumerate() {
        if families & (1 << family) != 0 {
            // SAFETY: one capture at a time, between captures' loans.
            if let Some(store) = unsafe { &mut *store.get() }.as_mut() {
                store.forget();
            }
        }
    }
}

/// Take a sliced family's fingerprint store for this hour's job.
///
/// # Safety
/// Captures only, and the previous capture returned the store.
unsafe fn take_slice_store(
    shared: &Shared,
    family: usize,
) -> Box<collector_certificates::slicing::SliceFingerprints> {
    unsafe { &mut *shared.slice_fingerprints[family].get() }
        .take()
        .expect("sliced family store returned by the previous callback")
}

/// Lend the slow job its stores for this hour.
///
/// # Safety
/// Captures only, and the previous capture's slow job returned its stores.
unsafe fn lend_slow_stores(shared: &Shared) -> crate::extended::SlowStores {
    unsafe {
        crate::extended::SlowStores {
            available: (*shared.slice_fingerprints
                [collector_certificates::slicing::AVAILABLE_ADVISORS]
                .get())
            .take(),
            advisors: (*shared.advisor_store.get()).take(),
            local_subunits: (*shared.local_subunit_store.get()).take(),
            doctrine_bytes: None,
            doctrines: Some(shared.doctrine_store.clone()),
        }
    }
}

/// Return a store [`take_slice_store`] lent.
///
/// # Safety
/// Captures only, in the capture that took it.
unsafe fn return_slice_store(
    shared: &Shared,
    family: usize,
    store: Box<collector_certificates::slicing::SliceFingerprints>,
) {
    unsafe { *shared.slice_fingerprints[family].get() = Some(store) };
}

fn lost_observer_interval() {
    collector_equipment_destruction::equipment_destruction::lost_drain();
    collector_naval_results::naval_result_sources::lost_drain();
    collector_experience_gains::experience_gains::lost_drain();
    // SAFETY: Capture recovery owns recording lost observer intervals for the active session.
    unsafe {
        collector_observers::event_feed_observer::lost_drain();
        collector_operation_outcomes::operation_outcomes::lost_drain();
        collector_politics::politics_changes::invalidate();
        collector_land_events::land_events::fail();
        collector_observers::air_losses::lost_drain();
        collector_observers::building_bombing::lost_drain();
        collector_observers::combat_engagement_refreshes::lost_drain();
        collector_observers::nuclear_strikes::lost_drain();
        collector_observers::market_changes::lost_drain();
        collector_map_events::interval::lost_output();
    }
}

fn drop_owned_frame(shared: &Shared, frame: &mut Captured, index: usize) {
    // SAFETY: The spool settled this frame's event-feed handoff before returning the slot.
    unsafe { frame.extended.discard_event_feed() };
    // SAFETY: This frame's owner is returning its outstanding naval loan before reuse.
    unsafe { frame.map.naval.discard_owned() };
    frame.extended.discard_owned_censuses();
    if let Some(store) = frame.extended.take_settled_army_store() {
        shared.return_army_stat_store(store);
    }
    lost_observer_interval();
    let read = frame.clock.is_some();
    if read {
        collector_combat_log::combat_log::forget_reads();
    }
    frame.extended.drop_air_copies();
    for (family, fingerprints) in shared
        .slice_fingerprints
        .iter()
        .enumerate()
        .filter(|_| read)
    {
        // SAFETY: one capture at a time; every store was returned by this capture.
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
    shared.recycle_frame_kept(index);
}

#[derive(Default)]
struct ClockCapture {
    entry: Owned,
    previous: Owned,
    initialized: bool,
    epoch: u64,
    prepared: usize,
}
impl ClockCapture {
    fn allocated_bytes(&self) -> usize {
        self.entry.allocated_bytes() + self.previous.allocated_bytes()
    }
    fn read(
        &mut self,
        memory: &impl engine_native_memory::Memory,
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
    dispatched: i64,
    phases: HourPhases,
    full: bool,
}

#[cfg(feature = "callback_profile")]
fn callback_profile_counters(shared: &Shared) -> collector_workers::callback_profile::Sample {
    let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
    let profile = &shared.profile;
    collector_workers::callback_profile::Sample {
        frame_grow_ns: load(&profile.frame_grow_ns),
        setup_ns: load(&profile.setup_ns),
        parallel_ns: load(&profile.parallel_ns),
        residual_ns: load(&profile.residual_ns),
        dispatch_ns: load(&profile.dispatch_ns),
        phase_calls: load(&profile.calls),
        segments_ns: profile.segments.each_ref().map(load),
        join_wait_ns: profile.join_wait.each_ref().map(load),
        ..Default::default()
    }
}

/// Contain callback failures without reusing partially restored capture storage.
/// Once a callback panics, subsequent hours record losses until the loader
/// restores the hook; they must not consume more buffers or wait for the lost one.
///
/// # Safety
/// Only the running capture may mutate the pending loss batch.
unsafe fn guarded_acquisition(
    shared: &Shared,
    entry: Option<Clock<&[u8]>>,
    acquire: impl FnOnce(),
) -> bool {
    if collector_memory::terminal::failed() {
        shared.recording.store(false, Ordering::Release);
        return false;
    }
    if !shared.acquisition_failed() {
        match catch_unwind(AssertUnwindSafe(acquire)) {
            Ok(()) if !collector_memory::terminal::failed() => return true,
            Ok(()) => {
                shared.recording.store(false, Ordering::Release);
                return false;
            }
            Err(payload) => {
                std::mem::forget(payload);
                shared.acquisition_panics.fetch_add(1, Ordering::Release);
            }
        }
    }
    lost_observer_interval();
    unsafe { (&mut *shared.pending.get()).push(entry) };
    shared.dropped.fetch_add(1, Ordering::Relaxed);
    false
}

#[cfg(any(test, feature = "test-support"))]
#[doc = "# Safety"]
#[doc = "The caller must hold exclusive access to the shared capture state for this callback."]
pub unsafe fn synthetic_callback(
    shared: &Shared,
    entry: Option<Clock<&[u8]>>,
    acquire: impl FnOnce(),
    original: impl FnOnce(),
) -> bool {
    original();
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Callback);
    collector_memory::terminal::set_context(collector_memory::terminal::Context {
        campaign_id: entry.and_then(|clock| clock.campaign_id.try_into().ok()),
        branch: None,
        load_signal: entry.map(|clock| clock.play_session_started_at),
        failed_hour: entry.map(|clock| clock.hour),
    });
    let complete = unsafe { guarded_acquisition(shared, entry, acquire) };
    collector_memory::terminal::attempted_hour(None);
    complete
}

/// Reads one hour, every scheduled table when `full`. The caller guarantees that no observed
/// command batch writes the simulation until it returns: the game thread is held, or every
/// command-batch writer waits at the capture gate. False when hourly captures are not admitted, so nothing was read; an admitted
/// hour that is not recorded (recording stopped, or the collector's allocation failed) is
/// consumed.
///
/// # Safety
/// One capture at a time, on the game thread or the capture thread. The caller holds the capture
/// gate at the admitted write boundary and excludes every observed command batch for the whole
/// native read. After an unproven publication, a batch already running when the hook was published
/// is not observed; `publication_status` reports that mode, which accepts the gap.
/// Portrait writers are not excluded: the caller holds a `PreImagesCapture` for the whole read, and
/// the portrait reader reads live bytes, then accepts, replaces or withholds them by that log, so
/// live portrait storage is never treated as fixed.
pub(crate) unsafe fn capture(placement: Placement, full: bool) -> bool {
    let hook_entry = qpc();
    let shared = SHARED.load(Ordering::Acquire);
    if shared.is_null() {
        return false;
    }
    // SAFETY: published before any hook is installed and withdrawn after the last capture.
    let shared = unsafe { &*shared };
    let Some(admission) = crate::hourly_admission::admit_hourly_capture() else {
        return false;
    };
    unsafe { record_lost(shared, crate::capture_point::account()) };
    let recording =
        shared.recording.load(Ordering::Acquire) && !collector_memory::terminal::failed();
    if recording {
        if let Some(pool) = shared.acquisition.get() {
            match placement {
                Placement::Held =>
                // SAFETY: Only this session's acquisition scheduling owner changes its policy
                // here.
                unsafe { pool.hold_pace() },
                Placement::Beside(game) => {
                    // SAFETY: Only this session's acquisition scheduling owner changes its policy
                    // here.
                    unsafe { pool.beside_pace(crate::capture_point::beside_readers(), game) };
                    if crate::capture_point::hurried() {
                        // SAFETY: Only this session's acquisition scheduling owner changes its
                        // policy here.
                        unsafe { pool.hold_pace() };
                    }
                }
            }
        }
        unsafe { capture_body(shared, hook_entry, placement, full) };
    }
    drop(admission);
    true
}

/// # Safety
/// Call only for the active session's capture, while its capture resources are live.
pub(crate) unsafe fn hurry() {
    let shared = SHARED.load(Ordering::Acquire);
    if !shared.is_null()
        && let Some(pool) = unsafe { &*shared }.acquisition.get()
    {
        // SAFETY: Only this session's acquisition scheduling owner changes its policy here.
        unsafe { pool.hold_pace() };
    }
}

/// Hours that ended and were never read, recorded as lost before whatever frame comes next.
///
/// # Safety
/// The running capture, or no capture can run.
pub unsafe fn record_lost(shared: &Shared, hours: u32) {
    let pending = unsafe { &mut *shared.pending.get() };
    for _ in 0..hours {
        pending.push(None);
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// # Safety
/// As [`capture`], and `shared` is the published runtime state.
unsafe fn capture_body(shared: &Shared, hook_entry: i64, placement: Placement, full: bool) {
    let _quiescent = collector_primitives::asymmetric::begin_capture_quiescence(|| {
        !collector_timing::hook_timing::any_other_in_flight()
    });
    let history_capture =
    // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
    unsafe { shared.history_frames.capture() };
    let _frame_priority = shared.frame_priority.callback();
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Callback);
    #[cfg(feature = "acquisition_profile")]
    let _reads = collector_memory::read_accounting::Capture::new();
    #[cfg(not(feature = "acquisition_profile"))]
    let _reads = collector_memory::read_accounting::Scope::new();
    let memory = unsafe { LiveMemory::new() };
    #[cfg(feature = "acquisition_profile")]
    let memory = engine_native_memory::memory::CountedMemory::new(memory);
    let world_generation = collector_lifetime::world_lifetime::generation();
    let clock_start = qpc();
    let clock_capture = unsafe { shared.prepared_clock_capture(&memory) };
    let (entry, campaign_epoch) = match clock_capture.read(&memory, shared.base) {
        Ok(value) => (Some(value.0), value.1),
        Err(_) => (None, 0),
    };
    let entry = entry.filter(|_| {
        world_generation != u64::MAX
            && world_generation == collector_lifetime::world_lifetime::generation()
    });
    collector_lifetime::world_lifetime::boundary(world_generation, entry.is_some());
    collector_memory::terminal::set_context(collector_memory::terminal::Context {
        campaign_id: entry.and_then(|clock| clock.campaign_id.try_into().ok()),
        branch: None,
        load_signal: entry.map(|clock| clock.play_session_started_at),
        failed_hour: entry.map(|clock| clock.hour),
    });
    collector_equipment_destruction::equipment_destruction::boundary(
        entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
    );
    collector_naval_results::naval_result_sources::boundary(
        entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
    );
    collector_experience_gains::experience_gains::boundary(
        entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
    );
    // SAFETY: Admitted hourly capture boundary advances air-loss, bombing, and nuclear ledgers.
    unsafe {
        collector_observers::air_losses::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
    }
    collector_fuel_awards::fuel_awards::boundary();
    // SAFETY: Admitted hourly capture boundary advances bombing, engagement-refresh, and nuclear ledgers.
    unsafe {
        collector_observers::building_bombing::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        collector_observers::combat_engagement_refreshes::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
        collector_observers::nuclear_strikes::boundary(
            entry.map(|clock| clock.with_campaign_id(campaign_epoch)),
        );
    }
    let entry_ticks = qpc() - clock_start;
    #[cfg(feature = "callback_profile")]
    let profile_before = callback_profile_counters(shared);
    #[cfg(feature = "callback_profile")]
    collector_workers::callback_profile_state::reset_join();
    #[cfg(feature = "callback_profile")]
    let profile_stamp = collector_workers::callback_profile::Stamp::start();
    let dispatched = qpc();
    collector_workers::hour_stages::begin(dispatched);
    let mut phases = HourPhases {
        entry_ns: shared.ns(dispatched - hook_entry),
        ..HourPhases::default()
    };
    phases.free_frames = u32::try_from(shared.frames.len()).unwrap_or(u32::MAX);
    if phases.free_frames == 0 {
        phases.holders = Some(holders(shared));
    }
    let index =
    // SAFETY: This is the admitted hour or isolated fixture taking exclusive ownership of an
    // installed slot.
    unsafe { shared.take_frame(&mut phases.frame_grow_ns) };
    if let Some(index) = index {
        let slot = shared.frames.slot(index);
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { history_capture.frame(&slot.fence) };
        slot.completed.store(0, Ordering::Relaxed);
        slot.callback_tail.store(0, Ordering::Relaxed);
    }
    let job = Dispatch {
        world_generation,
        campaign_epoch,
        index,
        entry,
        entry_ticks,
        dispatched,
        phases,
        full,
    };
    let priority = (placement == Placement::Held).then(CallbackPriority::raise);
    if unsafe { guarded_acquisition(shared, entry, || acquire(shared, job)) }
        && let Some(index) = index
    {
        shared
            .frames
            .slot(index)
            .callback_tail
            .store(1, Ordering::Release);
    }
    drop(priority);
    collector_memory::terminal::attempted_hour(None);
    #[cfg(feature = "callback_profile")]
    {
        let mut sample = profile_stamp.finish(shared.frequency);
        let after = callback_profile_counters(shared);
        sample.hour = entry.map(|clock| clock.hour);
        sample.frame_grow_ns = after
            .frame_grow_ns
            .saturating_sub(profile_before.frame_grow_ns);
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
        sample.join = collector_workers::callback_profile_state::take_join();
        // SAFETY: The current admitted callback has completed this sample's measurement.
        unsafe { collector_workers::callback_profile::record(sample) };
    }
}

/// # Safety
/// Only the hourly update's detour calls this, on the game thread, with that call's arguments,
/// once attach has published the runtime state.
pub unsafe extern "system" fn tick(state: usize, commands: usize) {
    #[cfg(feature = "worker_sampler")]
    let sampler_callback =
    // SAFETY: Only the current capture callback publishes its own sampler observation.
    unsafe { collector_workers::worker_sampler::callback() };
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::Hourly);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let shared = unsafe { &*SHARED.load(Ordering::Acquire) };
    let recording =
        shared.recording.load(Ordering::Acquire) && !collector_memory::terminal::failed();
    let update = crate::capture_point::update_entered();
    #[cfg(feature = "worker_sampler")]
    drop(sampler_callback);
    let original_begin = qpc();
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(shared.trampoline) };
    let ticking = TICKING.replace(true);
    // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
    // hourly exit.
    unsafe { collector_workers::tick_gate::enter(shared.tick_idle.as_ref()) };
    shared.frame_priority.tick(Some(main_processor()));
    timing.original(|| unsafe { original(state, commands) });
    shared.frame_priority.tick(None);
    // SAFETY: This is the matching native tick exit, with the same live idle event.
    unsafe { collector_workers::tick_gate::leave(shared.tick_idle.as_ref()) };
    TICKING.set(ticking);
    let original_end = qpc();
    if recording {
        shared
            .native_tick_ns
            .fetch_add(shared.ns(original_end - original_begin), Ordering::Relaxed);
        shared.native_tick_calls.fetch_add(1, Ordering::Relaxed);
    }
    crate::capture_point::hour_ended(update);
    drop(timing);
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

pub(crate) fn main_processor() -> Option<u32> {
    use windows_sys::Win32::System::Threading::{
        GetActiveProcessorGroupCount, GetCurrentProcessorNumber,
    };
    unsafe { (GetActiveProcessorGroupCount() == 1).then(|| GetCurrentProcessorNumber()) }
}

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

struct CallbackPriority(Option<i32>);
impl CallbackPriority {
    fn raise() -> Self {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
        };
        const UNKNOWN: i32 = i32::MAX;
        let previous = unsafe { GetThreadPriority(GetCurrentThread()) };
        let raised = previous != UNKNOWN
            && previous < THREAD_PRIORITY_HIGHEST
            && unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } != 0;
        Self(raised.then_some(previous))
    }
}
impl Drop for CallbackPriority {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
        if let Some(previous) = self.0 {
            unsafe { SetThreadPriority(GetCurrentThread(), previous) };
        }
    }
}

/// # Safety
/// The requirements of [`acquire_with_memory`] apply, and the callback holds the live acquisition
/// boundary: no observed command batch writes the simulation until this returns, because the game
/// thread is held in its callback or the capture runs beside it with every command-batch writer
/// waiting at the capture gate. A batch running when an unproven hook was published is not
/// observed. Portrait writers may proceed; the
/// capture's `PreImagesCapture` keeps their pre-images, and the portrait reader validates the live
/// bytes it reads against them.
unsafe fn acquire(shared: &Shared, dispatch: Dispatch<'_>) {
    let memory = unsafe { LiveMemory::new() };
    #[cfg(feature = "acquisition_profile")]
    let memory = engine_native_memory::memory::CountedMemory::new(memory);
    unsafe { acquire_with_memory(shared, dispatch, memory) };
}

/// # Safety
/// This is the only capture running against `shared`, so its capture-only state (the cadence,
/// clocks and stores) has one user. Every frame index in `dispatch` names a live frame this capture
/// holds exclusively, taken with [`Shared::take_frame`], and `memory` stays valid for every read,
/// the workers' included, until this returns. Reading the live game, the boundary requirement of
/// [`acquire`] applies as well.
unsafe fn acquire_with_memory(shared: &Shared, dispatch: Dispatch<'_>, memory: impl Memory) {
    let Dispatch {
        world_generation,
        campaign_epoch,
        index,
        entry,
        entry_ticks,
        dispatched,
        mut phases,
        full: requested,
    } = dispatch;
    let allocator_begin = collector_workers::private_heap::PRIVATE_HEAP.hold();
    let cycles_begin = collector_workers::spool_phases::thread_cycles();
    SLOWEST_JOIN.set(None);

    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        if !std::mem::take(&mut frame.verdicts_reset) {
            frame.reset_verdicts();
        }
        frame.extended.release_country_intel();
        frame.world_generation = world_generation;
        frame.set_clock(entry);
        frame.qpc = dispatched;
        let before = qpc();
        frame.counters = frame.clock.as_ref().map(|clock| {
            save_allocation_counters::read_counters(&memory, shared.base, clock.game_state as usize)
        });
        let clock_reads = qpc() - before;
        let clock_ticks = entry_ticks + clock_reads;
        let mut decode_slot =
        // SAFETY: The capture path holds this installed slot exclusively before publishing its
        // decode job.
        unsafe { decode::reserve(shared, index) };
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
        let mut relations_report = collector_intel::intel::RelationsCopyReport::default();
        let mut characters_report =
            [collector_characters::extended::characters::CharactersCopyReport::default();
                CHARACTER_PARTS];
        let mut projects_report = collector_projects::projects::ProjectsCopyReport::default();
        let mut lexical_report = collector_lexical::lexical::LexicalCopyReport::default();
        let mut divisions_report = DivisionsCopyReport::default();
        let mut air_wing_report = collector_air::extended::air::AirWingCopyReport::default();
        let mut air_stats_report = collector_air::extended::air::AirStatsCopyReport::default();
        let mut naval_report =
            collector_naval_history::extended::naval_history::NavalCopyReport::default();
        let mut players_report = collector_map_capture::map_capture::PlayersCopyReport::default();
        let mut session_report = collector_session::session::SessionCopyReport::default();
        let mut production_rest_report =
            collector_production::production::ProductionRestCopyReport::default();
        let mut construction_rest_report =
            collector_construction::construction::ConstructionRestCopyReport::default();
        let mut economy_rest_report = collector_economy::economy::EconomyRestCopyReport::default();
        let mut air_bases_report = collector_air::extended::air::AirBasesCopyReport::default();
        let mut supremacy_report =
            collector_naval_history::extended::naval_history::SupremacyCopyReport::default();
        let mut sunk_report =
            collector_naval_history::extended::naval_history::SunkCopyReport::default();
        let mut radar_report = collector_air::extended::fog::RadarCopyReport::default();
        let mut intel_report = collector_air::extended::fog::IntelCopyReport::default();
        let mut weather_report = collector_weather::weather::WeatherCopyReport::default();
        let mut capitulation_report = collector_intel::intel::CapitulationCopyReport::default();
        let mut decode_hour = 0;
        let mut decode_force_full = false;
        let mut ts_setup = dispatched;
        let mut ts_parallel = dispatched;
        let mut force_full = false;
        if let Some(clock) = entry {
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
            let continuous = cadence.continuous(&clock);
            let recovery = shared.force_next.swap(false, Ordering::AcqRel);
            let refresh = shared.refresh_next.swap(false, Ordering::AcqRel) | requested;
            forget_unpublished_families(shared);
            phases.full_reasons = cadence.full_reasons(&clock)
                | u8::from(recovery) * FULL_RECOVERY
                | u8::from(refresh) * FULL_REFRESH;
            force_full = phases.full_reasons != 0;
            if refresh {
                // SAFETY: This admitted capture consumes its session's spool retry cause.
                phases.retry_cause = unsafe { collector_workers::spool_phases::retry_cause() }
                    .swap(0, Ordering::Relaxed);
            }
            if !continuous || recovery {
                collector_characters::portrait_changes::invalidate();
                collector_characters::advisor_changes::invalidate();
                collector_observers::opinion_changes::invalidate();
                // SAFETY: Capture discontinuity recovery owns invalidating live observer epochs.
                unsafe {
                    collector_operation_outcomes::operation_outcomes::invalidate();
                    collector_politics::politics_changes::invalidate();
                    collector_map_events::interval::invalidate();
                }
                collector_intel::intel::forget_relation_opinions();
            }
            frame.extended.seal_map_events(clock.hour);
            let acquisition = shared.acquisition.get().expect("initialized before hook");
            let mut batch = acquisition::Batch::new(acquisition);
            let final_inputs_dispatch = batch.final_inputs();
            let mut boundary_request = frame
                .extended
                .take_observer_boundary_request(clock.hour, continuous, recovery);
            boundary_request.market_clock = Some(clock.with_campaign_id(campaign_epoch));
            boundary_request.market_refresh = force_full;
            // SAFETY: the capture's write boundary holds through the boundary job: the game is held,
            // or observed command-batch writers wait at the capture gate, and the batch drains the job before
            // the capture ends and the gate reopens.
            let boundary_pending =
                unsafe { submit_observer_boundary(shared, &mut batch, boundary_request) };
            let state = clock.game_state as usize;
            let cache_begin = qpc();
            shared.install_offered_country_cache();
            let country_snapshot = collector_memory::read_accounting::with_table(
                replay_data::rows::TableId::Countries,
                || {
                    Cache::prepare_shared(
                        unsafe { &mut *shared.country_cache.get() },
                        &memory,
                        shared.base,
                        state,
                    )
                },
            )
            .expect("every country snapshot lease returned before the next callback");
            let cached = country_snapshot.view(&memory);
            frame
                .extended
                .set_load_signal(clock.play_session_started_at);
            frame.extended.set_capture_mode(shared.capture_mode);
            frame.extended.prepare_hour(clock.hour, force_full);
            if let Some(store) = &shared.army_stat_store {
                store.prepare_hour(clock.hour, force_full);
            }
            frame.country_cache_status = cached.seed_status();
            frame.country_cache_entries = cached.cached_entries();
            let cache_prepare_ticks = qpc() - cache_begin;
            let characters_refresh = decode::characters_refresh_hour(clock.hour, force_full);
            let projects_refresh = decode::projects_refresh_hour(clock.hour, force_full);
            let lexical_max =
                engine_sites::at(shared.base, engine_native_memory::cpdx_lexer::MAX_TOKEN_ID_RVA)
                    .and_then(|at| engine_native_memory::read::<i32>(&memory, at))
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
            unsafe { finish_observer_boundary(shared, &mut batch, frame, boundary_pending) };
            ts_setup = qpc();
            let mut segment_prev = ts_setup;
            macro_rules! segment {
                ($index:expr) => {{
                    let now = qpc();
                    let ns = shared.ns(now - segment_prev);
                    phases.segments_ns[$index] += ns;
                    #[cfg(feature = "callback_profile")]
                    shared.profile.segments[$index].fetch_add(ns, Ordering::Relaxed);
                    segment_prev = now;
                }};
            }
            macro_rules! queue {
                ($job:expr) => {{
                    let job = || $job;
                    // SAFETY: batch drains all accepted work before this callback returns.
                    unsafe { queue_acquisition(shared, &mut batch, job) }
                }};
            }
            macro_rules! release {
                () => {{
                    batch.release();
                }};
            }
            let mut presentation = PresentationPending::default();
            macro_rules! collected {
                ($pending:expr, $variant:ident) => {{
                    let pending = $pending;
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
            batch.hold();
            let radar_due = replay_data::cadence::should_walk(
                TableId::RadarCoverage,
                clock.hour,
                force_full,
            );
            let world_scheduled = world_schedule(clock.hour, force_full);
            let construction_active = world_scheduled.1;
            let projects_omitted =
                projects_refresh && !frame.extended.records_any(&decode::PROJECTS);
            let radar_ctx = frame.extended.radar_pilot().radar_copy_context(
                shared.base,
                state,
                clock.hour,
                force_full,
            );
            let intel_ctx = frame
                .extended
                .intel_pilot()
                .intel_copy_context(shared.base, state);
            frame.extended.radar_pilot().radar_raw_mode = radar_due;
            frame.extended.radar_pilot().registrations_raw_mode = true;
            frame.extended.intel_pilot().intel_raw_mode = true;
            frame.extended.projects_pilot().raw_mode = projects_refresh;
            frame.extended.construction_rest_pilot().rest_raw_mode = construction_active;
            let (gated_pending, gated_projects_pending) = out_of_line(|| {
                let mut request = frame.extended.take_gated_request(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    unsafe { take_slice_store(shared, collector_certificates::slicing::PROJECTS) },
                    Some(engine_allocation::SharedBox::clone(&country_snapshot)),
                );
                let projects = request.take_projects();
                let projects = queue!(acquisition::Job::GatedProjects(projects));
                (queue!(acquisition::Job::Gated(request)), projects)
            });
            let (launch_store, state_stores) = unsafe {
                (
                    take_slice_store(shared, collector_certificates::slicing::RAID_LAUNCH),
                    crate::extended::RaidStateStores {
                        states: take_slice_store(
                            shared,
                            collector_certificates::slicing::RAID_STATE,
                        ),
                        buildings: take_slice_store(
                            shared,
                            collector_certificates::slicing::RAID_BUILDINGS,
                        ),
                        routes: take_slice_store(
                            shared,
                            collector_certificates::slicing::TRADE_ROUTES,
                        ),
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
            let countries_pending = queue!(acquisition::Job::Countries(
                frame.extended.take_countries_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    &mut frame.countries,
                    &mut frame.links,
                    (clock.hour, force_full),
                    unsafe { (*shared.appearance_store.get()).take() },
                )
            ));
            let templates_pending = queue!(acquisition::Job::Templates(
                frame.extended.take_templates_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    (clock.hour, force_full),
                )
            ));
            let (division_source_capacity, detail_capacities) =
                frame.extended.division_capture_capacities();
            let divisions_ctx = DivisionsCopyContext {
                base: shared.base,
                game_state: state,
                source_capacity: division_source_capacity,
                detail_capacities,
                details_active: replay_data::cadence::should_walk(
                    TableId::DivisionEquipment,
                    clock.hour,
                    force_full,
                ),
            };
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
            let status_recorded = frame
                .extended
                .records_any(&collector_divisions::extended::division_presentation::STATUS_TABLES);
            let intel_recorded = frame.extended.records_any(&decode::INTEL);
            let mut input_payload = decode::acquire_divisions_job();
            input_payload.slots = (split != 0).then(|| slots(0));
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
                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                context: divisions_ctx,
                payload: input_payload,
            }));
            let mut part_pending: [Option<PendingAcquisition>;
                collector_divisions::division_census::MAX_PARTS - 1] =
                [const { None }; collector_divisions::division_census::MAX_PARTS - 1];
            for (part, pending) in part_pending.iter_mut().enumerate().take(split) {
                if let Some(census) = unsafe { (*shared.division_parts.get())[part].take() } {
                    *pending = Some(queue!(acquisition::Job::DivisionInputParts(
                        acquisition::DivisionPartRequest {
                            countries: engine_allocation::SharedBox::clone(&country_snapshot),
                            base: shared.base,
                            game_state: state,
                            slots: slots(part + 1),
                            census,
                            presentation: Some((Arc::clone(&presentation_inputs), part)),
                        }
                    )));
                }
            }
            let naval = decode::acquire_naval_job();
            unsafe { begin_order_parts(shared, &mut frame.map.orders) };
            let root_pending = queue!(acquisition::Job::OrderRoots(
                acquisition::OrderRootsRequest {
                    base: shared.base,
                    orders: std::mem::replace(
                        &mut frame.map.orders,
                        collector_divisions::orders::Orders::vacant()
                    ),
                    naval,
                }
            ));
            let wing_limits = frame.extended.air_wing_pilot().wing_limits();
            let air_ctx = collector_air::extended::air::AirWingCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
                limits: [
                    frame.map.air_wings.rows.len(),
                    wing_limits[0],
                    wing_limits[1],
                ],
                game_score: shared.game_score_enabled,
            };
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
            air_payload.wing_store = unsafe { (*shared.wing_store.get()).take() };
            air_payload.slots = air_slots(0);
            air_payload.split = air_split != 0;
            let air_pending = queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                context: air_ctx,
                payload: air_payload,
            }));
            let mut air_part_pending: [Option<PendingAcquisition>;
                collector_air::extended::air::WING_PARTS - 1] =
                [const { None }; collector_air::extended::air::WING_PARTS - 1];
            for (part, pending) in air_part_pending.iter_mut().enumerate().take(air_split) {
                let mut payload =
                    std::mem::take(unsafe { &mut (*shared.air_wing_parts.get())[part] });
                payload.slots = air_slots(part + 1);
                payload.split = true;
                *pending = Some(queue!(acquisition::Job::AirWing(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: collector_air::extended::air::AirWingCopyContext {
                        limits: [0; 3],
                        ..air_ctx
                    },
                    payload,
                })));
            }
            let characters_ctx =
                collector_characters::extended::characters::CharactersCopyContext {
                    base: shared.base,
                    game_state: state,
                    force_full,
                    active: decode::CHARACTERS.map(|table| {
                        replay_data::cadence::should_walk(table, clock.hour, force_full)
                    }),
                };
            frame
                .extended
                .characters_pilot()
                .set_character_activity(characters_ctx.active);
            let mut characters_pending: [Option<PendingAcquisition>; CHARACTER_PARTS] =
                [const { None }; CHARACTER_PARTS];
            let character_part =
                |part: usize| -> [bool; collector_characters::extended::characters::N] {
                    std::array::from_fn(|index| {
                        characters_ctx.active[index]
                            && collector_characters::extended::characters::PART_MASKS[part][index]
                    })
                };
            let role_hour = (1..CHARACTER_PARTS)
                .any(|part| character_part(part).iter().any(|&value| value))
                || collector_characters::extended::character_inputs::TABLES
                    .iter()
                    .any(|&table| replay_data::cadence::should_walk(table, clock.hour, force_full));
            let character_part_recorded = |part: usize| {
                decode::CHARACTERS
                    .iter()
                    .zip(collector_characters::extended::characters::PART_MASKS[part])
                    .any(|(&table, member)| {
                        member && replay_data::scope::records(shared.capture_mode, table)
                    })
            };
            let roster_active = character_part(0);
            if characters_refresh && roster_active.iter().any(|&value| value) {
                characters_pending[0] = Some(queue!(acquisition::Job::Characters(
                    acquisition::CharactersRequest {
                        final_inputs: None,
                        context:
                            collector_characters::extended::characters::CharactersCopyContext {
                                active: roster_active,
                                ..characters_ctx
                            },
                        payload: decode::acquire_characters_job(0),
                        roles: if role_hour {
                            collector_characters::extended::characters::Roles::Gather
                        } else {
                            collector_characters::extended::characters::Roles::Independent
                        },
                    }
                )));
            }
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
            let economy_pending = queue!(acquisition::Job::Economy(acquire_economy_request(
                &country_snapshot,
                shared.base,
                state,
                frame.extended.economy_rest_pilot(),
                clock.hour,
                force_full,
            )));
            let research_pending = frame
                .extended
                .records_any(
                    &collector_governance::governance::RESEARCH
                        .map(|index| collector_governance::governance::TABLES[index]),
                )
                .then(|| {
                    let fingerprints = unsafe {
                        &mut *shared.slice_fingerprints[collector_certificates::slicing::RESEARCH]
                            .get()
                    }
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
            let governance_pending = queue!(acquisition::Job::Governance(
                frame.extended.take_governance_request(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    unsafe {
                        take_slice_store(shared, collector_certificates::slicing::TECHNOLOGIES)
                    },
                    unsafe { take_slice_store(shared, collector_certificates::slicing::FOCUS) },
                    unsafe { (*shared.building_store.get()).take() },
                    unsafe { (*shared.leader_store.get()).take() },
                    unsafe { (*shared.party_store.get()).take() },
                )
            ));
            let variants_pending = {
                let request = frame.extended.take_variants_request(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                );
                queue!(acquisition::Job::Variants(request))
            };
            let (mut ordinary_parts_pending, mut ordinary_slack_parts) = out_of_line(|| {
                let parts = frame.extended.take_ordinary_parts(
                    &country_snapshot,
                    shared.base,
                    state,
                    clock.hour,
                    collector_metadata::fixed_layout::task_force_eta_guard(
                        frame.fixed_layout.as_ref(),
                    ),
                    crate::extended::NavalMissionLoan {
                        force_full,
                        fingerprints: unsafe {
                            (*shared.slice_fingerprints
                                [collector_certificates::slicing::NAVAL_MISSIONS]
                                .get())
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
            let slow_stores = unsafe { lend_slow_stores(shared) };
            let slow_pending = queue!(acquisition::Job::Slow(frame.extended.take_slow_request(
                &country_snapshot,
                shared.base,
                state,
                clock.hour,
                force_full,
                slow_stores
            )));
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
            let mut production_part_pending: [Option<PendingAcquisition>;
                collector_production_daily::production_modifiers::MAX_PARTS - 1] =
                [const { None }; collector_production_daily::production_modifiers::MAX_PARTS - 1];
            if production_daily_due {
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
            let war_overview_pending = frame
                .extended
                .records_any(&collector_war_overview::war_overview::TABLES)
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
            let stability_pending = frame
                .extended
                .records_any(&collector_stability::country_stability::TABLES)
                .then(|| {
                    let stability_store = unsafe {
                        (*shared.slice_fingerprints[collector_certificates::slicing::STABILITY]
                            .get())
                        .take()
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
            let (relations_ctx, mut relations_selection) =
                frame.extended.relations_pilot().begin_relations(
                    shared.base,
                    state,
                    clock.hour,
                    force_full,
                    relations_refresh,
                );
            let (intel_residual_request, script_variables_request) =
                frame.extended.take_intel_residual_request(
                    &country_snapshot,
                    collector_intel::intel::CaptureContext {
                        base: shared.base,
                        game_state: state,
                        hour: clock.hour,
                        force_full,
                    },
                    unsafe { (*shared.script_flag_store.get()).take() },
                    unsafe { (*shared.script_variable_store.get()).take() },
                );
            let script_variables_pending =
                queue!(acquisition::Job::ScriptVariables(script_variables_request));
            let intel_residual_pending =
                queue!(acquisition::Job::IntelResidual(intel_residual_request));
            let air_bases_ctx = collector_air::extended::air::AirBasesCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let air_bases_pending = {
                let mut payload = decode::acquire_air_bases_job();
                payload.registration_store = unsafe { (*shared.air_base_store.get()).take() };
                queue!(acquisition::Job::AirBases(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: air_bases_ctx,
                    payload,
                }))
            };
            let goals_recorded = frame
                .extended
                .records_any(&collector_factions::faction_goals::TABLES);
            let companions_recorded = frame
                .extended
                .records_any(&collector_factions::faction_companions::TABLES);
            if !goals_recorded {
                frame.faction_goals.failure =
                    Some(collector_extended_support::outcomes::NOT_RECORDED);
            }
            if !companions_recorded {
                frame
                    .faction_companions
                    .refuse(collector_extended_support::outcomes::NOT_RECORDED);
            }
            let factions_pending =
                queue!(acquisition::Job::Factions(acquisition_factions::Request {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    factions: collector_states::acquisition_gather::Request {
                        base: shared.base,
                        game_state: state,
                        payload: acquisition_factions::Names::take(
                            &mut frame.factions,
                            &mut frame.faction_names
                        ),
                    },
                    members: collector_states::acquisition_gather::Request::take(
                        shared.base,
                        state,
                        &mut frame.faction_members
                    ),
                    goal_store: goals_recorded
                        .then(|| unsafe { (*shared.faction_goal_store.get()).take() })
                        .flatten(),
                    goals: std::mem::take(&mut frame.faction_goals),
                    companion_store: companions_recorded
                        .then(|| unsafe { (*shared.faction_companion_store.get()).take() })
                        .flatten(),
                    companions: std::mem::take(&mut frame.faction_companions),
                    force_full,
                }));
            let players_ctx = collector_map_capture::map_capture::PlayersCopyContext {
                base: shared.base,
                game_state: state,
            };
            let players_pending = {
                queue!(acquisition::Job::Players(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: players_ctx,
                    payload: decode::acquire_players_job(),
                }))
            };
            let air_stats_ctx = collector_air::extended::air::AirStatsCopyContext {
                base: shared.base,
                game_state: state,
                hour: clock.hour,
                force_full,
            };
            let air_stats_due =
                replay_data::cadence::should_walk(TableId::AirStatistics, clock.hour, force_full);
            let air_stats_omitted =
                air_stats_due && !frame.extended.records_any(&decode::AIR_STATS);
            let air_stats_pending = if air_stats_due && !air_stats_omitted {
                Some(queue!(acquisition::Job::AirStats(
                    acquisition::RawRequest {
                        countries: engine_allocation::SharedBox::clone(&country_snapshot),
                        context: air_stats_ctx,
                        payload: decode::acquire_air_stats_job(),
                    }
                )))
            } else {
                frame.extended.air_stats_pilot().stats_raw_mode = air_stats_omitted;
                None
            };
            let supremacy_ctx =
                collector_naval_history::extended::naval_history::SupremacyCopyContext {
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
                        countries: engine_allocation::SharedBox::clone(&country_snapshot),
                        context: supremacy_ctx,
                        payload: decode::acquire_supremacy_job(),
                    }
                )))
            } else {
                frame.extended.supremacy_pilot().supremacy_raw_mode = false;
                None
            };
            let sunk_ctx = collector_naval_history::extended::naval_history::SunkCopyContext {
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
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: sunk_ctx,
                    payload: decode::acquire_sunk_job(),
                })))
            } else {
                frame.extended.sunk_pilot().sunk_raw_mode = sunk_omitted;
                None
            };
            let radar_pending = if radar_due {
                Some(queue!(acquisition::Job::Radar(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: radar_ctx,
                    payload: decode::acquire_radar_job(),
                })))
            } else {
                None
            };
            let projects_ctx = collector_projects::projects::ProjectsCopyContext {
                base: shared.base,
                game_state: state,
                active: projects_refresh,
            };
            let projects_pending = if projects_refresh && !projects_omitted {
                Some(queue!(acquisition::Job::Projects(
                    acquisition::RawRequest {
                        countries: engine_allocation::SharedBox::clone(&country_snapshot),
                        context: projects_ctx,
                        payload: decode::acquire_projects_job(),
                    }
                )))
            } else {
                None
            };
            let lexical_ctx = collector_lexical::lexical::LexicalCopyContext { base: shared.base };
            let lexical_pending = if lexical_refresh {
                Some(queue!(acquisition::Job::Lexical(acquisition::RawRequest {
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: lexical_ctx,
                    payload: decode::acquire_lexical_job(),
                })))
            } else {
                None
            };
            let session_ctx = collector_session::session::SessionCopyContext {
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
                    countries: engine_allocation::SharedBox::clone(&country_snapshot),
                    context: session_ctx,
                    payload: decode::acquire_session_job(),
                })))
            } else {
                frame.extended.session_pilot().raw_mode = session_omitted;
                None
            };
            let raid_targets_pending = frame
                .extended
                .records_any(&collector_projects::raid_target_statuses::TABLES)
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
            let supply_trains_parts = if frame
                .extended
                .records_any(&collector_supply_trains::supply_trains::TABLES)
            {
                SUPPLY_PARTS
            } else {
                0
            };
            for (part, pending) in supply_trains_pending
                .iter_mut()
                .enumerate()
                .take(supply_trains_parts)
            {
                let fingerprints = (part == 0).then(|| unsafe {
                    take_slice_store(shared, collector_certificates::slicing::SUPPLY_TRAINS)
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
            let progress = frame.extended.take_combat_progress();
            let progress_recorded = frame
                .extended
                .records_any(&collector_land_events::extended::combat_progress::TABLES);
            let combat_request =
                // SAFETY: The admitted hour owns this frame and takes its combat job once.
                unsafe { frame
                    .map
                    .take_combat_request(shared.base, state, progress, progress_recorded) };
            let combat_pending = queue!(acquisition::Job::MapCombats(combat_request));
            let scores_rails_pending = queue!(acquisition::Job::ScoresRails({
                let air_scores = unsafe { &mut *shared.raid_air_scores.get() }
                    .take()
                    .expect("raid air score loan");
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
            let mut air_supply_pending = if replay_data::cadence::should_walk(
                TableId::AirGroundCrews,
                clock.hour,
                force_full,
            ) && frame
                .extended
                .records_any(&[TableId::AirGroundCrews])
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
            let mut root_pending = Some(root_pending);
            let mut input_pending = Some(input_pending);
            let mut ship_models_pending = Some(ship_models_pending);
            let mut countries_pending = Some(countries_pending);
            let mut air_pending = Some(air_pending);
            let mut combat_pending = Some(combat_pending);
            let mut order_joins: Option<(
                PendingAcquisition,
                [Option<PendingAcquisition>; collector_divisions::orders_parts::MAX_PARTS - 1],
            )> = None;
            let mut stitch_pending = None;
            let mut stitch_failure = StitchFailureBarrier::default();
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
            let mut ship_detail_pending =
                [const { None }; collector_ship_models::ship_models::MAX_PARTS];
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
            let mut map_ticks = [0; collector_map_capture::map_capture::MAP_TABLES];
            let mut inputs = None;
            let mut division_input_failure = None;
            let mut divisions_failure = None;
            let mut air_failure = None;
            let mut naval_failure = None;
            let mut players_failure = None;
            let mut radar_failure = None;
            let mut projects_failure =
                projects_omitted.then_some(collector_extended_support::outcomes::NOT_RECORDED);
            let mut air_stats_failure =
                air_stats_omitted.then_some(collector_extended_support::outcomes::NOT_RECORDED);
            let mut supremacy_failure = None;
            let mut sunk_failure =
                sunk_omitted.then_some(collector_extended_support::outcomes::NOT_RECORDED);
            let mut world_active = Default::default();
            let mut world_report = acquisition::WorldReport::default();
            let mut world_failure = None;
            let mut economy_failure = None;
            let mut air_stats_raw = None;
            let mut air_bases_raw = Some(true);
            let mut characters_failure = [None; CHARACTER_PARTS];
            frame.extended.characters_pilot().raw_active =
                [false; collector_characters::extended::characters::N];
            macro_rules! join_characters {
                ($part:expr) => {{
                    let part = $part;
                    if let Some(pending) = characters_pending[part].take() {
                        out_of_line(|| {
                            let (request, report, failure) = completed!(pending, Characters);
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
            let mut session_failure =
                session_omitted.then_some(collector_extended_support::outcomes::NOT_RECORDED);
            let mut relations_failure = None;
            let mut final_inputs_pending = None;
            let mut final_sources_done = false;
            let mut final_inputs_armed = false;
            macro_rules! join_final_sources {
                () => {{
                    out_of_line(|| {
                        let (intel_residual_request, intel_residual_report) = collected!(
                            intel_residual_pending
                                .take()
                                .expect("residual Intel producer"),
                            IntelResidual
                        );
                        let flags = frame.extended.restore_intel_residual_request(
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
                    });
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
                            unsafe {
                                *shared.slice_fingerprints[collector_certificates::slicing::RELATIONS].get() =
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
            let mut handling: Option<(EarlyStep, i64)> = None;
            while late_pending.is_none() || final_inputs_pending.is_none() {
                if verdicts_done && divisions_done {
                    stitch_failure.reach(&batch);
                }
                if final_sources_done
                    && late_pending.is_some()
                    && !final_inputs_armed
                    && !batch.stopped()
                    && !collector_memory::terminal::failed()
                {
                    frame.extended.settle_intel_residual(
                        collector_intel::intel::CaptureContext {
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
                        );
                    });
                    final_inputs_armed = true;
                }
                let mut input_handoff = None;
                let ready_begin = qpc();
                if let Some((step, handled)) = handling.take() {
                    let ns = shared.ns(ready_begin - handled);
                    if phases.slowest_step.is_none_or(|(_, slowest)| ns > slowest) {
                        phases.slowest_step = Some((step, ns));
                    }
                }
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
                let handled = qpc();
                let ns = shared.ns(handled - ready_begin);
                if phases.slowest_ready.is_none_or(|(_, slowest)| ns > slowest) {
                    phases.slowest_ready = Some((step, ns));
                }
                handling = Some((step, handled));
                match step {
                    EarlyStep::Presentation => {
                        // SAFETY: the batch owns the ready tickets and this callback holds the live-read
                        // lease; each is a published result or a parked rejection, and neither waits.
                        unsafe { presentation.advance(shared, &mut batch) };
                    }
                    EarlyStep::Roots => {
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
                        let (parts, pending) = out_of_line(|| {
                            let mut orders_request = frame.extended.take_orders_request(
                                &country_snapshot,
                                &mut frame.map.orders,
                                shared.base,
                                state,
                            );
                            let mut order_part_pending: [Option<PendingAcquisition>;
                                collector_divisions::orders_parts::MAX_PARTS - 1] =
                                [const { None }; collector_divisions::orders_parts::MAX_PARTS - 1];
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
                        let naval_ctx =
                            collector_naval_history::extended::naval_history::NavalCopyContext {
                                base: shared.base,
                                game_state: state,
                                hour: clock.hour,
                                force_full,
                            };
                        naval_install_ticks = naval_roots_ticks;
                        naval_job.defer_wings();
                        naval_pending =
                            Some(queue!(acquisition::Job::Naval(acquisition::RawRequest {
                                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                                context: naval_ctx,
                                payload: naval_job,
                            })));
                        order_joins = Some((pending, parts));
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
                            if let Some(store) = unsafe { (*shared.area_store.get()).take() } {
                                request.payload.area_store = Some(store);
                            } else if let Some(store) = request.payload.area_store.as_mut() {
                                store.forget();
                            }
                            if let Some(store) = request.payload.area_store.as_mut() {
                                store.force_full = force_full;
                            }
                            if let Some(store) = unsafe { (*shared.province_store.get()).take() } {
                                request.payload.province_store = Some(store);
                            } else if let Some(store) = request.payload.province_store.as_mut() {
                                store.forget();
                            }
                            if let Some(store) = request.payload.province_store.as_mut() {
                                store.force_full = force_full;
                            }
                            request
                        })));
                        states_pending = Some(queue!(acquisition::Job::States({
                            let mut request = frame.take_states_request(shared.base, state);
                            request.countries =
                                Some(engine_allocation::SharedBox::clone(&country_snapshot));
                            request.store = unsafe { (*shared.state_store.get()).take() };
                            if frame.extended.records_any(&[TableId::StateResistance]) {
                                request.occupation_store =
                                    unsafe { (*shared.occupation_store.get()).take() };
                            }
                            request
                        })));
                        let weather_ctx = collector_weather::weather::WeatherCopyContext {
                            game_state: state,
                            base: shared.base,
                        };
                        weather_pending = Some({
                            let mut payload = decode::acquire_weather_job();
                            // SAFETY: the capture; the previous capture returned it.
                            payload.carry = unsafe { &mut *shared.weather_carry.get() }.take();
                            if let Some(carry) = payload.carry.as_mut() {
                                carry.begin(
                                    clock.hour,
                                    force_full,
                                    collector_observers::country_input_changes::weather_writes(),
                                );
                            }
                            queue!(acquisition::Job::Weather(acquisition::RawRequest {
                                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                                context: weather_ctx,
                                payload,
                            }))
                        });
                        let capitulation_ctx = collector_intel::intel::CapitulationCopyContext {
                            base: shared.base,
                            game_state: state,
                        };
                        capitulation_pending = Some({
                            queue!(acquisition::Job::Capitulation(acquisition::RawRequest {
                                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                                context: capitulation_ctx,
                                payload: decode::acquire_capitulation_job(),
                            }))
                        });
                        wars_pending = Some(queue!(acquisition::Job::Wars({
                            let mut request =
                                frame.take_wars_request(&country_snapshot, shared.base, state);
                            request.store = unsafe { (*shared.war_store.get()).take() };
                            request
                        })));
                        relations_pending = if relations_refresh {
                            Some(queue!(acquisition::Job::Relations(
                                acquisition::RawRequest {
                                    countries: engine_allocation::SharedBox::clone(
                                        &country_snapshot,
                                    ),
                                    context: relations_ctx,
                                    payload: {
                                        let mut payload = decode::acquire_relations_job();
                                        payload.selection = relations_selection.take();
                                        payload.fingerprints = Some(unsafe {
                                            take_slice_store(
                                                shared,
                                                collector_certificates::slicing::RELATIONS,
                                            )
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
                        let mut parts =
                            [const { None }; collector_divisions::division_census::MAX_PARTS - 1];
                        for (part, pending) in
                            std::mem::take(&mut part_pending).into_iter().enumerate()
                        {
                            if let Some(pending) = pending {
                                let (request, _, failure) = completed!(pending, DivisionInputParts);
                                parts[part] = Some((request.census, failure));
                            }
                        }
                        let (joined, join_failure) =
                            input_handoff.restore_master(&mut input_request.payload.census);
                        input_request.payload.presentation = None;
                        input_failure = input_failure.or(join_failure);
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
                                    None => census.absorb(&mut captured),
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
                        let (pending, movement, speed_terms, split, omitted) = out_of_line(|| {
                            let dispatched = input_handoff.take_dispatched();
                            let retained = input_handoff.take_request();
                            drop(input_handoff);
                            if let Some((pending, movement, speed_terms, split)) = dispatched {
                                assert!(retained.is_none());
                                let (ticket, parked) = batch.adopt_presentation_ticket(pending);
                                let pending = if parked {
                                    PendingAcquisition::Rejected(ticket)
                                } else {
                                    PendingAcquisition::Accepted(ticket)
                                };
                                let speed_terms =
                                    queue!(acquisition::Job::PresentationSpeedTerms(speed_terms));
                                let movement =
                                    queue!(acquisition::Job::PresentationMovement(movement));
                                return (Some(pending), movement, Some(speed_terms), split, None);
                            }
                            let mut presentation_request = retained.expect("presentation retained");
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
                            let split = collector_divisions::extended::division_presentation::status_parts_for(
                                    divisions,
                                    acquisition.worker_count(),
                                ) > 1;
                            let presentation_pending = if split {
                                batch.arm_presentation(&mut presentation_request);
                                queue!(acquisition::Job::PresentationPrepare(presentation_request))
                            } else {
                                queue!(acquisition::Job::Presentation(presentation_request))
                            };
                            let speed_terms_pending = queue!(
                                acquisition::Job::PresentationSpeedTerms(speed_terms_request)
                            );
                            let movement_pending =
                                queue!(acquisition::Job::PresentationMovement(movement_request));
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
                                countries: engine_allocation::SharedBox::clone(&country_snapshot),
                                context: intel_ctx,
                                payload: decode::acquire_intel_job(),
                                inputs: inputs.clone(),
                                input_failure: input_failure.or((!inputs_ok)
                                    .then_some("division input acquisition could not complete")),
                            }))
                        });
                        province_unit_order_pending =
                            Some(queue!(acquisition::Job::ProvinceUnitOrder(
                                frame.extended.take_province_unit_order_request(
                                    shared.base,
                                    state,
                                    inputs.clone(),
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
                        naval_ns = frame.map.naval.ticks.map(|cost| shared.ns(cost as i64));
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
                                    store: unsafe { (*shared.character_role_store.get()).take() },
                                },
                            )
                        )));
                        for (part, pending) in characters_pending.iter_mut().enumerate().skip(1) {
                            let active = character_part(part);
                            if characters_refresh && active.iter().any(|&value| value) {
                                if !character_part_recorded(part) {
                                    for (raw, active) in frame
                                        .extended
                                        .characters_pilot()
                                        .raw_active
                                        .iter_mut()
                                        .zip(active)
                                    {
                                        *raw |= active;
                                    }
                                    characters_failure[part] =
                                        Some(collector_extended_support::outcomes::NOT_RECORDED);
                                    continue;
                                }
                                *pending = Some(queue!(acquisition::Job::Characters(
                                    acquisition::CharactersRequest {
                                        final_inputs: Some(Arc::clone(&final_inputs_dispatch)),
                                        context: collector_characters::extended::characters::CharactersCopyContext {
                                            active,
                                            ..characters_ctx
                                        },
                                        payload: decode::acquire_characters_job(part),
                                        roles: role_census.as_ref().map_or(
                                            collector_characters::extended::characters::Roles::Independent,
                                            |census| {
                                                collector_characters::extended::characters::Roles::Borrow(census.clone())
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
                        air_failure = out_of_line(|| {
                            let (mut air_request, report, air_failure) =
                                completed!(air_pending.take().expect("wing producer"), AirWing);
                            let mut air_failure = air_failure.or((!report.raw_ok)
                                .then_some("AirWing owned acquisition could not complete"));
                            air_wing_report = report;
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
                                air_wing_report.copy_ticks[0] += elapsed(begin);
                            }
                            if air_failure.is_none()
                                && let Some(split) = collector_air::extended::air::part_starts(
                                    &air_request.payload,
                                    active,
                                )
                            {
                                unsafe { *shared.air_wing_part_starts.get() = split };
                            }
                            let mut store = air_request.payload.wing_store.take();
                            let bytes = collector_air::extended::air::retained_store_bytes(
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
                            request
                                .payload
                                .supply_sources(&mut frame.extended, divisions_failure);
                            divisions_report.copy_ticks[0] += elapsed(source_started);
                            *divisions_payload = Some(request.payload);
                            frame.extended.set_divisions_raw_mode(true);
                            army_icons_pending = Some(queue!(acquisition::Job::ArmyIcons(
                                frame.extended.take_army_icons_request(shared.base)
                            )));
                            let ns = shared.ns(divisions_report.copy_ticks[0] as i64);
                            frame.table_ns[6] = ns;
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
                            world_failure =
                                Some(collector_extended_support::outcomes::NOT_RECORDED);
                            world_active = world_scheduled;
                        }
                        production_rest_report = world_report.production;
                        construction_rest_report = world_report.construction;
                        players_failure = out_of_line(|| {
                            let (request, report, failure) = completed!(players_pending, Players);
                            let failure = failure.or((!report.raw_ok)
                                .then_some("Players owned acquisition could not complete"));
                            players_report = report;
                            // SAFETY: The admitted frame owner just settled this hour's player job.
                            *unsafe { frame.map.players_raw_mode_mut() } =
                                report.raw_ok || failure.is_some();
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
                            debug_assert!(!report.contained_panic && failure.is_none());
                            unsafe { end_order_parts(shared, &request.orders) };
                            if fronts_recorded {
                                fronts_pending = Some(queue!(acquisition::Job::Fronts(
                                    acquisition::FrontsRequest {
                                        base: shared.base,
                                        fronts: request.plans.take_fronts(),
                                    }
                                )));
                            }
                            let mut retained = Some(acquisition::Job::ChildFronts(request));
                            stitched_orders =
                                Some(batch.park(&mut retained).expect("stitched order loan slot"));
                        });
                    }
                    EarlyStep::Naval => {
                        let (failure, child) = out_of_line(|| {
                            let (mut naval_request, mut report, naval_failure) =
                                completed!(naval_pending.take().expect("naval producer"), Naval);
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
                                air_wing_report.copy_ticks[0] +=
                                // SAFETY: This is the completed air job being restored to its
                                // original frame once.
                                unsafe { frame.map.install_owned_air(wings) };
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
                        let progress =
                        // SAFETY: The admitted hour is settling its own combat request in the
                        // originating frame.
                        unsafe { frame.map.restore_combat_request(
                            combat_request,
                            combat_report,
                            combat_failure,
                        ) };
                        frame
                            .extended
                            .restore_combat_progress(progress, combat_report.progress_ticks);
                        map_ticks =
                        // SAFETY: The admitted hour or isolated fixture owns this map and the
                        // corresponding game image.
                        unsafe { frame.map.sample(
                            &cached,
                            shared.base,
                            state,
                            &mut
                            // SAFETY: These are the admitted hour's own extended combat buffers
                            // and active image.
                             frame.extended.sources(&cached, shared.base) ,
                        ) };
                        if air_missions_pending.is_none() {
                            air_missions_pending = Some(queue!(acquisition::Job::AirMissions(
                                frame.extended.take_air_missions_request(
                                    &country_snapshot,
                                    shared.base,
                                    state,
                                    unsafe { (*shared.mission_store.get()).take() },
                                )
                            )));
                        }
                        map_done = true;
                    }
                    EarlyStep::ChildFronts => {
                        out_of_line(|| {
                            let (mut orders_request, _, child_fronts_failure) = completed!(
                                child_fronts_pending.take().expect("child fronts producer"),
                                ChildFronts
                            );
                            *naval_payload = orders_request.group_source.take();
                            if child_fronts_failure.is_some() {
                                orders_request.plans.fail_child_fronts();
                            }
                            frame.extended.restore_orders_request(
                                orders_request,
                                true,
                                &mut frame.map.orders,
                            );
                        });
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
                        let (scores_rails_request, _, scores_rails_failure) =
                            completed!(scores_rails_pending, ScoresRails);
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
                                unsafe { *shared.rail_store.get() = Some(store) };
                            }
                        });
                        missions_done = true;
                    }
                    EarlyStep::FinalSources => {
                        assert!(
                            final_sources_published(
                                &batch,
                                [
                                    intel_residual_pending.as_ref(),
                                    capitulation_pending.as_ref(),
                                    session_pending.as_ref(),
                                    relations_pending.as_ref(),
                                ],
                            ),
                            "successful final input sources"
                        );
                        join_final_sources!();
                        final_sources_done = true;
                    }
                    EarlyStep::Late => {
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
                        frame.extended.settle_intel_residual(
                            collector_intel::intel::CaptureContext {
                                base: shared.base,
                                game_state: state,
                                hour: clock.hour,
                                force_full,
                            },
                        );
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
                            // SAFETY: the callback still owns the live boundary, and the batch owns
                            // every part's ticket: each is published or a parked rejection.
                            let orders =
                                unsafe { collect_orders(shared, &mut batch, orders, parts) };
                            queue!(acquisition::Job::OrdersStitch(orders))
                        }));
                    }
                }
            }
            if let Some((step, handled)) = handling {
                let ns = shared.ns(qpc() - handled);
                if phases.slowest_step.is_none_or(|(_, slowest)| ns > slowest) {
                    phases.slowest_step = Some((step, ns));
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
            let weather_failure = out_of_line(|| {
                let (mut request, report, failure) = completed!(weather_pending, Weather);
                let failure = failure
                    .or((!report.raw_ok).then_some("Weather owned acquisition could not complete"));
                if let Some(mut carry) = request.payload.carry.take() {
                    if failure.is_some() || request.payload.staging_failed() {
                        carry.forget();
                    }
                    // SAFETY: the capture; the Weather job has joined.
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
            for (total, ticks) in frame.table_ns[10..].iter_mut().zip(map_ticks) {
                *total = shared.ns(ticks as i64);
            }
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
                    unsafe {
                        return_slice_store(
                            shared,
                            collector_certificates::slicing::SUPPLY_TRAINS,
                            store,
                        )
                    };
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
                frame
                    .extended
                    .restore_division_model_inputs_request(request, failure.is_none());
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
            let (slow_request, _, slow_failure) = completed!(slow_pending, Slow);
            let stores = frame
                .extended
                .restore_slow_request(slow_request, slow_failure.is_none());
            unsafe {
                if let Some(store) = stores.available {
                    *shared.slice_fingerprints
                        [collector_certificates::slicing::AVAILABLE_ADVISORS]
                        .get() = Some(store);
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
                unsafe { *shared.local_subunit_store.get() = Some(store) };
            }
            if let Some(bytes) = stores.doctrine_bytes {
                let previous = shared.doctrine_store_bytes.swap(bytes, Ordering::Relaxed);
                shared
                    .pool_bytes
                    .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
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
                    unsafe {
                        *shared.slice_fingerprints[collector_certificates::slicing::STABILITY]
                            .get() = Some(store)
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
            unsafe {
                return_slice_store(
                    shared,
                    collector_certificates::slicing::RAID_LAUNCH,
                    launch_store,
                );
                return_slice_store(
                    shared,
                    collector_certificates::slicing::RAID_STATE,
                    state_stores.states,
                );
                return_slice_store(
                    shared,
                    collector_certificates::slicing::RAID_BUILDINGS,
                    state_stores.buildings,
                );
                return_slice_store(
                    shared,
                    collector_certificates::slicing::TRADE_ROUTES,
                    state_stores.routes,
                );
            };
            let (mut gated_request, _, gated_failure) = completed!(gated_pending, Gated);
            let (projects_request, _, gated_projects_failure) =
                completed!(gated_projects_pending, GatedProjects);
            gated_request.restore_projects(projects_request);
            let project_store = frame.extended.restore_gated_request(
                gated_request,
                gated_failure.is_none() && gated_projects_failure.is_none(),
            );
            // SAFETY: the GatedProjects job owned the store; it has been collected and its request
            // handed the store back through `restore_projects`, so no worker retains it.
            unsafe {
                return_slice_store(
                    shared,
                    collector_certificates::slicing::PROJECTS,
                    project_store,
                )
            };
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
            out_of_line(|| {
                let (governance_request, governance_report) =
                    collected!(governance_pending, Governance);
                let (technologies, focus, building_store, leader_store, party_store) = frame
                    .extended
                    .restore_governance_request(governance_request, governance_report);
                unsafe {
                    *shared.slice_fingerprints[collector_certificates::slicing::TECHNOLOGIES]
                        .get() = Some(technologies);
                    *shared.slice_fingerprints[collector_certificates::slicing::FOCUS].get() =
                        Some(focus);
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
                    unsafe { *shared.leader_store.get() = Some(store) };
                }
                if let Some(store) = party_store {
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared.party_store_bytes.swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    unsafe { *shared.party_store.get() = Some(store) };
                }
            });
            let event_feed_seal_pending = queue!(acquisition::Job::EventFeedSeal(
                frame.extended.take_event_feed_seal()
            ));
            if let Some(pending) = research_pending {
                let (research_request, research_report) = collected!(pending, Research);
                let fingerprints = frame
                    .extended
                    .restore_research_request(research_request, research_report);
                unsafe {
                    *shared.slice_fingerprints[collector_certificates::slicing::RESEARCH].get() =
                        Some(fingerprints)
                };
            } else {
                frame
                    .extended
                    .omit_research(shared.base, state, clock.hour, force_full);
            }
            out_of_line(|| {
                let mut ship_detail_completed: [Option<(
                    acquisition::ShipModelDetailsRequest,
                    Result<collector_ship_models::ship_models::CaptureReport, acquisition::Failure>,
                )>;
                    collector_ship_models::ship_models::MAX_PARTS] =
                    [const { None }; collector_ship_models::ship_models::MAX_PARTS];
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
            let naval_missions = out_of_line(|| {
                let parts = ordinary_parts_pending.map(|pending| {
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
                    *shared.slice_fingerprints[collector_certificates::slicing::NAVAL_MISSIONS]
                        .get() = Some(store);
                }
            }
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
            let mut ticks =
                frame
                    .extended
                    .capture_tables(&cached, shared.base, state, clock.hour, force_full);
            segment!(10);
            let (late_request, _, late_failure) = completed!(late_pending, Late);
            frame.extended.restore_late_request(
                late_request,
                late_failure.is_none(),
                &mut frame.map.orders,
            );
            if let Some(pending) = fronts_pending {
                let (fronts_request, _, fronts_failure) = completed!(pending, Fronts);
                frame
                    .extended
                    .install_fronts(fronts_request.fronts, fronts_failure.is_none());
            }
            out_of_line(|| {
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
            let border_ticks = frame.extended.capture_border_inputs(
                &cached,
                shared.base,
                state,
                (&frame.provinces.rows, frame.provinces_result),
                (&mut frame.countries, &mut frame.countries_result),
            );
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
            out_of_line(|| {
                let (final_request, final_ticks, final_failure) =
                    completed!(final_pending, FinalInputs);
                frame.extended.restore_final_inputs_request(
                    final_request,
                    final_ticks,
                    final_failure,
                );
            });
            out_of_line(|| {
                let (request, report) = collected!(script_variables_pending, ScriptVariables);
                if let Some(store) = frame.extended.restore_script_variables(request, report) {
                    let bytes = store.allocated_bytes() as u64;
                    let previous = shared
                        .script_variable_store_bytes
                        .swap(bytes, Ordering::Relaxed);
                    shared
                        .pool_bytes
                        .fetch_add(bytes.wrapping_sub(previous), Ordering::Relaxed);
                    // SAFETY: the ScriptVariables job has joined; no worker retains this store.
                    unsafe { *shared.script_variable_store.get() = Some(store) };
                }
            });
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
            let intel_failure = match intel_pending {
                None => Some(collector_extended_support::outcomes::NOT_RECORDED),
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

                    drop(request.inputs);
                    failure
                }),
            };
            if let Some(pending) = projects_pending {
                out_of_line(|| {
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
                });
            }
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
                            && collector_characters::extended::characters::PART_MASKS[part][index]
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
            segment!(13);
            let ships = replay_data::rows::TableId::ShipModels.index();
            ticks[ships] = frame.extended.capture_ship_models(&cached, shared.base);
            segment!(14);
            ts_parallel = segment_prev;
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
            if relations_report.raw_ok {
                for (slot, table) in decode::RELATIONS.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(relations_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            for (part, report) in characters_report.iter().enumerate() {
                if !report.raw_ok {
                    continue;
                }
                for (slot, table) in decode::CHARACTERS.iter().enumerate() {
                    if !collector_characters::extended::characters::PART_MASKS[part][slot] {
                        continue;
                    }
                    let index = table.index();
                    let ns = shared.ns(report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if projects_report.raw_ok {
                for (slot, table) in decode::PROJECTS.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(projects_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if lexical_report.raw_ok {
                for (slot, table) in decode::LEXICAL.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(lexical_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if divisions_report.raw_ok {
                for (slot, table) in decode::DIVISIONS.iter().enumerate().skip(1) {
                    let index = table.index();
                    let ns = shared.ns(divisions_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
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
            if air_stats_report.raw_ok {
                for (slot, table) in decode::AIR_STATS.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(air_stats_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if naval_report.raw_ok {
                for (slot, table) in decode::NAVAL_ORDER.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(naval_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if players_report.raw_ok {
                for (slot, table) in decode::PLAYERS.iter().enumerate() {
                    let ns = shared.ns(players_report.copy_ticks[slot] as i64);
                    frame.table_ns[table.index()] = ns;
                }
            }
            if session_report.raw_ok {
                for (slot, table) in decode::SESSION.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(session_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if production_rest_report.raw_ok {
                for (slot, table) in decode::PRODUCTION_REST.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(production_rest_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if construction_rest_report.raw_ok {
                for (slot, table) in decode::CONSTRUCTION_REST.iter().enumerate() {
                    if *table == TableId::StateResistance {
                        continue;
                    }
                    let index = table.index();
                    let ns = shared.ns(construction_rest_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if economy_rest_report.raw_ok {
                for (slot, table) in decode::ECONOMY_REST.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(economy_rest_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if air_bases_report.raw_ok {
                for (slot, table) in decode::AIR_BASES.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(air_bases_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if supremacy_report.raw_ok {
                for (slot, table) in decode::SUPREMACY.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(supremacy_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if sunk_report.raw_ok {
                for (slot, table) in decode::SUNK.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(sunk_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if radar_report.raw_ok {
                for (slot, table) in decode::RADAR.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(radar_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if intel_report.raw_ok {
                for (slot, table) in decode::INTEL.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(intel_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if weather_report.raw_ok {
                for (slot, table) in decode::WEATHER.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(weather_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            if capitulation_report.raw_ok {
                for (slot, table) in decode::CAPITULATION.iter().enumerate() {
                    let index = table.index();
                    let ns = shared.ns(capitulation_report.copy_ticks[slot] as i64);
                    frame.table_ns[index] = ns;
                }
            }
            frame.country_cache_ns = shared.ns(cache_prepare_ticks);
            frame.table_ns[2] += frame.country_cache_ns;
        }
        unsafe {
            shared.sample_roster(&memory, frame.clock.as_ref().map(Clock::borrowed));
        }
        frame.table_ns[0] = shared.ns(clock_ticks);
        #[cfg(feature = "acquisition_profile")]
        collector_memory::read_accounting::snapshot_into(&mut frame.table_reads);
        if frame.world_generation != collector_lifetime::world_lifetime::generation() {
            // SAFETY: The spool settled this frame's event-feed handoff before returning the slot.
            unsafe { frame.extended.discard_event_feed() };
            lost_observer_interval();
            frame.set_clock(None);
            shared.force_next.store(true, Ordering::Release);
        }
        unsafe {
            std::ptr::swap(&mut frame.dropped_before, shared.pending.get());
        }
        frame.pool_bytes = shared.pool_total();
        frame.dropped_frames = shared.dropped.load(Ordering::Relaxed);
        shared
            .frames
            .slot(index)
            .completed
            .store(0, Ordering::Relaxed);
        shared
            .frames
            .slot(index)
            .callback_tail
            .store(0, Ordering::Relaxed);
        let game_elapsed = (qpc() - dispatched).max(1);
        phases.setup_ns = shared
            .ns(ts_setup - dispatched)
            .saturating_sub(phases.frame_grow_ns);
        phases.parallel_ns = shared.ns(ts_parallel - ts_setup);
        phases.residual_ns = shared.ns(dispatched + game_elapsed - ts_parallel);
        phases.slowest_join = SLOWEST_JOIN.take();
        phases.variant = collector_production::production::design_changes::drain_profile();
        phases.variant_drain_ns = shared
            .ns(crate::acquisition_jobs::VARIANT_DRAIN_TICKS.swap(0, Ordering::Relaxed) as i64);
        let allocator = collector_workers::private_heap::PRIVATE_HEAP
            .hold()
            .since(allocator_begin);
        phases.heap_calls = allocator.heap.calls;
        phases.heap_ns = shared.ns(allocator.heap.ticks as i64);
        phases.page_calls = allocator.pages.calls;
        phases.page_ns = shared.ns(allocator.pages.ticks as i64);
        shared.heap_waits.add(phases.heap_ns);
        phases.cycles = collector_workers::spool_phases::cycles_since(cycles_begin);
        #[cfg(feature = "callback_profile")]
        {
            let profile = &shared.profile;
            profile
                .setup_ns
                .fetch_add(shared.ns(ts_setup - dispatched), Ordering::Relaxed);
            profile
                .parallel_ns
                .fetch_add(shared.ns(ts_parallel - ts_setup), Ordering::Relaxed);
            profile.residual_ns.fetch_add(
                shared.ns(dispatched + game_elapsed - ts_parallel),
                Ordering::Relaxed,
            );
            profile.calls.fetch_add(1, Ordering::Relaxed);
        }
        frame.force_full = frame.clock.is_some() && force_full;
        phases.force_full = frame.force_full;
        collector_workers::hour_stages::keep(
            frame.clock.as_ref().map(|clock| clock.hour),
            phases.entry_ns.saturating_add(shared.ns(game_elapsed)),
            frame.force_full,
            &|ticks| shared.ns(ticks),
        );
        frame.phases = phases;
        frame.capture_seq =
        // SAFETY: Only the admitted hour advances this session's decode sequence.
        unsafe { decode::next_seq() };
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
        // SAFETY: This stage owns the installed frame slot.
        unsafe { shared.frame_stage(index, FrameStage::Queued) };
        if
        // SAFETY: This unpublished decode job belongs to the capture's owned frame.
        unsafe { decode::enqueue_reserved(shared, decode_slot) } {
            unsafe { (&mut *shared.pending.get()).clear() }
        } else {
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

fn air_bases_join_failure(failure: Option<&'static str>, raw_ok: bool) -> Option<&'static str> {
    failure.or((!raw_ok).then_some("AirBases owned acquisition could not complete"))
}

fn raw_copy_join_failure(
    failure: Option<&'static str>,
    raw_ok: bool,
    incomplete: &'static str,
) -> Option<&'static str> {
    failure.or((!raw_ok).then_some(incomplete))
}

fn install_air_verdicts(frame: &mut Captured, stats: Option<bool>, bases: Option<bool>) {
    if let Some(raw) = stats {
        frame.extended.air_stats_pilot().stats_raw_mode = raw;
    }
    if let Some(raw) = bases {
        frame.extended.air_bases_pilot().bases_raw_mode = raw;
    }
}

#[inline(never)]
fn out_of_line<R>(work: impl FnOnce() -> R) -> R {
    work()
}

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
    unsafe { submit_retained(shared, batch, &mut retained) }
}

/// # Safety
/// `batch` belongs to this callback, which holds the live-read lease until the batch drains.
unsafe fn submit_acquisition(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    request: acquisition::Job,
) -> PendingAcquisition {
    unsafe { submit_retained(shared, batch, &mut Some(request)) }
}

/// # Safety
/// As [`submit_acquisition`].
unsafe fn submit_retained(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    retained: &mut Option<acquisition::Job>,
) -> PendingAcquisition {
    #[cfg(feature = "callback_profile")]
    let begin = qpc();
    // SAFETY: the caller's batch owns this callback's complete live lease.
    let pending = match unsafe { batch.submit(retained) } {
        Ok(ticket) => PendingAcquisition::Accepted(ticket),
        Err(_) => PendingAcquisition::Rejected(
            batch
                .park(retained)
                .expect("acquisition retains at most MAX_JOBS requests"),
        ),
    };
    #[cfg(feature = "callback_profile")]
    shared
        .profile
        .dispatch_ns
        .fetch_add(shared.ns(qpc() - begin), Ordering::Relaxed);
    #[cfg(not(feature = "callback_profile"))]
    let _ = shared;
    pending
}

/// Collects one join out of line: the completion, with its job-sized request
/// and read counts, lives only in this frame. The hourly body receives the
/// variant's request and report.
///
/// # Safety
/// As `finish_acquisition_into`. `request` must select the variant payload the completion owns
/// exclusively: it is moved out once and the completion is never dropped.
#[inline(never)]
unsafe fn collect_acquisition<T, R>(
    shared: &Shared,
    batch: &mut acquisition::Batch<'_>,
    pending: PendingAcquisition,
    request: impl FnOnce(&acquisition::Job) -> &T,
    report: impl FnOnce(acquisition::Report) -> R,
) -> (T, Result<R, acquisition::Failure>) {
    let mut completed = std::mem::ManuallyDrop::new(None);
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
    unsafe { finish_acquisition_into(shared, batch, pending, &mut completed) };
    completed.expect("every join yields a completion")
}

/// # Safety
/// `batch` owns `pending`'s uncollected ticket, whether [`submit_acquisition`] made it or the batch
/// adopted a worker's child ticket, and the callback still holds the live-read lease. An accepted
/// ticket joins its published completion; a rejected one completes its parked request here without
/// a worker, as a failure for most jobs, or run inline where the request is eligible.
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
            let role = result.request.role() as usize;
            let wait = shared.ns(qpc() - begin);
            if SLOWEST_JOIN.get().is_none_or(|(_, slowest)| wait > slowest) {
                SLOWEST_JOIN.set(Some((role, wait)));
            }
            #[cfg(feature = "callback_profile")]
            {
                collector_workers::callback_profile_state::record_join(role, wait, result.profile);
                shared.profile.join_wait[role].fetch_add(wait, Ordering::Relaxed);
            }
        }
        PendingAcquisition::Rejected(ticket) => unsafe { finish_rejected(batch, ticket, out) },
    }
}

#[derive(Clone, Copy, Debug)]
pub enum EarlyStep {
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
fn pending_complete(batch: &acquisition::Batch<'_>, pending: &PendingAcquisition) -> bool {
    match pending {
        PendingAcquisition::Accepted(ticket) => batch.is_complete(*ticket),
        PendingAcquisition::Rejected(_) => true,
    }
}

fn final_sources_published(
    batch: &acquisition::Batch<'_>,
    producers: [Option<&PendingAcquisition>; 4],
) -> bool {
    producers.into_iter().enumerate().all(|(index, pending)| {
        let Some(pending) = pending else {
            return matches!(index, 2 | 3);
        };
        let PendingAcquisition::Accepted(ticket) = pending else {
            return false;
        };
        let Some(Ok(report)) = batch.completed_report(*ticket) else {
            return false;
        };
        match (index, report) {
            (0, acquisition::Report::IntelResidual(_)) => true,
            (1, acquisition::Report::Capitulation(report)) => report.raw_ok,
            (2, acquisition::Report::Session(report)) => report.raw_ok,
            (3, acquisition::Report::Relations(report)) => report.raw_ok,
            _ => false,
        }
    })
}

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

#[derive(Default)]
struct PresentationPending {
    prepare: Option<PendingAcquisition>,
    parts: [Option<PendingAcquisition>;
        collector_divisions::extended::division_presentation::MAX_STATUS_PARTS],
    prepared: Option<acquisition::Ticket>,
    failure: Option<acquisition::Failure>,
    pending: Option<PendingAcquisition>,
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
            let advance = batch.wait_ready(|batch| {
                if self.ready(batch) {
                    return Some(true);
                }
                pending
                    .is_some_and(|pending| pending_complete(batch, pending))
                    .then_some(false)
            });
            if !advance {
                break;
            }
            // SAFETY: readiness is proved above, a published result or a parked rejection, and
            // the same boundary is held.
            unsafe { self.advance(shared, batch) };
        }
    }

    /// Collect only ready producers, dispatching their consumer immediately.
    /// # Safety
    /// As `wait_for`; `ready` must hold, so each producer's ticket is a published result or a parked
    /// rejection that the batch owns.
    #[inline(never)]
    unsafe fn advance(&mut self, shared: &Shared, batch: &mut acquisition::Batch<'_>) {
        assert!(self.ready(batch), "presentation producer published");
        if let Some(pending) = self.prepare.take() {
            unsafe { self.dispatch_ranges(shared, batch, pending) };
        } else {
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

    /// # Safety
    /// `pending` is this hour's preparation on the callback's `batch`, either published or parked
    /// after a rejection (which completes as a failure), and the callback holds the live-read lease
    /// until the batch drains.
    #[inline(never)]
    unsafe fn dispatch_ranges(
        &mut self,
        shared: &Shared,
        batch: &mut acquisition::Batch<'_>,
        pending: PendingAcquisition,
    ) {
        // SAFETY: this preparation's ticket belongs to the callback's batch and the lease is held;
        // it is published, or parked after a rejection.
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
                                countries: engine_allocation::SharedBox::clone(&request.countries),
                                context: request.context,
                                inputs: request.inputs.clone(),
                                preparation: matches!(
                                    &part,
                                    collector_divisions::extended::division_presentation::StatusWork::Preparation { .. }
                                ),
                                part: Some(part),
                                stitch: None,
                            },
                        )
                    })
                });
            }
            batch.release();
        }
        self.park(batch, request);
    }

    /// # Safety
    /// Every range this dispatch submitted is on the callback's `batch`, and the callback holds the
    /// live-read lease until the batch drains.
    #[inline(never)]
    unsafe fn dispatch_stitch(&mut self, shared: &Shared, batch: &mut acquisition::Batch<'_>) {
        let mut request = self.unpark(batch);
        let worker_stitch = request.dispatch.continuation();
        for (index, pending) in self.parts.iter_mut().enumerate() {
            let Some(pending) = pending.take() else {
                continue;
            };
            out_of_line(|| {
                // SAFETY: the batch owns each range's ticket and the callback holds the live-read
                // lease; a published range is collected here, a parked one completes as a failure.
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
        Result<
            collector_divisions::extended::division_presentation::CaptureReport,
            acquisition::Failure,
        >,
    ) {
        if let Some(mut request) = self.omitted.take() {
            let report = request
                .captured
                .omit_status(collector_extended_support::outcomes::NOT_RECORDED);
            return (request, Ok(report));
        }
        unsafe { self.wait_for(shared, batch, None) };
        if let Some(pending) = self.pending.take() {
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
    parts: [Option<PendingAcquisition>; collector_divisions::orders_parts::MAX_PARTS - 1],
) -> acquisition::OrdersRequest {
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

fn settle_stitch(
    batch: &acquisition::Batch<'_>,
    orders: &mut acquisition::OrdersRequest,
    report: acquisition::StitchReport,
    failure: Option<&'static str>,
    stitch_admissible: bool,
) {
    if report.contained_panic && stitch_admissible {
        batch.stop_for_contained_panic(collector_workers::roles::Role::OrdersStitch);
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
/// Only the capture reads and writes `shared`'s order split.
unsafe fn begin_order_parts(shared: &Shared, orders: &mut collector_divisions::orders::Orders) {
    let split = unsafe { *shared.order_split.get() };
    orders.begin_parts(split, shared.frequency);
}

/// After this hour's stitch joins: the split it planned for the next hour,
/// or one part when it planned none.
///
/// # Safety
/// As [`begin_order_parts`].
unsafe fn end_order_parts(shared: &Shared, orders: &collector_divisions::orders::Orders) {
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
            acquisition::Job::OrdersStitch(orders) => {
                if !orders.orders.failed() {
                    orders.fail("army order worker admission failed");
                }
                acquisition::Report::OrdersStitch(acquisition::StitchReport::default())
            }
            acquisition::Job::OrdersPart(part) => {
                part.rejected = true;
                acquisition::Report::OrdersPart(acquisition::OrdersReport)
            }
            acquisition::Job::AirRelations(relations) => {
                relations
                    .staging
                    .fail("air relation worker admission failed");
                collector_air_relations::air_relations::certificates(relations.part).forget();
                acquisition::Report::AirRelations(acquisition::AirRelationsReport)
            }
            acquisition::Job::AirHistories(histories) => {
                collector_air_histories::air_histories::refuse(&mut histories.staging);
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

/// # Safety
/// `batch` rejected and parked `ticket`'s request before it acquired any live state, and the
/// callback holds the batch's live-read lease.
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
    *out = Some(unsafe { batch.run_inline(request) });
}

pub(crate) fn acquisition_failure_reason(failure: acquisition::Failure) -> &'static str {
    match failure {
        acquisition::Failure::Panicked => "acquisition worker panicked",
        acquisition::Failure::Stopped => "acquisition worker stopped before completion",
    }
}

pub(crate) fn acquire_world_request(
    countries: &engine_allocation::SharedBox<Cache>,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
) -> acquisition::WorldRequest {
    let (production, construction) = world_schedule(hour, force_full);
    acquisition::WorldRequest {
        countries: engine_allocation::SharedBox::clone(countries),
        production: production.then(|| acquisition::WorldPart {
            context: collector_production::production::ProductionRestCopyContext {
                base,
                game_state,
            },
            payload: decode::acquire_production_rest_job(),
        }),
        construction: construction.then(|| acquisition::WorldPart {
            context: collector_construction::construction::ConstructionRestCopyContext {
                base,
                game_state,
            },
            payload: decode::acquire_construction_rest_job(),
        }),
    }
}

fn world_schedule(hour: i32, force_full: bool) -> (bool, bool) {
    (
        replay_data::cadence::should_walk(TableId::ProductionLines, hour, force_full),
        replay_data::cadence::should_walk(TableId::StateBaseResources, hour, force_full),
    )
}

pub(crate) fn acquire_economy_request(
    countries: &engine_allocation::SharedBox<Cache>,
    base: usize,
    game_state: usize,
    economy: &mut collector_economy::economy::Captured,
    hour: i32,
    force_full: bool,
) -> acquisition::EconomyRequest {
    acquisition::EconomyRequest {
        countries: engine_allocation::SharedBox::clone(countries),
        context: economy.resource_copy_context(base, game_state, hour, force_full),
        payload: decode::acquire_economy_rest_job(),
    }
}

pub(crate) fn settle_world(
    extended: &mut extended::Captured,
    world: acquisition::WorldRequest,
    report: &mut acquisition::WorldReport,
    failure: Option<&'static str>,
    production_payload: &mut Option<collector_production::production::ProductionRestJob>,
    construction_payload: &mut Option<collector_construction::construction::ConstructionRestJob>,
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

pub(crate) fn settle_economy(
    economy: &mut collector_economy::economy::Captured,
    request: acquisition::EconomyRequest,
    report: &mut collector_economy::economy::EconomyRestCopyReport,
    failure: Option<&'static str>,
    payload: &mut Option<collector_economy::economy::EconomyRestJob>,
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

pub(crate) fn refuse_economy_rest(
    economy: &mut collector_economy::economy::Captured,
    report: &collector_economy::economy::EconomyRestCopyReport,
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

fn modifier_table_moved(shared: &Shared, memory: &impl engine_native_memory::Memory) -> bool {
    let observed = engine_native_parts::modifier_definition::table(memory, shared.base)
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

fn new_division_mark(
    shared: &Shared,
    memory: &impl Memory,
    daily: bool,
) -> collector_new_divisions::new_divisions::Mark {
    collector_new_divisions::new_divisions::next_mark(
        &shared.division_mark,
        daily,
        engine_sites::at(shared.base, engine_native_memory::save_allocation_counters::NEXT_ID_RVA)
            .and_then(|at| engine_native_memory::read::<i32>(memory, at)),
        engine_sites::at(
            shared.base,
            engine_native_memory::save_allocation_counters::ID_COUNTER_RVA,
        )
        .and_then(|at| engine_native_memory::read::<u32>(memory, at)),
    )
}

fn exported_table_ticks(table: replay_data::rows::TableId, walk: bool, ticks: u64) -> Option<u64> {
    (walk || (table == replay_data::rows::TableId::CountryIntel && ticks != 0)).then_some(ticks)
}

#[cfg(test)]
mod exported_ticks_tests;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod capture_scope_test_kit;

/// # Safety
/// Only the installed player-command callback for this attachment may invoke this function. Shared
/// and its roster storage must remain live until this invocation returns.
pub unsafe fn sample_command_roster(memory: &LiveMemory, state: usize) {
    let shared = SHARED.load(Ordering::Acquire);
    if !shared.is_null() {
        let shared = unsafe { &*shared };
        if !shared.roster_ready.load(Ordering::Acquire) {
            let hour = engine_native_memory::read(memory, state + cgame_state::CURRENT_HOUR);
            shared.sample_content_roster(memory, hour);
        }
    }
}
