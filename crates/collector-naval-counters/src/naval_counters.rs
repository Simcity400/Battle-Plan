use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{collect_vec, copy_vec, owned_vec};
use collector_observers::fleet_region_changes::Certificate as FleetCertificate;
use engine_native::{Memory, cfleet, ctask_force, cunit::FirstCombatMemo, walk::Outcome};
use replay_data::{
    rows::FleetCounterInputRow, rows::FreshTaskForceCounterEta, rows::NavalEngagementTarget,
    rows::NavalPortMissionInputs, rows::NavalSearcher, rows::Recorded, rows::TableId,
    rows::TaskForceCounterInputRow, rows::TaskForceMapMotionRow, rows::TaskForceMissionRow,
    rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::TaskForceCounterInputs,
    TableId::FleetCounterInputs,
    TableId::TaskForceMapMotion,
    TableId::TaskForceMissions,
];

static ARROW_REFUSALS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];

static FRESH_REFUSALS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TASK_GATHER_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn task_gather_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    return !TASK_GATHER_OFF.get();
    #[cfg(not(any(test, feature = "test-support")))]
    true
}

static PHASES: [std::sync::atomic::AtomicU64; 5] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 5];

static WORLD_HOLDS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];

#[derive(Clone, Copy)]
pub struct Holds {
    pub world: Option<u64>,
    pub fleets: ctask_force::Certify,
}

fn unobserved(_: usize) -> Option<(u64, u64)> {
    None
}

impl Holds {
    pub const NONE: Self = Self {
        world: None,
        fleets: unobserved,
    };
}

pub fn phase_data() -> impl serde::Serialize {
    let load = |index: usize| PHASES[index].load(std::sync::atomic::Ordering::Relaxed);
    let held = |index: usize| WORLD_HOLDS[index].load(std::sync::atomic::Ordering::Relaxed);
    let mut frequency = 0;
    let hz = unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
    } != 0;
    let ns = |ticks: u64| {
        (hz && frequency > 0)
            .then(|| (u128::from(ticks) * 1_000_000_000 / frequency as u128) as u64)
    };
    crate::status::status!({
        "captures": load(0),
        "task_forces": load(1),
        "walk_ns": ns(load(2)),
        "missions_ns": ns(load(3)),
        "motion_ns": ns(load(4)),
        "fresh_eta_task_forces": engine_native::ctask_force::FRESH_ETA[0]
            .load(std::sync::atomic::Ordering::Relaxed),
        "fresh_eta_ns": ns(engine_native::ctask_force::FRESH_ETA[1]
            .load(std::sync::atomic::Ordering::Relaxed)),
        "world_holds": {
            "region_ids_held": held(0),
            "region_ids_read": held(1),
            "locations_held": held(2),
            "locations_read": held(3),
            "fleet_leaders_carried": held(4),
            "fleet_leaders_read": held(5),
        },
        "scope": "cumulative since attach: the counter walk's captures, task forces visited (each task force's fields gathered in one guarded call after its identity read), and the whole walk's time with the part spent in the mission reader and the motion reader, of which fresh_eta_ns is the fresh ETA graph (every motion capture since attach, counted in fresh_eta_task_forces); the rest is the fleet and task-force census, counter, cached-name, supply and port-mission reads; world_holds counts mission region ids and fresh ETA location region and terrain answered from those held for the native world occurrence and those read (an unobserved world, a new world or a failed Ordinary family holds none), and fresh ETA fleet leaders served while the fleet's leader-binding certificate and the leader's naval tree certificate, each taken before its read, hold, and those read",
    })
}

pub fn fresh_eta_data() -> impl serde::Serialize {
    crate::status::status!({
        "base_speed_refused": FRESH_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "speed_modifier_refused": FRESH_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "terrain_class_refused": FRESH_REFUSALS[2].load(std::sync::atomic::Ordering::Relaxed),
        "modified_range_refused": FRESH_REFUSALS[3].load(std::sync::atomic::Ordering::Relaxed),
        "exiled_refused": FRESH_REFUSALS[4].load(std::sync::atomic::Ordering::Relaxed),
        "strategic_redeployment_refused": FRESH_REFUSALS[5].load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative component refusals in current-hour task-force counter ETA demand; old motion remains available",
    })
}

pub fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "movement_numerator_refused": ARROW_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "retreat_refused": ARROW_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "withdrawal_refused": ARROW_REFUSALS[2].load(std::sync::atomic::Ordering::Relaxed),
        "naval_transfer_refused": ARROW_REFUSALS[3].load(std::sync::atomic::Ordering::Relaxed),
        "combat_links_refused": ARROW_REFUSALS[4].load(std::sync::atomic::Ordering::Relaxed),
        "cached_name_refused": ARROW_REFUSALS[5].load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local task-force movement, combat-link and cached-name refusals; established counter and motion fields remain available",
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    fleets: usize,
    rosters: usize,
    motion: usize,
    paths: usize,
    missions: usize,
    regions: usize,
    names: usize,
    supply_origins: usize,
    port_searchers: usize,
    port_avoided: usize,
    port_bytes: usize,
    port_answers: usize,
}

collector_primitives::capacity_max!(Capacities {
    task_forces,
    fleets,
    rosters,
    motion,
    paths,
    missions,
    regions,
    names,
    supply_origins,
    port_searchers,
    port_avoided,
    port_bytes,
    port_answers,
});

impl Capacities {
    /// # Safety
    /// Run only against the authenticated session image while fleet-region watches remain live for
    /// the active session (or an isolated test) and `WatchRegistry::release` is excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let countries = engine_native::ccountry::Reader::new(memory, base, state)?;
        let mut demand = *self;
        let mut fleets = 0_usize;
        let mut tasks = 0_usize;
        let mut paths = 0_usize;
        let mut regions = 0_usize;
        let mut names = 0_usize;
        let mut origins = 0_usize;
        let mut port = ctask_force::port_mission::Demand::default();
        let mut refusal = None;
        let add = |total: &mut usize, count: usize| {
            *total = total
                .checked_add(count)
                .ok_or("naval counter sizing overflow")?;
            Ok::<(), &'static str>(())
        };
        cfleet::visit_task_forces(memory, base, &countries, |visit| {
            match visit {
                cfleet::Visit::Fleet { .. } => add(&mut fleets, 1)?,
                cfleet::Visit::TaskForce(task, _) => {
                    add(&mut tasks, 1)?;
                    for offset in [
                        engine_native::cunit::ORDINARY_ROUTE_DATA,
                        engine_native::cunit::ALTERNATE_ROUTE_DATA,
                    ] {
                        let array = task
                            .task
                            .checked_add(offset)
                            .and_then(|at| engine_native::cpdx_array::Array::read(memory, at, 4))
                            .ok_or("task-force route sizing header unreadable or invalid")?;
                        add(&mut paths, array.count as usize)?;
                    }
                    let array = task
                        .task
                        .checked_add(ctask_force::NAVAL_MISSION)
                        .and_then(|root| {
                            root.checked_add(engine_native::cnaval_mission::ACCESSIBLE_REGIONS)
                        })
                        .and_then(|at| engine_native::cpdx_array::Array::read(memory, at, 8))
                        .ok_or("task-force region sizing header unreadable or invalid")?;
                    add(&mut regions, array.count as usize)?;
                    let observed =
                        ctask_force::port_mission::demand(memory, base, task.task, task.kind);
                    add(&mut port.searchers, observed.searchers)?;
                    add(&mut port.avoided, observed.avoided)?;
                    add(&mut port.bytes, observed.bytes)?;
                    add(&mut port.references, observed.references)?;
                    if let Some(reason) = observed.refusal {
                        refusal.get_or_insert(reason);
                    }
                    for result in [
                        ctask_force::cached_name_demand(memory, base, task.task, task.id)
                            .and_then(|count| add(&mut names, count)),
                        engine_native::cunit_supply_inputs::origin_demand(memory, task.task)
                            .map_err(|failure| failure.reason())
                            .and_then(|count| add(&mut origins, count)),
                    ] {
                        if let Err(reason) = result {
                            refusal.get_or_insert(reason);
                        }
                    }
                }
            }
            Ok(())
        })?;
        demand.task_forces = demand.task_forces.max(tasks);
        demand.fleets = demand.fleets.max(fleets);
        // SAFETY: Caller guarantees live fleet-region watches with `release` excluded.
        unsafe {
            collector_observers::fleet_region_changes::demand(fleets);
            collector_observers::fleet_region_changes::presize();
        }
        demand.rosters = demand.rosters.max(tasks);
        demand.motion = demand.motion.max(tasks);
        demand.paths = demand.paths.max(paths);
        demand.missions = demand.missions.max(tasks);
        demand.regions = demand.regions.max(regions);
        demand.names = demand.names.max(names);
        demand.supply_origins = demand.supply_origins.max(origins);
        demand.raise_for_port(port)?;
        *self = demand;
        refusal.map_or(Ok(()), Err)
    }

    pub fn raise_for_names(&mut self, rows: usize, bytes: usize) {
        self.task_forces = self.task_forces.max(rows);
        self.names = self.names.max(bytes);
    }

    pub fn raise_for_supply_origins(&mut self, origins: usize) {
        self.supply_origins = self.supply_origins.max(origins);
    }

    pub fn raise_for_port(
        &mut self,
        demand: engine_native::ctask_force::port_mission::Demand,
    ) -> Result<(), &'static str> {
        self.port_searchers = self.port_searchers.max(demand.searchers);
        self.port_avoided = self.port_avoided.max(demand.avoided);
        self.port_bytes = self.port_bytes.max(demand.bytes);
        let answers = engine_native::ctask_force::port_mission::reference_slots(demand.references)
            .ok_or("port reference sizing overflow")?;
        self.port_answers = self.port_answers.max(answers);
        Ok(())
    }
}

#[derive(Clone)]
struct HeldFleetRegions {
    pointer: usize,
    certificate: Option<FleetCertificate>,
    regions: Option<std::sync::Arc<Vec<u32>>>,
}

fn observe_fleet_regions(
    store: &mut std::collections::BTreeMap<u64, HeldFleetRegions>,
    pointer: usize,
    id: u64,
    before_header: Option<FleetCertificate>,
    mut certificate: impl FnMut() -> Option<FleetCertificate>,
    read: impl FnOnce() -> Result<Vec<u32>, &'static str>,
) -> Option<(usize, usize)> {
    let after_header = certificate();
    let header_certified = before_header.is_some() && before_header == after_header;
    if header_certified
        && store
            .get(&id)
            .is_some_and(|held| held.pointer == pointer && held.certificate == before_header)
    {
        return None;
    }
    let bytes = |held: &HeldFleetRegions| {
        held.regions
            .as_ref()
            .map_or(0, |rows| rows.capacity() * size_of::<u32>())
            + size_of::<HeldFleetRegions>()
    };
    let previous = store.get(&id).map_or(0, bytes);
    let regions = header_certified
        .then(|| read().ok())
        .flatten()
        .map(std::sync::Arc::new);
    let after = certificate();
    let certified = header_certified && before_header == after;
    let regions = certified.then_some(regions).flatten();
    let held = HeldFleetRegions {
        pointer,
        certificate: certified.then_some(after).flatten(),
        regions,
    };
    let next = bytes(&held);
    store.insert(id, held);
    Some((previous, next))
}

pub struct Captured {
    owned_pending: bool,
    fleet_regions: std::collections::BTreeMap<u64, HeldFleetRegions>,
    fleet_region_bytes: usize,
    task_forces: CaptureRows<ctask_force::CounterRow>,
    fleets: CaptureRows<cfleet::CounterRow>,
    rosters: Arena<u64>,
    motion: CaptureRows<ctask_force::MapMotionRow>,
    paths: Arena<u32>,
    combats: FirstCombatMemo,
    missions: CaptureRows<ctask_force::MissionRow>,
    regions: Arena<u32>,
    names: Arena<u8>,
    supply_origins: Arena<engine_native::cunit_supply_inputs::Origin>,
    port_searchers: Arena<ctask_force::port_mission::Searcher>,
    port_avoided: Arena<Option<engine_native::cname_group_member::Span>>,
    port_bytes: Arena<u8>,
    port_answers: Arena<ctask_force::port_mission::ReferenceAnswer>,
    censuses: CensusStore,
    region_ids: ctask_force::WorldIds,
    locations: ctask_force::WorldHold<(u32, usize)>,
    fleet_leaders: ctask_force::FleetLeaders,
}

#[derive(Default)]
struct CensusStore {
    held: Vec<ctask_force::HeldCensus>,
    serve: Vec<ctask_force::HeldCensus>,
    log: Vec<ctask_force::HeldCensus>,
}

fn held_census(
    sorted: &[ctask_force::HeldCensus],
    identity: &ctask_force::CensusIdentity,
) -> Option<ctask_force::HeldCensus> {
    sorted
        .binary_search_by_key(identity, ctask_force::HeldCensus::identity)
        .ok()
        .map(|index| sorted[index])
}

impl CensusStore {
    fn prepare(&mut self) {
        self.serve.clear();
        std::mem::swap(&mut self.held, &mut self.serve);
        self.log.clear();
    }

    fn finish(
        &mut self,
        complete: bool,
        certificate: impl Fn(usize) -> Option<ctask_force::Certificate>,
    ) -> u64 {
        let mut served = 0_u64;
        self.log
            .sort_unstable_by_key(ctask_force::HeldCensus::identity);
        self.log.dedup_by_key(|census| census.identity());
        self.held.clear();
        if complete
            && engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.held, self.log.len())
            })
            .is_ok()
        {
            for census in &self.log {
                if held_census(&self.serve, &census.identity())
                    .is_some_and(|held| held.certificate == census.certificate)
                {
                    served += 1;
                }
                if certificate(census.receiver) == Some(census.certificate) {
                    self.held.push(*census);
                }
            }
        }
        self.serve.clear();
        served
    }

    fn forget(&mut self) {
        self.held.clear();
        self.serve.clear();
        self.log.clear();
    }

    fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.serve.capacity() + self.log.capacity())
            * size_of::<ctask_force::HeldCensus>()
    }
}
impl Captured {
    pub fn vacant() -> Self {
        // SAFETY: Zero fleet capacity neither updates fleet demand nor offers watch storage.
        unsafe { Self::with_capacities(&Capacities::default()) }
    }

    /// # Safety
    /// Allocate only while fleet-region watches remain live for the active session (or an isolated
    /// test) and `WatchRegistry::release` is excluded; `WatchRegistry::offer` atomically swaps
    /// offered tables if frame construction overlaps with sizing.
    pub unsafe fn with_capacities(capacity: &Capacities) -> Self {
        if capacity.fleets != 0 {
            // SAFETY: Caller guarantees live fleet-region watches with `release` excluded.
            unsafe {
                collector_observers::fleet_region_changes::demand(capacity.fleets);
                collector_observers::fleet_region_changes::presize();
            }
        }
        Self {
            owned_pending: false,
            task_forces: CaptureRows::new(capacity.task_forces),
            fleets: CaptureRows::new(capacity.fleets),
            fleet_regions: Default::default(),
            fleet_region_bytes: 0,
            rosters: Arena::new(capacity.rosters),
            motion: CaptureRows::new(capacity.motion),
            paths: Arena::new(capacity.paths),
            combats: FirstCombatMemo::try_new(capacity.motion)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            missions: CaptureRows::new(capacity.missions),
            regions: Arena::new(capacity.regions),
            names: Arena::new(capacity.names),
            supply_origins: Arena::new(capacity.supply_origins),
            port_searchers: Arena::new(capacity.port_searchers),
            port_avoided: Arena::new(capacity.port_avoided),
            port_bytes: Arena::new(capacity.port_bytes),
            censuses: CensusStore::default(),
            port_answers: Arena::new(capacity.port_answers),
            region_ids: Default::default(),
            locations: Default::default(),
            fleet_leaders: Default::default(),
        }
    }
    pub fn forget_censuses(&mut self) {
        self.censuses.forget();
        self.fleet_regions.clear();
        self.fleet_region_bytes = 0;
        self.region_ids.forget();
        self.locations.forget();
        self.fleet_leaders.forget();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_census(&mut self, census: ctask_force::HeldCensus) {
        self.censuses.held.push(census);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn held_censuses(&self) -> usize {
        self.censuses.held.len()
    }

    pub fn reset(&mut self) {
        self.decode_owned();
        self.clear_fresh_owner_demands();
        self.task_forces.reset();
        self.fleets.reset();
        self.rosters.reset();
        self.motion.reset();
        self.paths.reset();
        self.combats.clear();
        self.missions.reset();
        self.regions.reset();
        self.names.reset();
        self.supply_origins.reset();
        self.port_searchers.reset();
        self.port_avoided.reset();
        self.port_bytes.reset();
        self.port_answers.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.task_forces.allocated_bytes()
            + self.fleet_region_bytes
            + self.fleets.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.motion.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.missions.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.names.allocated_bytes()
            + self.supply_origins.allocated_bytes()
            + self.port_searchers.allocated_bytes()
            + self.port_avoided.allocated_bytes()
            + self.port_bytes.allocated_bytes()
            + self.port_answers.allocated_bytes()
            + self.censuses.allocated_bytes()
            + self.region_ids.allocated_bytes()
            + self.locations.allocated_bytes()
            + self.fleet_leaders.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.task_forces.grow(&mut capacity.task_forces);
        self.fleets.grow(&mut capacity.fleets);
        grow_capacity(&mut capacity.rosters, self.rosters.required);
        self.rosters.grow(&mut capacity.rosters);
        self.motion.grow(&mut capacity.motion);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        if self
            .combats
            .reserve(capacity.motion)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        self.missions.grow(&mut capacity.missions);
        grow_capacity(&mut capacity.regions, self.regions.required);
        self.regions.grow(&mut capacity.regions);
        self.names.grow(&mut capacity.names);
        self.supply_origins.grow(&mut capacity.supply_origins);
        self.port_searchers.grow(&mut capacity.port_searchers);
        self.port_avoided.grow(&mut capacity.port_avoided);
        self.port_bytes.grow(&mut capacity.port_bytes);
        self.port_answers.grow(&mut capacity.port_answers);
    }
    /// # Safety
    /// Run only during admitted naval capture (or an isolated fixture) while fleet-region watches
    /// remain live and teardown is excluded.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.capture_at(
            memory,
            base,
            state,
            None,
            Err("fresh task-force modifier layout unverified"),
        )
    }

    #[cfg(test)]
    pub(crate) fn capture_with_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        modifier_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.capture_at(memory, base, state, Some(hour), modifier_guard)
    }

    /// # Safety
    /// Run only during admitted naval capture (or an isolated fixture) while fleet-region watches
    /// remain live and teardown is excluded.
    pub unsafe fn acquire_with_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        modifier_guard: Result<(), &'static str>,
        holds: Holds,
    ) -> [u64; N] {
        self.capture_impl(memory, base, state, Some(hour), modifier_guard, holds, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: Option<i32>,
        modifier_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.capture_impl(
            memory,
            base,
            state,
            hour,
            modifier_guard,
            Holds::NONE,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_impl(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: Option<i32>,
        modifier_guard: Result<(), &'static str>,
        holds: Holds,
        defer_owned: bool,
    ) -> [u64; N] {
        self.clear_fresh_owner_demands();
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.region_ids.begin(holds.world);
        self.locations.begin(holds.world);
        self.fleet_leaders.begin();
        self.combats.clear();
        let tasks = engine_native::cship::SwitchedObject::new(memory, ctask_force::TASK_FIELDS);
        let memory = &tasks;
        let (mut visited, mut missions_ticks, mut motion_ticks) = (0_u64, 0_u64, 0_u64);
        self.censuses.prepare();
        let mut motion = ctask_force::MapMotionCollector::new(
            memory,
            base,
            state,
            &mut self.motion.rows,
            &mut self.paths.first,
            &mut self.combats,
        )
        .with_fresh_modifier_guard(modifier_guard)
        .with_censuses(ctask_force::Censuses {
            held: &self.censuses.serve,
            certify: &crate::naval_tree_changes::sequence,
            log: &mut self.censuses.log,
        })
        .with_world_locations(&mut self.locations)
        .with_fleet_leaders(
            &mut self.fleet_leaders,
            holds.fleets,
            crate::naval_tree_changes::sequence,
        );
        if let Some(hour) = hour {
            motion = motion.with_fresh_hour(hour);
        }
        let mut missions = ctask_force::MissionCollector::new(
            memory,
            base,
            state,
            &mut self.missions.rows,
            &mut self.regions.first,
        )
        .with_region_ids(&mut self.region_ids);
        let mut names = engine_native::cname_group_member::Names {
            bytes: engine_native::walk::Buffer::Growing(&mut self.names.first),
            len: 0,
            required: 0,
        };
        let mut supply = ctask_force::SupplyOrigins {
            origins: engine_native::walk::Buffer::Growing(&mut self.supply_origins.first),
            len: 0,
            required: 0,
        };
        let mut port = hour.map(|hour| {
            ctask_force::port_mission::Collector::new(
                memory,
                base,
                hour,
                &mut self.port_searchers.first,
                &mut self.port_avoided.first,
                &mut self.port_bytes.first,
                &mut self.port_answers.first,
            )
        });
        let captured = cfleet::walk_counter_inputs_with_port_and_fleet_observed(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.rosters.first,
            (
                &mut self.task_forces.rows,
                Some(&mut names),
                Some(&mut supply),
                port.as_mut(),
            ),
            (
                |pointer: usize| {
                    if task_gather_enabled() {
                        tasks.select(pointer);
                    }
                },
                |pointer: usize, id: u64| {
                    let started = qpc();
                    visited += 1;
                    missions.visit(pointer, id);
                    missions_ticks += qpc().saturating_sub(started).max(0) as u64;
                },
            ),
            |pointer, id, _original| {
                let started = qpc();
                motion.visit(pointer, id);
                motion_ticks += qpc().saturating_sub(started).max(0) as u64;
            },
            |pointer, _id| collector_observers::fleet_region_changes::sequence(pointer),
            |pointer, id, header, before_header| {
                if let Some((previous, next)) = observe_fleet_regions(
                    &mut self.fleet_regions,
                    pointer,
                    id,
                    before_header,
                    || collector_observers::fleet_region_changes::sequence(pointer),
                    || cfleet::read_mission_regions(memory, base, header),
                ) {
                    self.fleet_region_bytes = self
                        .fleet_region_bytes
                        .saturating_sub(previous)
                        .saturating_add(next);
                }
            },
        );

        let port = port
            .map(ctask_force::port_mission::Collector::finish)
            .unwrap_or_default();
        self.port_searchers.len = port.searching_len;
        self.port_searchers.required = port.searching_required;
        self.port_avoided.len = port.avoided_len;
        self.port_avoided.required = port.avoided_required;
        self.port_bytes.len = port.bytes_len;
        self.port_bytes.required = port.bytes_required;
        self.port_answers.required =
            ctask_force::port_mission::reference_slots(port.reference_required)
                .unwrap_or(usize::MAX);
        self.names.len = names.len;
        self.names.required = names.required;
        self.supply_origins.len = supply.len;
        self.supply_origins.required = supply.required;
        let total = qpc().saturating_sub(begin).max(0) as u64;
        for (phase, value) in PHASES
            .iter()
            .zip([1, visited, total, missions_ticks, motion_ticks])
        {
            phase.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
        }
        let failed = match captured.task_forces {
            Outcome::Unavailable { reason } => Some(reason),
            _ => None,
        };
        let walked = motion.finish(failed);
        let served = self.censuses.finish(
            matches!(walked.outcome, Outcome::Rows { .. }),
            crate::naval_tree_changes::sequence,
        );
        crate::naval_tree_changes::censuses(served);
        let registered = missions.finish(failed);
        for (total, value) in WORLD_HOLDS.iter().zip([
            self.region_ids.held,
            self.region_ids.read,
            self.locations.held,
            self.locations.read,
            self.fleet_leaders.carried,
            self.fleet_leaders.read,
        ]) {
            total.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
        }
        self.missions.outcome = registered.outcome;
        self.regions.len = registered.regions_len;
        self.regions.required = registered.required_regions;
        self.task_forces.outcome = captured.task_forces;
        self.fleets.outcome = captured.fleets;
        // SAFETY: Admitted naval capture records measured fleet demand for the live session.
        unsafe { collector_observers::fleet_region_changes::demand(self.fleets.outcome.len()) };
        self.rosters.len = captured.roster_len;
        self.rosters.required = captured.roster_required;
        self.motion.outcome = walked.outcome;
        self.paths.len = walked.paths_len;
        self.paths.required = walked.required_paths;
        self.owned_pending = true;
        if !defer_owned {
            self.decode_owned();
        }
        [qpc().saturating_sub(begin).max(0) as u64, 0, 0, 0]
    }

    pub fn decode_owned(&mut self) {
        if !std::mem::take(&mut self.owned_pending) {
            return;
        }
        let mut arrow_refusals = [0_u64; 6];
        let mut fresh_refusals = [0_u64; 6];
        for row in &self.task_forces.rows[..self.task_forces.outcome.len()] {
            if row.name.is_none() {
                arrow_refusals[5] += 1;
            }
        }
        if !matches!(self.fleets.outcome, Outcome::Rows { .. }) {
            self.fleet_regions.clear();
            self.fleet_region_bytes = 0;
        }
        if self.fleet_regions.len() > self.fleets.outcome.len().saturating_mul(2) {
            let live: std::collections::BTreeSet<_> = self.fleets.rows[..self.fleets.outcome.len()]
                .iter()
                .map(|row| row.id)
                .collect();
            self.fleet_regions.retain(|id, _| live.contains(id));
            self.fleet_region_bytes = self
                .fleet_regions
                .values()
                .map(|held| {
                    held.regions
                        .as_ref()
                        .map_or(0, |rows| rows.capacity() * size_of::<u32>())
                        + size_of::<HeldFleetRegions>()
                })
                .sum();
        }
        for row in &self.motion.rows[..self.motion.outcome.len()] {
            if let Some(fresh) = row.fresh {
                for (index, refused) in [
                    fresh.base_speed.is_err(),
                    fresh.speed_modifier.is_err(),
                    fresh.terrain_class.is_err(),
                    fresh.modified_range.is_err(),
                    fresh.exiled.is_err(),
                    fresh.strategic_redeployment.is_err(),
                ]
                .into_iter()
                .enumerate()
                {
                    if refused {
                        fresh_refusals[index] += 1;
                    }
                }
            }
            for (index, missing) in [
                row.movement_numerator.is_none(),
                row.retreat.is_none(),
                row.withdrawal.is_none(),
                row.naval_transfer.is_none(),
                row.combat_links.is_none(),
            ]
            .into_iter()
            .enumerate()
            {
                if missing {
                    arrow_refusals[index] += 1;
                }
            }
        }
        for (counter, refused) in FRESH_REFUSALS.iter().zip(fresh_refusals) {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        for (counter, refused) in ARROW_REFUSALS.iter().zip(arrow_refusals) {
            if refused != 0 {
                counter.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    fn supply_origins(
        &self,
        row: &ctask_force::CounterRow,
    ) -> Option<&[engine_native::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .first
            .get(..self.supply_origins.len)?
            .get(row.supply_origins_start..end)
    }

    fn port_inputs(&self, row: &ctask_force::CounterRow) -> Option<NavalPortMissionInputs> {
        let input = row.port_mission?;
        let span = |(start, len): engine_native::cname_group_member::Span| {
            start.checked_add(len).map(|end| start..end)
        };
        let searching = input.searching.and_then(|range| {
            let records = self
                .port_searchers
                .first
                .get(..self.port_searchers.len)?
                .get(span(range)?)?;
            Some(owned_vec(collect_vec(records.iter().map(|entry| {
                NavalSearcher {
                    reference: entry.reference,
                    resolves: entry.resolves,
                    roster_owner: entry.roster_owner,
                }
            }))))
        });
        let avoided_ship_types = input.avoided_ship_types.and_then(|range| {
            let records = self
                .port_avoided
                .first
                .get(..self.port_avoided.len)?
                .get(span(range)?)?;
            Some(owned_vec(collect_vec(records.iter().map(|entry| {
                let bytes = self
                    .port_bytes
                    .first
                    .get(..self.port_bytes.len)?
                    .get(span((*entry)?)?)?;
                Some(owned_vec(copy_vec(bytes)))
            }))))
        });
        Some(NavalPortMissionInputs {
            sample_hour: replay_data::rows::PacketHour::Host,
            searching,
            engagement_target: input.engagement_target.map(|target| NavalEngagementTarget {
                reference: target.reference,
                resolves: target.resolves,
            }),
            avoided_ship_types,
        })
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.task_forces.outcome,
            self.fleets.outcome,
            self.motion.outcome,
            self.missions.outcome,
        ]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fresh_owner_demands(
        &self,
    ) -> impl Iterator<Item = (u32, Result<engine_native::ccountry::Country, &'static str>)> + '_
    {
        self.motion.rows[..self.motion.outcome.len().min(self.motion.rows.len())]
            .iter()
            .filter_map(|row| row.fresh_owner.map(|owner| (row.logical_owner, owner)))
    }

    pub fn clear_fresh_owner_demands(&mut self) {
        for row in &mut self.motion.rows {
            row.fresh_owner = None;
        }
    }

    pub fn tables(&self) -> [TableView; N] {
        let [tasks, fleets, ..] = self.outcomes();
        let task_table = || {
            table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, |row| {
                task(
                    row,
                    self.names.first.get(..self.names.len),
                    self.supply_origins(row),
                    self.port_inputs(row),
                )
            })
        };
        let motion = self.motion_table();
        let missions = self.mission_table();
        let Some(rosters) = self.rosters.first.get(..self.rosters.len) else {
            return [
                task_table(),
                unavailable("fleet roster arena length invalid"),
                motion,
                missions,
            ];
        };
        let fleet_rows = &self.fleets.rows[..fleets.len().min(self.fleets.rows.len())];
        if fleet_rows.iter().any(|r| {
            r.start
                .checked_add(r.len)
                .is_none_or(|end| end > rosters.len())
        }) {
            return [
                task_table(),
                unavailable("fleet roster lies outside captured arena"),
                motion,
                missions,
            ];
        }
        [
            task_table(),
            table::<tables::FleetCounterInputs, _>(fleets, &self.fleets.rows, |r| {
                FleetCounterInputRow {
                    fleet: r.id,
                    red: r.rgba[0],
                    green: r.rgba[1],
                    blue: r.rgba[2],
                    alpha: r.rgba[3],
                    mission_region_count: r.mission_region_count,
                    mission_regions: (self
                        .fleet_regions
                        .get(&r.id)
                        .and_then(|held| held.regions.clone()))
                    .into(),
                    task_forces: owned_vec(copy_vec(&rosters[r.start..r.start + r.len])),
                }
            }),
            motion,
            missions,
        ]
    }

    fn mission_table(&self) -> TableView {
        let Some(regions) = self.regions.first.get(..self.regions.len) else {
            return unavailable("task-force mission region arena length invalid");
        };
        let rows = &self.missions.rows[..self.missions.outcome.len().min(self.missions.rows.len())];
        if rows.iter().any(|row| {
            row.regions_start
                .checked_add(row.regions_len)
                .is_none_or(|end| end > regions.len())
        }) {
            return unavailable("task-force mission regions lie outside the captured arena");
        }
        table::<tables::TaskForceMissions, _>(self.missions.outcome, &self.missions.rows, |r| {
            TaskForceMissionRow {
                task_force: r.id,
                owner: r.owner,
                current_region: r.current_region,
                regions: owned_vec(copy_vec(
                    &regions[r.regions_start..r.regions_start + r.regions_len],
                )),
            }
        })
    }

    fn motion_table(&self) -> TableView {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return unavailable("task-force route arena length invalid");
        };
        let rows = &self.motion.rows[..self.motion.outcome.len().min(self.motion.rows.len())];
        if rows.iter().any(|row| {
            row.ordinary_start
                .checked_add(row.ordinary_len)
                .and_then(|end| end.checked_add(row.alternate_len))
                .is_none_or(|end| end > paths.len())
        }) {
            return unavailable("task-force route lies outside the captured arena");
        }
        let Ok(mut handles) = collect_vec(rows.iter().map(|row| row.id)) else {
            return unavailable("collector allocation failed");
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("task force appears in more than one fleet roster");
        }
        table::<tables::TaskForceMapMotion, _>(self.motion.outcome, &self.motion.rows, |r| {
            let alternate = r.ordinary_start + r.ordinary_len;
            TaskForceMapMotionRow {
                task_force: r.id,
                target: r.target,
                combat_role: r.combat_role,
                path: owned_vec(copy_vec(&paths[r.ordinary_start..alternate])),
                alternate_path: owned_vec(copy_vec(&paths[alternate..alternate + r.alternate_len])),
                movement_numerator: Recorded::Value(r.movement_numerator),
                retreat: Recorded::Value(r.retreat),
                withdrawal: Recorded::Value(r.withdrawal),
                naval_transfer: Recorded::Value(r.naval_transfer),
                combat_links: Recorded::Value(r.combat_links),
                fresh: Recorded::Value(r.fresh.map(|fresh| FreshTaskForceCounterEta {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    base_speed: fresh.base_speed.ok(),
                    speed_modifier: fresh.speed_modifier.ok(),
                    terrain_class: fresh.terrain_class.ok(),
                    modified_range: fresh.modified_range.ok(),
                    exiled: fresh.exiled.ok(),
                    strategic_redeployment: fresh.strategic_redeployment.ok(),
                })),
            }
        })
    }
}

fn task(
    r: &ctask_force::CounterRow,
    names: Option<&[u8]>,
    origins: Option<&[engine_native::cunit_supply_inputs::Origin]>,
    port_mission: Option<NavalPortMissionInputs>,
) -> TaskForceCounterInputRow {
    TaskForceCounterInputRow {
        task_force: r.id,
        is_in_regions: r.is_in_regions,
        running_away: r.running_away,
        engagement_rule: r.engagement_rule,
        mission_region_count: r.mission_region_count,
        bombardment_region: r.bombardment_region,
        repair_parent: r.repair_parent,
        detached_activity: r.detached_activity,
        repair_last_mission: r.repair_last_mission,
        received_supply_ratio: r.received_supply_ratio,
        attached_headquarters: r.attached_headquarters,
        name: Recorded::Value(r.name.and_then(|(start, len)| {
            names?
                .get(start..start.checked_add(len)?)
                .map(|bytes| owned_vec(copy_vec(bytes)))
        })),
        supply: Recorded::Value(super::supply_inputs::project(r.supply, origins)),
        port_mission: Recorded::Value(port_mission),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
    use replay_data::rows::TableRows;

    #[test]
    fn sibling_failure_reset_settles_completed_counter_diagnostics_once() {
        use std::sync::atomic::Ordering::Relaxed;
        let before = ARROW_REFUSALS.each_ref().map(|counter| counter.load(Relaxed));
        let mut captured = Captured::vacant();
        captured.task_forces.rows.push(Default::default());
        captured.task_forces.outcome = Outcome::Rows { len: 1 };
        captured.motion.rows.push(Default::default());
        captured.motion.outcome = Outcome::Rows { len: 1 };
        captured.owned_pending = true;
        captured.reset();
        let after = ARROW_REFUSALS.each_ref().map(|counter| counter.load(Relaxed));
        assert_eq!(after, before.map(|count| count + 1));
        captured.reset();
        captured.decode_owned();
        assert_eq!(ARROW_REFUSALS.each_ref().map(|counter| counter.load(Relaxed)), after);
    }

    struct Singles<'a> {
        source: &'a SliceMemory<'a>,
        singles: std::cell::RefCell<Vec<(usize, usize)>>,
        calls: std::cell::Cell<usize>,
    }

    impl Memory for Singles<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.singles.borrow_mut().push((address, output.len()));
            self.calls.set(self.calls.get() + 1);
            self.source.copy(address, output)
        }

        fn copy_records(
            &self,
            sources: engine_native::memory::RecordSources<'_>,
            layout: &engine_native::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            self.source.copy_records(sources, layout, destination)
        }

        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            self.source.copy_scattered(addresses, destinations)
        }

        fn copy_scattered_prefetched<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            self.source
                .copy_scattered_prefetched(addresses, destinations)
        }
    }

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }
    const STATE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const FLEET: usize = 0x4000;
    const TASK: usize = 0x5000;
    const REGION: usize = 0x8000;

    fn cold_storage_world() -> Vec<u8> {
        use engine_native::{ccountry, cgame_state, cnaval_mission, cstrategic_region, cunit};
        let mut bytes = vec![0; 0x9000];
        array(&mut bytes, STATE + cgame_state::COUNTRIES, 0x2000, 2);
        array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x2020, 2);
        array(&mut bytes, STATE + cgame_state::PROVINCES, 0x2050, 2);
        array(
            &mut bytes,
            STATE + cgame_state::STRATEGIC_REGIONS,
            0x2060,
            2,
        );
        p64(&mut bytes, 0x2008, COUNTRY as u64);
        p32(&mut bytes, 0x2024, 1);
        p64(&mut bytes, 0x2068, REGION as u64);
        p64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, COUNTRY + ccountry::ID, 1);
        array(&mut bytes, COUNTRY + ccountry::FLEETS, 0x2040, 1);
        p64(&mut bytes, 0x2040, FLEET as u64);
        p64(&mut bytes, FLEET, cfleet::VTABLE_RVA.0 as u64);
        p64(&mut bytes, FLEET + cfleet::HANDLE, (1 << 32) | 61);
        p64(&mut bytes, FLEET + cfleet::OWNER, COUNTRY as u64);
        array(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x2030, 1);
        p64(&mut bytes, 0x2030, TASK as u64);
        p64(&mut bytes, TASK, ctask_force::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            TASK + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
        );
        p64(&mut bytes, TASK + cunit::HANDLE, (1 << 32) | 51);
        p64(&mut bytes, TASK + ctask_force::FLEET, FLEET as u64);
        p32(&mut bytes, TASK + cunit::ROSTER_OWNER, 1);
        p32(&mut bytes, TASK + cunit::LOGICAL_OWNER, 1);
        p32(&mut bytes, TASK + ctask_force::MISSION, 2);
        let mission = TASK + ctask_force::NAVAL_MISSION;
        p64(&mut bytes, mission, cnaval_mission::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            mission + cnaval_mission::TASK_FORCE,
            TASK as u64,
        );
        p32(&mut bytes, mission + cnaval_mission::ENGAGEMENT_RULE, 2);
        array(
            &mut bytes,
            mission + cnaval_mission::ACCESSIBLE_REGIONS,
            0x7020,
            1,
        );
        p64(&mut bytes, 0x7020, REGION as u64);
        p64(&mut bytes, REGION, cstrategic_region::VTABLE_RVA.0 as u64);
        p32(&mut bytes, REGION + cstrategic_region::ID, 1);
        for offset in [cunit::ORDINARY_ROUTE_DATA, cunit::ALTERNATE_ROUTE_DATA] {
            array(&mut bytes, TASK + offset, 0x7010, 1);
        }
        p32(&mut bytes, 0x7010, 1);
        let name = TASK + 0x258;
        bytes[name..name + 4].copy_from_slice(b"Navy");
        p64(&mut bytes, name + 16, 4);
        p64(&mut bytes, name + 24, 15);
        array(
            &mut bytes,
            TASK + engine_native::cunit_supply_inputs::ORIGINS,
            0x7100,
            1,
        );
        array(&mut bytes, TASK + 0x610, 0x7030, 1);
        array(&mut bytes, TASK + 0x740, 0x7200, 1);
        bytes[0x7200..0x7203].copy_from_slice(b"mod");
        p64(&mut bytes, 0x7210, 3);
        p64(&mut bytes, 0x7218, 15);
        bytes
    }

    #[test]
    fn mission_region_ids_are_held_for_their_world() {
        use engine_native::cstrategic_region;
        let mut bytes = cold_storage_world();
        let id_read = |memory: &Singles<'_>| {
            memory
                .singles
                .borrow()
                .iter()
                .any(|&(at, _)| at == REGION + cstrategic_region::ID)
        };
        let regions = |captured: &Captured| {
            let [.., TableView::Complete(TableRows::TaskForceMissions(rows))] = captured.tables()
            else {
                panic!("complete missions");
            };
            rows[0].regions.to_vec()
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut captured = unsafe { Captured::with_capacities(&Capacities::default()) };
        let capture = |captured: &mut Captured, bytes: &[u8], world: Option<u64>| {
            let slice = SliceMemory { base: 0, bytes };
            let memory = Singles {
                source: &slice,
                singles: Default::default(),
                calls: Default::default(),
            };
            captured.reset();
            let holds = Holds {
                world,
                ..Holds::NONE
            };
            captured.capture_impl(&memory, 0, STATE, Some(1), Err("fixture"), holds, false);
            id_read(&memory)
        };
        assert!(capture(&mut captured, &bytes, Some(7)));
        assert_eq!(regions(&captured), [1]);
        let first = captured.tables();
        p32(&mut bytes, REGION + cstrategic_region::ID, 9);
        assert!(!capture(&mut captured, &bytes, Some(7)));
        assert_eq!(regions(&captured), [1]);
        assert_eq!(captured.tables()[0], first[0]);
        p32(&mut bytes, REGION + cstrategic_region::ID, 1);
        assert!(capture(&mut captured, &bytes, Some(8)));
        assert_eq!(regions(&captured), [1]);
        assert!(capture(&mut captured, &bytes, None));
        assert!(capture(&mut captured, &bytes, None));
    }

    #[test]
    fn default_cold_storage_captures_original_headers_and_live_hints() {
        let bytes = cold_storage_world();
        let slice = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let memory = Singles {
            source: &slice,
            singles: Default::default(),
            calls: Default::default(),
        };
        let mut capacity = Capacities::default();
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut captured = unsafe { Captured::with_capacities(&capacity) };
        assert_eq!(captured.allocated_bytes(), 0);
        captured.capture_with_hour(&memory, 0, STATE, 123, Err("fixture modifier refusal"));
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
        let singles = memory.singles.borrow();
        for (offset, len) in [
            (0x258, 32),
            (engine_native::cunit_supply_inputs::ORIGINS, 16),
        ] {
            assert!(
                !singles.contains(&(TASK + offset, len)),
                "{offset:#x} read alone"
            );
        }
        drop(singles);
        let calls = |gather: bool| {
            TASK_GATHER_OFF.set(!gather);
            memory.calls.set(0);
            // SAFETY: synthetic unit test with no live naval observer hooks.
            let mut captured = unsafe { Captured::with_capacities(&Capacities::default()) };
            captured.capture_with_hour(&memory, 0, STATE, 123, Err("fixture modifier refusal"));
            TASK_GATHER_OFF.set(false);
            memory.calls.get()
        };
        let (gathered, direct) = (calls(true), calls(false));
        eprintln!("TASK_FORCE_COUNTER_CALLS gathered {gathered} direct {direct}");
        assert!(
            gathered + 10 <= direct,
            "{gathered} calls gathered, {direct} direct"
        );
        let memory = slice;
        assert_eq!(
            (
                captured.rosters.len,
                captured.paths.len,
                captured.regions.len
            ),
            (1, 2, 1)
        );
        assert_eq!((captured.names.len, captured.supply_origins.len), (4, 1));
        assert_eq!(
            (
                captured.port_searchers.len,
                captured.port_avoided.len,
                captured.port_bytes.len
            ),
            (1, 1, 3)
        );
        assert_eq!(
            captured.task_forces.rows[0].supply.unwrap().origins_len,
            Ok(1)
        );
        assert!(
            captured
                .tables()
                .iter()
                .all(|table| matches!(table, TableView::Complete(_)))
        );
        // SAFETY: synthetic unit test with no live naval observer hooks.
        unsafe { capacity.presize(&memory, 0, STATE) }.unwrap();
        assert_eq!(
            (
                capacity.task_forces,
                capacity.fleets,
                capacity.rosters,
                capacity.motion,
                capacity.missions
            ),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(
            (
                capacity.paths,
                capacity.regions,
                capacity.names,
                capacity.supply_origins
            ),
            (2, 1, 4, 1)
        );
        assert_eq!(
            (
                capacity.port_searchers,
                capacity.port_avoided,
                capacity.port_bytes,
                capacity.port_answers
            ),
            (1, 1, 3, 4)
        );
    }

    #[test]
    fn unreadable_countries_fail_every_table_and_rosters_convert_in_native_order() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe {
            Captured::with_capacities(&Capacities {
                task_forces: 1,
                fleets: 1,
                rosters: 2,
                motion: 1,
                paths: 1,
                missions: 1,
                regions: 1,
                names: 0,
                supply_origins: 0,
                port_searchers: 0,
                port_avoided: 0,
                port_bytes: 0,
                port_answers: 0,
            })
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        unsafe { capture.capture(&memory, 0, 0) };
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.reset();
        capture.fleets.rows[0] = cfleet::CounterRow {
            id: (1 << 32) | 61,
            rgba: [0x3f80_0000, 0, 0x8000_0000, u32::MAX],
            mission_region_count: 2,
            start: 0,
            len: 2,
        };
        capture.rosters.first[..2].copy_from_slice(&[(54 << 32) | 51, (43 << 32) | 51]);
        capture.rosters.len = 2;
        capture.fleets.outcome = Outcome::Rows { len: 1 };
        capture.task_forces.outcome = Outcome::Rows { len: 0 };
        capture.motion.outcome = Outcome::Rows { len: 0 };
        let [
            _,
            TableView::Complete(TableRows::FleetCounterInputs(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].task_forces, [(54 << 32) | 51, (43 << 32) | 51]);
        assert_eq!(rows[0].blue, 0x8000_0000);
        capture.fleets.rows[0].len = 3;
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }

    #[test]
    fn cached_task_force_names_are_owned_and_failure_never_hides_counter_prefixes() {
        let mut capacity = Capacities {
            task_forces: 2,
            names: 7,
            ..Default::default()
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: 51,
            received_supply_ratio: 35_000,
            name: Some((0, 0)),
            ..Default::default()
        };
        capture.task_forces.rows[1] = ctask_force::CounterRow {
            id: 61,
            running_away: 1,
            name: Some((0, 7)),
            ..Default::default()
        };
        capture.names.first[..7].copy_from_slice(b"TF\0tail");
        capture.names.len = 7;
        capture.task_forces.outcome = Outcome::Rows { len: 2 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(owned)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("complete counter prefix");
        };
        assert_eq!(owned[0].name, Recorded::Value(Some(Vec::new())));
        assert_eq!(owned[1].name, Recorded::Value(Some(b"TF\0tail".to_vec())));
        capture.names.first[..7].copy_from_slice(b"Renamed");
        let TableView::Complete(TableRows::TaskForceCounterInputs(renamed)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("complete counter prefix");
        };
        assert_eq!(renamed[1].name, Recorded::Value(Some(b"Renamed".to_vec())));
        assert_eq!(
            owned[1].name,
            Recorded::Value(Some(b"TF\0tail".to_vec())),
            "projection owns its bytes"
        );
        capture.task_forces.rows[0].name = None;
        capture.task_forces.rows[1].name = Some((usize::MAX, 7));
        let TableView::Complete(TableRows::TaskForceCounterInputs(refused)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("name refusals preserve the table");
        };
        assert!(refused.iter().all(|row| row.name == Recorded::Value(None)));
        assert_eq!(refused[0].received_supply_ratio, 35_000);
        assert_eq!(refused[1].running_away, 1);
        capture.names.required = 35;
        capture.grow(&mut capacity);
        assert!(capacity.names >= 35 && capture.names.first.len() >= 35);
        assert_eq!(capture.names.len, 7);
        capacity.raise_for_names(100, 200);
        assert_eq!((capacity.task_forces, capacity.names), (100, 200));
        capture.reset();
        assert_eq!((capture.names.len, capture.names.required), (0, 0));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    }

    #[test]
    fn motion_routes_split_from_the_arena_and_torn_censuses_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 2,
            paths: 3,
            missions: 1,
            regions: 1,
            names: 0,
            supply_origins: 0,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        capture.motion.rows[0] = ctask_force::MapMotionRow {
            id: (5 << 32) | 61,
            target: 9,
            combat_role: 2,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_len: 1,
            combat_links: Some([u64::MAX, 0x8000_0000_0000_0001]),
            ..Default::default()
        };
        capture.paths.first.copy_from_slice(&[7, 9, 11]);
        capture.paths.len = 3;
        capture.motion.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMapMotion(rows)) = capture.motion_table()
        else {
            panic!("complete motion table");
        };
        assert_eq!(
            (rows[0].path.as_slice(), rows[0].alternate_path.as_slice()),
            (&[7, 9][..], &[11][..])
        );
        assert_eq!((rows[0].target, rows[0].combat_role), (9, 2));
        assert_eq!(
            rows[0].combat_links,
            Recorded::Value(Some([u64::MAX, 0x8000_0000_0000_0001]))
        );
        capture.motion.rows[0].combat_links = None;
        let TableView::Complete(TableRows::TaskForceMapMotion(refused)) = capture.motion_table()
        else {
            panic!("complete motion prefix after suffix refusal");
        };
        assert_eq!(refused[0].combat_links, Recorded::Value(None));
        assert_eq!((refused[0].target, refused[0].combat_role), (9, 2));
        capture.motion.rows[0].alternate_len = 2;
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.rows[0].alternate_len = 0;
        capture.motion.rows[1] = capture.motion.rows[0];
        capture.motion.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        capture.paths.required = 12;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 12 && capture.paths.first.len() >= 12);
    }

    #[test]
    fn fresh_eta_projection_keeps_independent_refusals_and_clears_every_owner_loan() {
        let capacity = Capacities {
            motion: 2,
            ..Capacities::default()
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        let owner = engine_native::ccountry::Country {
            pointer: 0x1234,
            id: 19,
            canonical_slot: 3,
        };
        capture.motion.rows[0] = ctask_force::MapMotionRow {
            id: 7,
            logical_owner: 22,
            fresh_owner: Some(Ok(owner)),
            fresh: Some(ctask_force::FreshEtaInputs {
                sample_hour: 123,
                base_speed: Err("fixture base refusal"),
                speed_modifier: Ok(i64::MIN),
                terrain_class: Ok(-1),
                modified_range: Ok(i64::MAX),
                exiled: Ok(0xfe),
                strategic_redeployment: Err("fixture redeployment refusal"),
            }),
            ..Default::default()
        };
        capture.motion.outcome = Outcome::Rows { len: 1 };
        assert_eq!(
            capture.fresh_owner_demands().collect::<Vec<_>>(),
            [(22, Ok(owner))]
        );
        let TableView::Complete(TableRows::TaskForceMapMotion(rows)) = capture.motion_table()
        else {
            panic!("complete prefix");
        };
        assert_eq!(
            rows[0].fresh,
            Recorded::Value(Some(FreshTaskForceCounterEta {
                sample_hour: replay_data::rows::PacketHour::Host,
                base_speed: None,
                speed_modifier: Some(i64::MIN),
                terrain_class: Some(-1),
                modified_range: Some(i64::MAX),
                exiled: Some(0xfe),
                strategic_redeployment: None,
            }))
        );
        capture.motion.rows[1].fresh_owner = Some(Err("fixture owner refusal"));
        capture.motion.outcome = Outcome::Unavailable {
            reason: "fixture later census fault",
        };
        capture.clear_fresh_owner_demands();
        assert!(
            capture
                .motion
                .rows
                .iter()
                .all(|row| row.fresh_owner.is_none())
        );
        capture.motion.rows[1].fresh_owner = Some(Ok(owner));
        capture.reset();
        assert!(
            capture
                .motion
                .rows
                .iter()
                .all(|row| row.fresh_owner.is_none())
        );
        assert_eq!(capture.motion.rows[0].fresh.unwrap().sample_hour, 123);
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn mission_regions_split_from_the_arena_and_stray_ranges_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 1,
            paths: 1,
            missions: 1,
            regions: 3,
            names: 0,
            supply_origins: 0,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        capture.missions.rows[0] = ctask_force::MissionRow {
            id: (5 << 32) | 61,
            owner: 42,
            current_region: 2,
            regions_start: 0,
            regions_len: 3,
        };
        capture.regions.first.copy_from_slice(&[2, 1, 2]);
        capture.regions.len = 3;
        capture.missions.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMissions(rows)) = capture.mission_table()
        else {
            panic!("complete mission table");
        };
        assert_eq!(
            (rows[0].task_force, rows[0].owner, rows[0].current_region),
            ((5 << 32) | 61, 42, 2)
        );
        assert_eq!(rows[0].regions, [2, 1, 2]);
        capture.missions.rows[0].regions_len = 4;
        assert!(matches!(
            capture.mission_table(),
            TableView::Unavailable { .. }
        ));
        capture.missions.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.regions.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.regions >= 9 && capture.regions.first.len() >= 9);
    }

    #[test]
    fn supply_nodes_fail_independently_and_owned_ship_demand_keeps_names_and_prefixes() {
        use engine_native::{
            cunit_supply_inputs::Demand, cunit_supply_inputs::Failure, cunit_supply_inputs::Inputs,
            cunit_supply_inputs::Origin,
        };
        let mut capacity = Capacities {
            task_forces: 1,
            names: 4,
            supply_origins: 2,
            port_searchers: 0,
            port_avoided: 0,
            port_bytes: 0,
            port_answers: 0,
            ..Default::default()
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.supply_origins.first[..2].copy_from_slice(&[
            Origin {
                province: 13,
                requested: -1,
                received: 2,
            },
            Origin {
                province: 13,
                requested: 3,
                received: -4,
            },
        ]);
        capture.supply_origins.len = 2;
        let input = Inputs {
            asked: 190_000,
            received: 23_000,
            ship_demand: Demand {
                count: 3,
                value: 190_000,
            },
            disruption: -9,
            origins_len: Ok(2),
            ..Default::default()
        };
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            received_supply_ratio: 12_105,
            name: Some((0, 4)),
            supply: Some(input),
            ..Default::default()
        };
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(owned)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("supply suffix must not hide the counter prefix")
        };
        let supply = owned[0]
            .supply
            .require("fixture supply")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(
            (supply.ship_demand.count, supply.ship_demand.value),
            (3, 190_000)
        );
        let origins = supply.origins.as_ref().unwrap();
        assert_eq!(
            (
                origins[1].province,
                origins[1].requested,
                origins[1].received
            ),
            (13, 1_000, -1_000)
        );
        capture.supply_origins.first[1].received = 999;
        assert_eq!(
            origins[1].received, -1_000,
            "projection owns the original ordered bytes"
        );
        for reason in [
            Failure::InvalidOrigins,
            Failure::OriginsChanged,
            Failure::OriginsCapacity { required: 41 },
        ] {
            capture.task_forces.rows[0].supply = Some(Inputs {
                origins_len: Err(reason),
                ..input
            });
            capture.task_forces.rows[0].supply_failure = Some(reason);
            let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
                capture.tables().into_iter().next().unwrap()
            else {
                panic!("origin refusal must preserve supply scalars")
            };
            let supply = rows[0]
                .supply
                .require("fixture supply")
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!(
                (supply.asked, supply.received, supply.disruption),
                (190_000, 23_000, -1_000)
            );
            assert_eq!(supply.ship_demand.count, 3);
            assert!(supply.origins.is_none());
            assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
            assert_eq!(rows[0].received_supply_ratio, 12_105);
        }
        capture.supply_origins.required = 41;
        capture.grow(&mut capacity);
        assert!(capacity.supply_origins >= 41 && capture.supply_origins.first.len() >= 41);
        capture.task_forces.rows[0].supply = None;
        capture.task_forces.rows[0].supply_failure = Some(Failure::Unreadable);
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("scalar refusal must preserve the native counter prefix")
        };
        assert_eq!(rows[0].supply, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
        assert_eq!(rows[0].received_supply_ratio, 12_105);
    }

    #[test]
    fn invalid_supply_origin_span_withholds_only_nodes_and_name_fault_keeps_supply() {
        use engine_native::cunit_supply_inputs::Inputs;
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe {
            Captured::with_capacities(&Capacities {
                task_forces: 1,
                ..Default::default()
            })
        };
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            running_away: 0xff,
            supply: Some(Inputs {
                received: i64::MIN,
                origins_len: Ok(1),
                ..Default::default()
            }),
            supply_origins_start: usize::MAX,
            ..Default::default()
        };
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("an optional arena failure is not a counter census failure")
        };
        let supply = rows[0]
            .supply
            .require("fixture supply")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(supply.received, i64::MIN);
        assert!(supply.origins.is_none());
        assert_eq!(rows[0].name, Recorded::Value(None));
        assert_eq!(rows[0].running_away, 0xff);
    }

    #[test]
    fn port_projection_owns_ordered_fault_slots_and_exact_bytes_without_hiding_old_fields() {
        use ctask_force::port_mission::{EngagementTarget, Inputs, Searcher};
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe {
            Captured::with_capacities(&Capacities {
                task_forces: 1,
                names: 4,
                port_searchers: 3,
                port_avoided: 3,
                port_bytes: 3,
                ..Default::default()
            })
        };
        capture.task_forces.rows[0] = ctask_force::CounterRow {
            id: (29 << 32) | 61,
            received_supply_ratio: 12_345,
            name: Some((0, 4)),
            port_mission: Some(Inputs {
                sample_hour: 123,
                searching: Some((0, 3)),
                engagement_target: Some(EngagementTarget {
                    reference: (99 << 32) | 4713,
                    resolves: Some(false),
                }),
                avoided_ship_types: Some((0, 3)),
            }),
            ..Default::default()
        };
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.port_searchers.first.copy_from_slice(&[
            Searcher {
                reference: Some(0),
                resolves: Some(false),
                roster_owner: None,
            },
            Searcher {
                reference: Some((17 << 32) | 4713),
                resolves: Some(true),
                roster_owner: Some(0),
            },
            Searcher::default(),
        ]);
        capture.port_searchers.len = 3;
        capture
            .port_avoided
            .first
            .copy_from_slice(&[Some((0, 3)), None, Some((3, 0))]);
        capture.port_avoided.len = 3;
        capture.port_bytes.first.copy_from_slice(&[0xff, 0, 7]);
        capture.port_bytes.len = 3;
        capture.task_forces.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceCounterInputs(rows)) =
            capture.tables().into_iter().next().unwrap()
        else {
            panic!("optional port faults preserve the counter census")
        };
        let packet = rows[0]
            .port_mission
            .require("fixture port")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(packet.sample_hour, replay_data::rows::PacketHour::Host);
        assert_eq!(packet.searching.as_ref().unwrap()[1].roster_owner, Some(0));
        assert_eq!(packet.searching.as_ref().unwrap()[2].reference, None);
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().reference,
            (99 << 32) | 4713
        );
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().resolves,
            Some(false)
        );
        assert_eq!(
            packet.avoided_ship_types,
            Some(vec![Some(vec![0xff, 0, 7]), None, Some(vec![])])
        );
        capture.port_bytes.first.fill(0);
        capture.port_searchers.first.fill(Searcher::default());
        assert_eq!(
            packet.avoided_ship_types,
            Some(vec![Some(vec![0xff, 0, 7]), None, Some(vec![])])
        );
        assert_eq!(packet.searching.as_ref().unwrap()[1].roster_owner, Some(0));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"Navy".to_vec())));
        assert_eq!(rows[0].received_supply_ratio, 12_345);
        capture.task_forces.rows[0]
            .port_mission
            .as_mut()
            .unwrap()
            .searching = Some((usize::MAX, 3));
        capture.port_avoided.first[0] = Some((usize::MAX, 3));
        let packet = capture.port_inputs(&capture.task_forces.rows[0]).unwrap();
        assert!(packet.searching.is_none());
        assert_eq!(packet.avoided_ship_types.as_ref().unwrap()[0], None);
        assert_eq!(
            packet.engagement_target.as_ref().unwrap().resolves,
            Some(false)
        );
    }

    #[test]
    fn port_growth_and_sizing_never_change_cached_name_capacity_or_bytes() {
        let mut capacity = Capacities {
            names: 4,
            ..Default::default()
        };
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut capture = unsafe { Captured::with_capacities(&capacity) };
        let mut old_capacity = capacity;
        // SAFETY: synthetic unit test with no live naval observer hooks.
        let mut old = unsafe { Captured::with_capacities(&old_capacity) };
        old.names.first[..4].copy_from_slice(b"Navy");
        old.names.len = 4;
        capture.names.first[..4].copy_from_slice(b"Navy");
        capture.names.len = 4;
        capture.port_searchers.required = 27;
        capture.port_avoided.required = 31;
        capture.port_bytes.required = 1025;
        capture.port_answers.required = 128;
        capture.grow(&mut capacity);
        old.grow(&mut old_capacity);
        assert_eq!(capacity.names, old_capacity.names);
        assert_eq!(capture.names.first.as_slice(), old.names.first.as_slice());
        assert_eq!(&capture.names.first[..capture.names.len], b"Navy");
        assert!(capacity.port_searchers >= 27 && capacity.port_avoided >= 31);
        assert!(capacity.port_bytes >= 1025 && capacity.port_answers >= 128);
        let before = capacity;
        capacity
            .raise_for_port(ctask_force::port_mission::Demand {
                searchers: 100,
                avoided: 200,
                bytes: 3000,
                references: 101,
                refusal: None,
            })
            .unwrap();
        assert_eq!(
            (
                capacity.port_searchers,
                capacity.port_avoided,
                capacity.port_bytes,
                capacity.port_answers
            ),
            (100, 200, 3000, 256)
        );
        assert_eq!(capacity.names, before.names);
        assert_eq!(capacity.supply_origins, before.supply_origins);
        capture.reset();
        assert_eq!(
            (
                capture.port_searchers.len,
                capture.port_searchers.required,
                capture.port_avoided.len,
                capture.port_avoided.required,
                capture.port_bytes.len,
                capture.port_bytes.required,
                capture.port_answers.required
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
    }

    fn census(receiver: usize, raw: i64, certificate: (u64, u64)) -> ctask_force::HeldCensus {
        let mut census = ctask_force::HeldCensus {
            receiver,
            address: receiver + 0x5b8,
            certificate,
            ..Default::default()
        };
        census.keys[0] = 7;
        census.values[0] = engine_native::modifier_accumulator::Value { raw, present: true };
        census
    }

    fn use_census(
        store: &mut CensusStore,
        registry: &crate::watch_registry::WatchRegistry,
        receiver: usize,
        current: i64,
        reads: &mut usize,
    ) -> i64 {
        let certificate = registry.sequence(receiver);
        let fresh = census(receiver, current, certificate.unwrap_or_default());
        let used = certificate
            .and_then(|certificate| {
                held_census(&store.serve, &fresh.identity())
                    .filter(|held| held.certificate == certificate)
            })
            .unwrap_or_else(|| {
                *reads += 1;
                fresh
            });
        if certificate.is_some() {
            store.log.push(used);
        }
        used.values[0].raw
    }

    fn census_capture(
        store: &mut CensusStore,
        registry: &crate::watch_registry::WatchRegistry,
        trees: &[(usize, i64)],
        reads: &mut usize,
    ) -> (Vec<i64>, u64) {
        store.prepare();
        let used = trees
            .iter()
            .map(|&(receiver, current)| use_census(store, registry, receiver, current, reads))
            .collect();
        (
            used,
            store.finish(true, |receiver| registry.sequence(receiver)),
        )
    }

    const TREE: usize = 0x10_0000;

    #[test]
    fn a_fresh_census_is_not_held_past_a_writer_after_its_read() {
        let registry = crate::watch_registry::WatchRegistry::enabled(64);
        let mut store = CensusStore::default();
        let mut reads = 0;
        store.prepare();
        assert_eq!(use_census(&mut store, &registry, TREE, 10, &mut reads), 10);
        drop(registry.notify(TREE));
        store.finish(true, |receiver| registry.sequence(receiver));
        assert!(
            store.held.is_empty(),
            "the census is not held past the writer"
        );
        let (used, _) = census_capture(&mut store, &registry, &[(TREE, 11)], &mut reads);
        assert_eq!((used, reads), (vec![11], 2));
    }

    #[test]
    fn a_served_census_is_not_held_past_a_writer_after_it_was_served() {
        let registry = crate::watch_registry::WatchRegistry::enabled(64);
        let mut store = CensusStore::default();
        let mut reads = 0;
        census_capture(&mut store, &registry, &[(TREE, 10)], &mut reads);
        assert_eq!(store.held.len(), 1);
        store.prepare();
        assert_eq!(use_census(&mut store, &registry, TREE, 10, &mut reads), 10);
        assert_eq!(reads, 1, "served");
        drop(registry.notify(TREE));
        let served = store.finish(true, |receiver| registry.sequence(receiver));
        assert_eq!(served, 1);
        assert!(store.held.is_empty(), "the served census is not held again");
        let (used, _) = census_capture(&mut store, &registry, &[(TREE, 11)], &mut reads);
        assert_eq!((used, reads), (vec![11], 2));
    }

    #[test]
    fn a_writer_before_the_first_registration_is_read() {
        let registry = crate::watch_registry::WatchRegistry::enabled(64);
        let mut store = CensusStore::default();
        let mut reads = 0;
        drop(registry.notify(TREE));
        let (used, _) = census_capture(&mut store, &registry, &[(TREE, 11)], &mut reads);
        assert_eq!(used, [11]);
        let (used, served) = census_capture(&mut store, &registry, &[(TREE, 11)], &mut reads);
        assert_eq!((used, served, reads), (vec![11], 1, 1));
    }

    #[test]
    fn an_incomplete_census_capture_holds_nothing() {
        let registry = crate::watch_registry::WatchRegistry::enabled(64);
        let mut store = CensusStore::default();
        let mut reads = 0;
        census_capture(&mut store, &registry, &[(TREE, 10)], &mut reads);
        store.prepare();
        use_census(&mut store, &registry, TREE, 10, &mut reads);
        store.finish(false, |receiver| registry.sequence(receiver));
        assert!(store.held.is_empty());
        census_capture(&mut store, &registry, &[(TREE, 10)], &mut reads);
        assert_eq!(store.held.len(), 1);
        store.prepare();
        assert!(store.held.is_empty(), "taken out while the capture runs");
        store.prepare();
        assert!(store.serve.is_empty());
        census_capture(&mut store, &registry, &[(TREE, 10)], &mut reads);
        assert_eq!(store.held.len(), 1);
        let bytes = store.allocated_bytes();
        store.forget();
        assert!(store.held.is_empty() && store.serve.is_empty() && store.log.is_empty());
        assert_eq!(store.allocated_bytes(), bytes);
    }

    #[test]
    fn censuses_of_many_receivers_are_held_and_served() {
        const RECEIVERS: usize = 8;
        let registry =
            crate::watch_registry::WatchRegistry::enabled((4 * RECEIVERS).next_power_of_two());
        let mut store = CensusStore::default();
        let mut reads = 0;
        let trees: Vec<_> = (0..RECEIVERS)
            .rev()
            .map(|index| (TREE + index * 0x1310, index as i64))
            .collect();
        census_capture(&mut store, &registry, &trees, &mut reads);
        assert_eq!((store.held.len(), reads), (RECEIVERS, RECEIVERS));
        assert!(
            store
                .held
                .is_sorted_by_key(ctask_force::HeldCensus::identity)
        );
        let (_, served) = census_capture(&mut store, &registry, &trees, &mut reads);
        assert_eq!((served, reads), (RECEIVERS as u64, RECEIVERS));
        assert_eq!(store.held.len(), RECEIVERS);
    }
    fn fleet(epoch: u64, sequence: u64) -> FleetCertificate {
        FleetCertificate::Fleet { epoch, sequence }
    }
    #[test]
    fn quiet_fleet_region_payloads_carry_without_reads_or_allocations() {
        use std::sync::Arc;
        let mut store = std::collections::BTreeMap::new();
        let _ = observe_fleet_regions(
            &mut store,
            16,
            7,
            Some(fleet(1, 2)),
            || Some(fleet(1, 2)),
            || Ok(vec![3, 3, 4]),
        );
        let held = store[&7].regions.clone().unwrap();
        collector_test_support::with_allocations_denied(|| {
            let _ = observe_fleet_regions(
                &mut store,
                16,
                7,
                Some(fleet(1, 2)),
                || Some(fleet(1, 2)),
                || panic!("quiet fleet payload reread"),
            );
            assert!(Arc::ptr_eq(store[&7].regions.as_ref().unwrap(), &held));
            let cloned = store[&7].regions.clone().unwrap();
            assert!(Arc::ptr_eq(&cloned, &held));
        });
        let _ = observe_fleet_regions(
            &mut store,
            16,
            7,
            Some(fleet(1, 3)),
            || Some(fleet(1, 3)),
            || Ok(vec![3, 4]),
        );
        assert_eq!(store[&7].regions.as_ref().unwrap().as_slice(), [3, 4]);
        let _ = observe_fleet_regions(
            &mut store,
            16,
            7,
            None,
            || None,
            || panic!("disabled observation polled a fleet"),
        );
        assert!(store[&7].regions.is_none());
        let mut calls = 0;
        let _ = observe_fleet_regions(
            &mut store,
            16,
            7,
            Some(fleet(1, 1)),
            || {
                calls += 1;
                Some(fleet(1, calls))
            },
            || Ok(vec![5]),
        );
        assert!(store[&7].regions.is_none());
    }

    #[test]
    fn fleet_writer_finishing_during_header_read_cannot_certify_the_old_header() {
        use std::cell::Cell;
        struct HeaderRace {
            sequence: Cell<u64>,
            changed: Cell<bool>,
            reads: Cell<usize>,
        }
        impl Memory for HeaderRace {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert_eq!(address, 16 + cfleet::MISSION_REGIONS);
                assert_eq!(output.len(), 16);
                self.reads.set(self.reads.get() + 1);
                let count = if self.changed.get() { 2_u32 } else { 1 };
                output[..8].copy_from_slice(&0x300_u64.to_le_bytes());
                output[8..12].copy_from_slice(&count.to_le_bytes());
                output[12..16].copy_from_slice(&count.to_le_bytes());
                if !self.changed.replace(true) {
                    self.sequence.set(self.sequence.get() + 1);
                }
                true
            }
        }
        let memory = HeaderRace {
            sequence: Cell::new(1),
            changed: Cell::new(false),
            reads: Cell::new(0),
        };
        let token = || Some(fleet(1, memory.sequence.get()));
        let mut store = std::collections::BTreeMap::new();
        let _ = observe_fleet_regions(&mut store, 16, 7, token(), token, || Ok(vec![9]));
        let before_header = token();
        let old = engine_native::cpdx_array::Array::read(&memory, 16 + cfleet::MISSION_REGIONS, 8)
            .unwrap();
        assert_eq!(old.count, 1);
        let _ = observe_fleet_regions(&mut store, 16, 7, before_header, token, || {
            panic!("raced header payload must not be read")
        });
        assert!(store[&7].regions.is_none());
        assert_eq!(store[&7].certificate, None);
        let before_header = token();
        let current =
            engine_native::cpdx_array::Array::read(&memory, 16 + cfleet::MISSION_REGIONS, 8)
                .unwrap();
        let _ = observe_fleet_regions(&mut store, 16, 7, before_header, token, || {
            Ok((0..current.count).collect())
        });
        assert_eq!(store[&7].regions.as_ref().unwrap().as_slice(), [0, 1]);
        assert_eq!(store[&7].certificate, token());
        assert_eq!(memory.reads.get(), 2);
    }
}
