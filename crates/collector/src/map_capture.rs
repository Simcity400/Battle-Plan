//! Pooled map-table reads. The spool grows reusable storage; the worker-only
//! Orders job can also grow payloads from its original native observations.
//!
//! Combat, ship and order censuses run inside their worker jobs; the ship
//! census runs inside the ShipModels job and the order census inside the Orders
//! job. Dependent Air/Naval consumers run only when their own raw payload did
//! not stage; the Players/HumanControllers pair keeps its worker split.

use crate::raw::RawLog;
use crate::sampling::qpc;
use crate::table_capture::{CaptureRows, grow_capacity, grow_vec};
use engine::{
    Memory, cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant,
    corder_instance, corders_group, cship, ctask_force, walk::Outcome,
};
use replay::rows::TableId;
use std::mem::size_of;

pub const MAP_TABLES: usize = 10;

mod combat;
pub(crate) use combat::{CombatReport, CombatRequest};

static AIR_WING_NAMES_REFUSED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static AIR_WING_NAME_FAILURES: [std::sync::atomic::AtomicU64;
    cair_wing::NameFailure::REASONS.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; cair_wing::NameFailure::REASONS.len()];
static AIR_WING_TRANSPORT_REFUSED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static AIR_WING_TRANSPORT_BYTES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn air_wing_name_data() -> impl serde::Serialize {
    let refused = AIR_WING_NAMES_REFUSED.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused,
        "transport_fields_refused": AIR_WING_TRANSPORT_REFUSED.load(std::sync::atomic::Ordering::Relaxed),
        "transport_source_bytes": AIR_WING_TRANSPORT_BYTES.load(std::sync::atomic::Ordering::Relaxed),
        "refusals": crate::status::map(std::iter::once(("air wing name guarded read or arena refusal", refused)).filter(|(_, count)| *count != 0)),
        "worker_name_refusals": crate::status::map(cair_wing::NameFailure::REASONS.into_iter().zip(
            AIR_WING_NAME_FAILURES.iter().map(|count| count.load(std::sync::atomic::Ordering::Relaxed))
        ).filter(|(_, count)| *count != 0)),
        "scope": "cumulative stored air-wing name and optional transport acquisition refusals; core AirWings columns remain available; transport_source_bytes covers owned wing acquisitions",
    })
}

pub(crate) fn observe_air_wing_name_refusals(count: usize) {
    AIR_WING_NAMES_REFUSED.fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
}

/// One census's worker name failures, by [`cair_wing::NameFailure::index`].
pub(crate) fn observe_air_wing_name_failures(
    failures: &[usize; cair_wing::NameFailure::REASONS.len()],
) {
    for (total, &count) in AIR_WING_NAME_FAILURES.iter().zip(failures) {
        if count != 0 {
            total.fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Inline source consumers own their state and validation. Return raw QPC ticks
/// attributed to extension tables so the map buckets do not charge them twice.
/// The begin/finish pair runs even for an empty or failed source census; every
/// dependent table must preserve that outcome instead of accepting its prefix.
pub trait MapSources {
    fn begin_land(&mut self) -> u64 {
        0
    }
    fn land(&mut self, _source: cland_combat::Source) -> u64 {
        0
    }
    fn finish_land(&mut self, _outcome: Outcome) -> u64 {
        0
    }
    fn begin_air(&mut self) -> u64;
    fn air(&mut self, pointer: usize, pool: usize, row: &cair_wing::Row) -> u64;
    fn air_owner(&mut self, _slot: u32, _entry: Option<usize>) -> u64 {
        0
    }
    fn air_score_limit(&self) -> Option<usize> {
        None
    }
    fn air_score(&mut self, _pointer: usize, _row: &cair_wing::ScoreLeaf) -> u64 {
        0
    }
    fn finish_air_score(&mut self, _outcome: Outcome) -> u64 {
        0
    }
    fn finish_air(&mut self, outcome: Outcome) -> u64;
}

#[cfg(test)]
struct NoSources;

#[cfg(test)]
impl MapSources for NoSources {
    fn begin_air(&mut self) -> u64 {
        0
    }
    fn air(&mut self, _: usize, _: usize, _: &cair_wing::Row) -> u64 {
        0
    }
    fn finish_air(&mut self, _: Outcome) -> u64 {
        0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapCapacities {
    rows: [usize; MAP_TABLES],
    player_names: usize,
    air_wing_names: usize,
    land_participants: usize,
    order_geometry: usize,
    order_groups: usize,
    order_visits: usize,
    naval_groups: usize,
}

impl Default for MapCapacities {
    fn default() -> Self {
        Self {
            rows: [0; MAP_TABLES],
            player_names: 0,
            air_wing_names: crate::country_scale::air_wing_name_bytes(),
            land_participants: 0,
            order_geometry: 0,
            order_groups: 0,
            order_visits: 0,
            naval_groups: 0,
        }
    }
}

impl MapCapacities {
    /// Attach/input cadence only; mutable headers remain hints for the worker.
    pub(crate) fn presize_combats(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        let state = base
            .checked_add(engine::cgame_state::SINGLETON_RVA)
            .and_then(|address| engine::read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("map combat sizing state unreadable")?;
        let land = cland_combat::core_demand(memory, base, state);
        let naval = cnaval_combat::storage_demand(memory, base, state);
        let mut failure = None;
        match land {
            Ok(demand) => {
                grow_capacity(&mut self.rows[2], demand.rows);
                grow_capacity(&mut self.land_participants, demand.participants);
            }
            Err(reason) => failure = Some(reason),
        }
        for (index, outcome) in [(6, naval.combats), (7, naval.participants)] {
            match outcome {
                Outcome::Rows { len } => grow_capacity(&mut self.rows[index], len),
                Outcome::Truncated { len, unread_rows } => grow_capacity(
                    &mut self.rows[index],
                    len.saturating_add(unread_rows as usize),
                ),
                Outcome::Unavailable { reason } => {
                    failure.get_or_insert(reason);
                }
            }
        }
        grow_capacity(&mut self.naval_groups, naval.required_group_capacity);
        failure.map_or(Ok(()), Err)
    }

    pub(crate) fn raise_air_wings(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.rows[8], crate::country_scale::rows(count, 1));
    }

    pub(crate) fn raise_naval(&mut self, index: usize, count: usize) {
        crate::country_scale::raise(
            &mut self.rows[index + 3],
            crate::country_scale::rows(count, 1),
        );
    }

    pub(crate) fn raise_for_air_wing_names(&mut self, bytes: usize) {
        crate::country_scale::raise(&mut self.air_wing_names, bytes);
    }

    /// HumanControllers has one row per authenticated canonical country
    /// (`chuman::walk_controllers`); the other map tables follow activity.
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows[1], crate::country_scale::rows(slots, 1));
    }
}

/// Fleets, task forces and ships. Walked inside the ShipModels job, whose
/// details the ships seed, so the game thread never runs the census.
pub struct ShipCensus {
    pub fleets: CaptureRows<cfleet::Row>,
    pub fleet_names: Vec<u8>,
    pub task_forces: CaptureRows<ctask_force::Row>,
    pub ships: CaptureRows<cship::Row>,
    /// Fleet, task force and ship walk ticks.
    pub ticks: [u64; 3],
}

impl ShipCensus {
    fn new(rows: &[usize]) -> Self {
        Self {
            fleets: CaptureRows::new(rows[0]),
            fleet_names: Vec::new(),
            task_forces: CaptureRows::new(rows[1]),
            ships: CaptureRows::new(rows[2]),
            ticks: [0; 3],
        }
    }
    /// Placeholder while the job owns the buffers.
    pub fn vacant() -> Self {
        Self::new(&[0; 3])
    }
    fn reset(&mut self) {
        self.fleets.reset();
        self.fleet_names.clear();
        self.task_forces.reset();
        self.ships.reset();
        self.ticks = [0; 3];
    }
    fn allocated_bytes(&self) -> usize {
        self.fleet_names.capacity()
            + self.fleets.allocated_bytes()
            + self.task_forces.allocated_bytes()
            + self.ships.allocated_bytes()
    }
    pub fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        models: &mut crate::extended::ship_models::Captured,
    ) {
        let tables = [TableId::Fleets, TableId::TaskForces, TableId::Ships];
        let mut reads = crate::read_accounting::table(tables[0]);
        let begin = qpc();
        models.begin_sources();
        let naval = cfleet::walk_timed_growing_with_names(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.task_forces.rows,
            &mut self.ships.rows,
            |phase| {
                reads.select(tables[phase]);
                qpc() as u64
            },
            |pointer, row| models.visit_source_growing(pointer, row),
            &mut self.fleet_names,
        );
        models.finish_sources(naval.capture.ships);
        self.fleets.outcome = naval.capture.fleets;
        self.task_forces.outcome = naval.capture.task_forces;
        self.ships.outcome = naval.capture.ships;
        self.ticks = naval.read_ticks;
        self.ticks[2] = elapsed(begin)
            .saturating_sub(self.ticks[0])
            .saturating_sub(self.ticks[1]);
    }
    pub fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.fleets.outcome = outcome;
        self.task_forces.outcome = outcome;
        self.ships.outcome = outcome;
        self.ticks = [0; 3];
    }
}

const NOT_WALKED: Outcome = Outcome::Unavailable {
    reason: "army order census not acquired",
};

/// The army order census and its graph scratch. Walked inside the Orders job;
/// the plans and the naval order sources read the roots and visits it leaves.
pub struct Orders {
    pub rows: CaptureRows<corder_instance::Row>,
    pub geometry: Vec<u32>,
    pub geometry_len: usize,
    geometry_required: usize,
    groups: Vec<corders_group::Root>,
    groups_required: usize,
    visits: Vec<corder_instance::Visit>,
    visits_required: usize,
    /// The successor ids this census resolved, for the plans of the same job.
    successors: corder_instance::SuccessorLists,
    pub root_outcome: Outcome,
    /// [`Self::walk_roots`] filled `groups` and `root_outcome` in this pass,
    /// for this pass's [`Self::walk`] only (the single census the tests and
    /// benches walk; the hourly job walks parts).
    #[cfg_attr(not(test), allow(dead_code))]
    roots_walked: bool,
    pub ticks: u64,
    /// This pass's census and plan-walk parts ([`crate::orders_parts`]),
    /// reused across hours: part 0 is the Orders job's, the rest the
    /// OrdersPart jobs' while they hold them.
    parts: [Option<Box<crate::orders_parts::Part>>; crate::orders_parts::MAX_PARTS],
    /// This pass's part count, set by [`Self::prepare_parts`].
    part_count: usize,
    /// This pass's split, which the callback carries from the last stitched
    /// hour ([`Self::begin_parts`]).
    split: crate::orders_parts::Split,
    /// The session's QPC frequency, for sizing the next split.
    frequency: u64,
    /// The next hour's split, planned when this pass's parts are stitched
    /// ([`Self::plan_split`]); one part until then.
    next_split: crate::orders_parts::Split,
    /// A failed or refused job failed this pass's census ([`Self::fail`]).
    failed: bool,
}

impl Orders {
    fn new(capacity: &MapCapacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows[9]),
            geometry: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.order_geometry,
            )),
            geometry_len: 0,
            geometry_required: 0,
            groups: engine::allocation::owned_vec(engine::allocation::default_vec::<
                corders_group::Root,
            >(capacity.order_groups)),
            groups_required: 0,
            visits: engine::allocation::owned_vec(engine::allocation::default_vec::<
                corder_instance::Visit,
            >(capacity.order_visits)),
            visits_required: 0,
            successors: corder_instance::SuccessorLists::default(),
            root_outcome: NOT_WALKED,
            roots_walked: false,
            ticks: 0,
            parts: [const { None }; crate::orders_parts::MAX_PARTS],
            part_count: 0,
            split: crate::orders_parts::Split::default(),
            frequency: 0,
            next_split: crate::orders_parts::Split::default(),
            failed: false,
        }
    }
    #[cfg(test)]
    pub(crate) fn for_test(capacity: &MapCapacities) -> Self {
        Self::new(capacity)
    }
    /// Placeholder while the job owns the buffers.
    pub fn vacant() -> Self {
        Self::new(&MapCapacities {
            rows: [0; MAP_TABLES],
            player_names: 0,
            air_wing_names: 0,
            land_participants: 0,
            order_geometry: 0,
            order_groups: 0,
            order_visits: 0,
            naval_groups: 0,
        })
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.geometry_len = 0;
        self.geometry_required = 0;
        self.groups_required = 0;
        self.visits_required = 0;
        self.root_outcome = NOT_WALKED;
        self.roots_walked = false;
        self.ticks = 0;
        self.part_count = 0;
        self.next_split = crate::orders_parts::Split::default();
        self.failed = false;
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.geometry.capacity() * size_of::<u32>()
            + self.groups.capacity() * size_of::<corders_group::Root>()
            + self.visits.capacity() * size_of::<corder_instance::Visit>()
            + self.successors.allocated_bytes()
            + self.parts_allocated_bytes()
    }
    /// The parts' storage, their boxes included: the root census job
    /// prepares it ([`Self::prepare_parts`]).
    pub(crate) fn parts_allocated_bytes(&self) -> usize {
        self.parts
            .iter()
            .flatten()
            .map(|part| part.allocated_bytes())
            .sum()
    }
    fn grow(&mut self, capacities: &mut MapCapacities) {
        capacities.rows[9] = capacities.rows[9].max(self.rows.outcome.len());
        capacities.order_geometry = capacities.order_geometry.max(self.geometry_required);
        capacities.order_visits = capacities.order_visits.max(self.visits_required);
        self.rows.grow(&mut capacities.rows[9]);
        grow_capacity(&mut capacities.order_geometry, self.geometry_required);
        grow_capacity(&mut capacities.order_groups, self.groups_required);
        grow_capacity(&mut capacities.order_visits, self.visits_required);
        grow_vec(&mut self.geometry, capacities.order_geometry);
        grow_vec(&mut self.groups, capacities.order_groups);
        grow_vec(&mut self.visits, capacities.order_visits);
    }
    /// The orders-group registry census of this pass, taken once by the
    /// callback before the jobs that read it are queued: the Orders job's
    /// [`Self::walk`] and the Naval job's order-of-battle copy.
    #[cfg(test)]
    pub fn walk_roots(&mut self, memory: &impl Memory, base: usize) {
        let _reads = crate::read_accounting::table(TableId::ArmyOrders);
        let begin = qpc();
        self.root_outcome = corders_group::walk_roots(memory, base, &mut self.groups);
        self.roots_walked = true;
        self.ticks = elapsed(begin);
    }
    pub(crate) fn fail_roots(&mut self, reason: &'static str) {
        self.root_outcome = Outcome::Unavailable { reason };
        self.roots_walked = true;
    }
    pub(crate) fn walk_roots_growing(&mut self, memory: &impl Memory, base: usize) {
        let _reads = crate::read_accounting::table(TableId::ArmyOrders);
        let begin = qpc();
        self.root_outcome = corders_group::walk_roots_growing(memory, base, &mut self.groups);
        self.roots_walked = true;
        self.ticks = elapsed(begin);
    }
    /// The single census over all groups, as one job walks it; the hourly
    /// job walks it in parts ([`Self::prepare_parts`]).
    #[cfg(test)]
    pub fn walk(&mut self, memory: &impl Memory, base: usize) {
        let mut reads = crate::read_accounting::table(TableId::ArmyOrders);
        self.rows.outcome = NOT_WALKED;
        let begin = qpc();
        // Reuse within the pass: `walk_roots` filled the groups from this
        // memory in this same callback, which holds the game thread until
        // every job has finished, so walking the registry again gives the
        // same roots and outcome. The flag is cleared here and by `reset`,
        // so a census from another pass is never reused.
        let walk = if std::mem::take(&mut self.roots_walked) {
            corder_instance::walk_listing_successors_of(
                memory,
                base,
                &mut self.rows.rows,
                &mut self.geometry,
                &self.groups,
                self.root_outcome,
                &mut self.visits,
                &mut self.successors,
            )
        } else {
            self.ticks = 0;
            corder_instance::walk_listing_successors(
                memory,
                base,
                &mut self.rows.rows,
                &mut self.geometry,
                &mut self.groups,
                &mut self.visits,
                &mut self.successors,
            )
        };
        self.rows.outcome = walk.outcome;
        self.root_outcome = walk.root_outcome;
        self.geometry_len = walk.geometry_len;
        self.geometry_required = walk.required_geometry;
        self.groups_required = walk.required_groups;
        self.visits_required = walk.minimum_visits.max(walk.visited.saturating_mul(2));
        self.ticks += elapsed(begin);
        // The census gathered the plans' order records while it held each
        // order: that is ArmyOrderPlans work, charged there, not here. The
        // plans add these ticks when they visit this census.
        let (plan_reads, plan_ticks) = self.successors.plan_gathers();
        self.ticks = self.ticks.saturating_sub(plan_ticks);
        reads.select(TableId::ArmyOrderPlans);
        plan_reads.add_current();
    }
    /// Terminal worker/admission failure; retain allocations and growth demand.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.rows.outcome = outcome;
        self.root_outcome = outcome;
        self.roots_walked = false;
        self.geometry_len = 0;
        self.ticks = 0;
        self.failed = true;
    }
    /// Before the root census job: this pass's `split`, the one the last
    /// stitched hour planned, and the session's QPC `frequency`.
    pub(crate) fn begin_parts(&mut self, split: crate::orders_parts::Split, frequency: u64) {
        self.split = split;
        self.frequency = frequency;
        self.next_split = crate::orders_parts::Split::default();
    }
    /// After the root census, on its worker: this pass's parts, each with
    /// the roots and its range of them under this pass's split. A part that
    /// cannot be prepared fails the census.
    pub(crate) fn prepare_parts(&mut self) {
        let count = self.split.parts();
        let ranges = self.split.ranges(self.roots());
        let mut result = Ok(());
        for (index, range) in ranges.into_iter().enumerate().take(count) {
            if self.parts[index].is_none() {
                match engine::allocation::boxed(crate::orders_parts::Part::new()) {
                    Ok(part) => self.parts[index] = Some(part),
                    Err(_) => {
                        result = Err("army order part allocation failed");
                        break;
                    }
                }
            }
            let roots = &self.groups[..self.root_outcome.len().min(self.groups.len())];
            if let Some(part) = self.parts[index].as_deref_mut()
                && let Err(reason) = part.take_roots(roots, self.root_outcome, range)
            {
                result = Err(reason);
                break;
            }
        }
        match result {
            Ok(()) => self.part_count = count,
            Err(reason) => {
                self.part_count = 0;
                self.fail(reason);
            }
        }
    }
    /// The next hour's split: the one this pass's stitch planned, or one
    /// part when it planned none.
    pub(crate) fn next_split(&self) -> crate::orders_parts::Split {
        self.next_split
    }
    /// This pass's part count; zero when its parts could not be prepared.
    pub(crate) fn part_count(&self) -> usize {
        self.part_count
    }
    /// A failed or refused job failed this pass's census.
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }
    /// Loan part `index` to its OrdersPart job.
    pub(crate) fn take_part(&mut self, index: usize) -> Option<Box<crate::orders_parts::Part>> {
        self.parts.get_mut(index)?.take()
    }
    /// Its OrdersPart job's part, back.
    pub(crate) fn return_part(&mut self, index: usize, part: Box<crate::orders_parts::Part>) {
        self.parts[index] = Some(part);
    }
    /// The Orders job: part 0's census and plan walk.
    pub(crate) fn walk_first_part(&mut self, memory: &impl Memory, base: usize) {
        if self.part_count == 0 {
            return;
        }
        if let Some(part) = self.parts[0].as_deref_mut() {
            part.run(memory, base);
        }
    }
    /// The OrdersStitch job: this pass's parts' census, stitched into the
    /// census one walk over all groups takes, which then holds the rows,
    /// geometry, visits and census order. Returns the parts' plan-walk
    /// ticks; a census already failed (a failed or refused job) stays so.
    pub(crate) fn stitch(&mut self) -> u64 {
        if self.failed {
            return 0;
        }
        let count = self.part_count;
        if count == 0 {
            // No part was prepared: the root census job failed first
            // (`fail_roots`), and the census refuses those roots as the
            // single census does.
            match corder_instance::refuse_census(self.root_outcome, &mut self.successors) {
                Some(walk) => self.install_census(walk),
                None => self.fail("army order parts unavailable"),
            }
            return 0;
        }
        let mut census_ticks = 0;
        let mut plan_ticks = 0;
        for part in self.parts[..count].iter().flatten() {
            census_ticks += part.census_ticks;
            plan_ticks += part.plan_ticks;
        }
        // The views in a fixed array, without allocating: the slots past
        // this pass's count repeat its first part and are not passed.
        let views = self
            .parts
            .each_ref()
            .map(|part| part.as_deref()?.census_part());
        let Some(first) = views[0].filter(|_| views[..count].iter().all(Option::is_some)) else {
            self.fail("army order part did not run");
            return 0;
        };
        let views = views.map(|view| view.unwrap_or(first));
        let walk = corder_instance::stitch_census(
            &views[..count],
            &mut self.rows.rows,
            &mut self.geometry,
            &mut self.visits,
            &mut self.successors,
        );
        self.install_census(walk);
        self.ticks += census_ticks;
        plan_ticks
    }
    /// The stitched or refused census's outcome and demand, as
    /// [`Self::walk`] records the single census's.
    fn install_census(&mut self, walk: corder_instance::Walk) {
        self.roots_walked = false;
        self.rows.outcome = walk.outcome;
        self.root_outcome = walk.root_outcome;
        self.geometry_len = walk.geometry_len;
        self.geometry_required = walk.required_geometry;
        self.groups_required = walk.required_groups;
        self.visits_required = walk.minimum_visits.max(walk.visited.saturating_mul(2));
    }
    /// This pass's parts' plan walks, for the plans' stitch: the first
    /// [`Self::part_count`] of a fixed array, without allocating (the slots
    /// past it repeat the first part).
    pub(crate) fn plan_parts(
        &self,
    ) -> Option<[corder_instance::PlanPart<'_>; crate::orders_parts::MAX_PARTS]> {
        let views = self
            .parts
            .each_ref()
            .map(|part| part.as_deref()?.plan_part());
        let first = views[0].filter(|_| views[..self.part_count].iter().all(Option::is_some))?;
        Some(views.map(|view| view.unwrap_or(first)))
    }
    /// After this pass's stitch: the next hour's split, from the parts'
    /// measured census and plan-walk time and the stitched census.
    pub(crate) fn plan_split(&mut self) {
        let ticks = self.parts[..self.part_count]
            .iter()
            .flatten()
            .map(|part| part.census_ticks + part.plan_ticks)
            .sum();
        self.next_split = match self.rows.outcome {
            Outcome::Rows { len } => crate::orders_parts::Split::plan(
                ticks,
                self.frequency,
                self.roots(),
                &self.rows.rows[..len],
            ),
            _ => crate::orders_parts::Split::default(),
        };
    }
    /// The authenticated group roots of the last walk.
    pub fn roots(&self) -> &[corders_group::Root] {
        &self.groups[..self.root_outcome.len().min(self.groups.len())]
    }
    pub fn visits(&self) -> &[corder_instance::Visit] {
        &self.visits
    }
    /// The successor lists of the last walk; valid only within its job.
    pub fn successor_lists(&self) -> &corder_instance::SuccessorLists {
        &self.successors
    }
}

pub struct MapCaptured {
    pub players: CaptureRows<chuman::Row>,
    pub human_controllers: CaptureRows<chuman::ControllerRow>,
    pub land_combats: CaptureRows<cland_combat::Row>,
    pub naval: ShipCensus,
    pub naval_combats: CaptureRows<cnaval_combat::Row>,
    pub naval_combat_participants: CaptureRows<cnaval_combatant::Row>,
    pub air_wings: CaptureRows<cair_wing::Row>,
    pub air_wing_names: Vec<u8>,
    pub air_wing_names_len: usize,
    air_wing_names_required: usize,
    pub(crate) air_wing_name_failures: usize,
    air_transport: cair_wing::transport_operands::Scratch,
    air_transport_capacity_charge: crate::raw_memory::Charge,
    pub orders: Orders,
    /// Raw-copy split gate for the hourly players pair (Players,
    /// HumanControllers). Set by the game thread every hour with a valid raw
    /// payload; `sample` then skips both typed walks and the decode worker
    /// fills these rows before the ready handoff. Preserved across `reset`
    /// via explicit clearing (set per hour, cleared by the worker).
    pub(crate) players_raw_mode: bool,
    /// Current callback's shared complete wing source already installed this
    /// independently bounded map projection; never a cross-hour cache.
    owned_air_captured: bool,
    /// An accepted wing acquisition failed this callback. Retain its terminal
    /// outcome until reset so neither the map nor its visitors retry the source.
    air_capture_failure: Option<&'static str>,
    pub player_names: Vec<u8>,
    pub player_names_len: usize,
    player_names_required: usize,
    pub land_participants: Vec<u64>,
    /// Each participant's unit class, beside `land_participants`.
    pub land_classes: Vec<u32>,
    pub land_participants_len: usize,
    land_participants_required: usize,
    naval_groups: Vec<usize>,
    naval_groups_required: usize,
    combat_ticks: Option<[u64; MAP_TABLES]>,
}

impl MapCaptured {
    pub(crate) fn install_owned_air(&mut self, job: &mut crate::extended::air::AirWingJob) -> u64 {
        if self.air_capture_failure.is_some() {
            return 0;
        }
        let begin = qpc();
        let len = job.map_outcome.len();
        AIR_WING_TRANSPORT_REFUSED.fetch_add(
            job.transport.failures as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        AIR_WING_TRANSPORT_BYTES.fetch_add(
            job.transport_raw_bytes,
            std::sync::atomic::Ordering::Relaxed,
        );
        if len > self.air_wings.rows.len() {
            // The allocating wing worker already owns complete storage.
            // Park the old frame allocation in its job for off-thread reuse.
            std::mem::swap(&mut self.air_wings.rows, &mut job.map_rows);
        } else {
            self.air_wings.rows[..len].copy_from_slice(&job.map_rows[..len]);
        }
        let names_swapped = job.names_len > self.air_wing_names.len();
        if names_swapped {
            std::mem::swap(&mut self.air_wing_names, &mut job.names);
            self.air_wing_names_len = job.names_len;
        }
        let names_fit = job.names_len <= self.air_wing_names.len();
        if names_fit {
            if !names_swapped {
                self.air_wing_names[..job.names_len].copy_from_slice(&job.names[..job.names_len]);
            }
            self.air_wing_names_len = job.names_len;
        } else {
            self.air_wing_names_len = 0;
            let omitted = self.air_wings.rows[..len]
                .iter()
                .filter(|row| row.name.is_some())
                .count();
            observe_air_wing_name_refusals(omitted);
            for row in &mut self.air_wings.rows[..len] {
                row.name = None;
            }
        }
        self.air_wing_names_required = job.names_required.max(job.names_len);
        self.air_wing_name_failures = job.name_failures + if names_fit { 0 } else { len };
        self.air_wings.outcome = job.map_outcome;
        self.owned_air_captured = true;
        qpc().saturating_sub(begin).max(0) as u64
    }

    /// Call before `sample` after an accepted wing acquisition fails. The
    /// terminal gate survives sampling and clears only at the next frame reset.
    pub(crate) fn fail_air_capture(&mut self, reason: &'static str) {
        self.air_capture_failure = Some(reason);
        self.owned_air_captured = false;
        self.air_wings.outcome = Outcome::Unavailable { reason };
    }

    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_capacities(&MapCapacities::default())
    }

    pub fn with_capacities(capacity: &MapCapacities) -> Self {
        let air_transport = cair_wing::transport_operands::Scratch::owned(capacity.rows[8]);
        let mut air_transport_capacity_charge = crate::raw_memory::Charge::default();
        air_transport_capacity_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: air_transport.allocated_bytes() as u64,
            ..Default::default()
        });
        Self {
            players: CaptureRows::new(capacity.rows[0]),
            human_controllers: CaptureRows::new(capacity.rows[1]),
            land_combats: CaptureRows::new(capacity.rows[2]),
            naval: ShipCensus::new(&capacity.rows[3..6]),
            naval_combats: CaptureRows::new(capacity.rows[6]),
            naval_combat_participants: CaptureRows::new(capacity.rows[7]),
            air_wings: CaptureRows::new(capacity.rows[8]),
            air_wing_names: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.air_wing_names,
            )),
            air_wing_names_len: 0,
            air_wing_names_required: 0,
            air_wing_name_failures: 0,
            air_transport,
            air_transport_capacity_charge,
            owned_air_captured: false,
            air_capture_failure: None,
            orders: Orders::new(capacity),
            player_names: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.player_names,
            )),
            player_names_len: 0,
            player_names_required: 0,
            land_participants: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.land_participants,
            )),
            land_classes: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.land_participants,
            )),
            land_participants_len: 0,
            land_participants_required: 0,
            naval_groups: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.naval_groups,
            )),
            naval_groups_required: 0,
            players_raw_mode: false,
            combat_ticks: None,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        // The containing Captured accounts for this struct's inline bytes.
        self.players.allocated_bytes()
            + self.human_controllers.allocated_bytes()
            + self.land_combats.allocated_bytes()
            + self.naval.allocated_bytes()
            + self.naval_combats.allocated_bytes()
            + self.naval_combat_participants.allocated_bytes()
            + self.air_wings.allocated_bytes()
            + self.air_wing_names.capacity()
            + self.air_transport.allocated_bytes()
            + self.orders.allocated_bytes()
            + self.player_names.capacity()
            + self.land_participants.capacity() * size_of::<u64>()
            + self.land_classes.capacity() * size_of::<u32>()
            + self.naval_groups.capacity() * size_of::<usize>()
    }

    pub fn reset(&mut self) {
        self.players.reset();
        self.human_controllers.reset();
        self.land_combats.reset();
        self.naval.reset();
        self.naval_combats.reset();
        self.naval_combat_participants.reset();
        self.air_wings.reset();
        self.air_wing_names_len = 0;
        self.air_wing_names_required = 0;
        self.air_wing_name_failures = 0;
        self.air_transport.reset_metadata();
        self.owned_air_captured = false;
        self.air_capture_failure = None;
        self.orders.reset();
        self.player_names_len = 0;
        self.player_names_required = 0;
        self.land_participants_len = 0;
        self.land_participants_required = 0;
        self.naval_groups_required = 0;
        self.players_raw_mode = false;
        self.combat_ticks = None;
    }

    pub fn outcomes(&self) -> [Outcome; MAP_TABLES] {
        [
            self.players.outcome,
            self.human_controllers.outcome,
            self.land_combats.outcome,
            self.naval.fleets.outcome,
            self.naval.task_forces.outcome,
            self.naval.ships.outcome,
            self.naval_combats.outcome,
            self.naval_combat_participants.outcome,
            self.air_wings.outcome,
            self.orders.rows.outcome,
        ]
    }

    /// Exclusive spool-thread ownership is required, including for scratch buffers.
    pub fn grow(&mut self, capacities: &mut MapCapacities) {
        self.players.grow(&mut capacities.rows[0]);
        self.human_controllers.grow(&mut capacities.rows[1]);
        self.land_combats.grow(&mut capacities.rows[2]);
        capacities.rows[3] = capacities.rows[3].max(self.naval.fleets.rows.len());
        self.naval.fleets.grow(&mut capacities.rows[3]);
        capacities.rows[4] = capacities.rows[4].max(self.naval.task_forces.rows.len());
        self.naval.task_forces.grow(&mut capacities.rows[4]);
        capacities.rows[5] = capacities.rows[5].max(self.naval.ships.rows.len());
        self.naval.ships.grow(&mut capacities.rows[5]);
        self.naval_combats.grow(&mut capacities.rows[6]);
        self.naval_combat_participants.grow(&mut capacities.rows[7]);
        capacities.rows[8] = capacities.rows[8].max(self.air_wings.rows.len());
        self.air_wings.grow(&mut capacities.rows[8]);
        let _ = self.air_transport.grow(
            self.air_wings
                .rows
                .len()
                .max(self.air_transport.required_map_capacity),
        );
        self.air_transport_capacity_charge
            .set(crate::raw_memory::Capacity {
                auxiliary_vectors: self.air_transport.allocated_bytes() as u64,
                ..Default::default()
            });
        grow_capacity(
            &mut capacities.air_wing_names,
            self.air_wing_names_required
                .max(crate::country_scale::air_wing_name_bytes()),
        );
        grow_vec(&mut self.air_wing_names, capacities.air_wing_names);
        self.orders.grow(capacities);
        grow_capacity(&mut capacities.player_names, self.player_names_required);
        grow_capacity(
            &mut capacities.land_participants,
            self.land_participants_required,
        );
        grow_capacity(&mut capacities.naval_groups, self.naval_groups_required);
        grow_vec(&mut self.player_names, capacities.player_names);
        grow_vec(&mut self.land_participants, capacities.land_participants);
        grow_vec(&mut self.land_classes, capacities.land_participants);
        grow_vec(&mut self.naval_groups, capacities.naval_groups);
    }

    /// Returns caller-clock ticks for the ten tables.
    /// Numeric values and variable rows remain native; conversion happens in map_spool.
    /// The fleet, task force and ship census. It reads live and seeds the ship
    /// models job, so it runs first in the block and the job starts at once;
    /// `sample` then reuses its outcome and ticks.
    pub fn sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &mut impl MapSources,
    ) -> [u64; MAP_TABLES] {
        let mut ticks = self.combat_ticks.unwrap_or([0; MAP_TABLES]);
        // Raw-copy split (hourly players pair): every hour the game thread
        // stages raw bytes via `copy_players_refresh` and the decode worker
        // fills both rows before the ready handoff. Skip both typed walks
        // here; copy costs install from the report in the tick. The ship and
        // order censuses run in their pool jobs, so their ticks stay zero here.
        if !self.players_raw_mode {
            let mut reads = crate::read_accounting::table(TableId::Players);
            let begin = qpc();
            let names = chuman::walk_growing(
                memory,
                base,
                state,
                &mut self.players.rows,
                &mut self.player_names,
            );
            self.players.outcome = names.outcome;
            self.player_names_len = names.bytes_len;
            self.player_names_required = names.required_bytes;
            ticks[0] = (qpc() - begin).max(0) as u64;

            reads.select(TableId::HumanControllers);
            let begin = qpc();
            self.human_controllers.outcome = chuman::walk_controllers_growing(
                memory,
                base,
                state,
                &mut self.human_controllers.rows,
            );
            ticks[1] = (qpc() - begin).max(0) as u64;
        }

        let mut reads = crate::read_accounting::table(TableId::LandCombats);
        if self.combat_ticks.is_none() {
            let begin = qpc();
            let mut nested = sources.begin_land();
            let land = cland_combat::walk_with(
                memory,
                base,
                state,
                &mut self.land_combats.rows,
                &mut self.land_participants,
                &mut self.land_classes,
                |source| nested = nested.saturating_add(sources.land(source)),
            );
            nested = nested.saturating_add(sources.finish_land(land.outcome));
            self.land_combats.outcome = land.outcome;
            self.land_participants_len = land.participants_len;
            self.land_participants_required = land.required_participants;
            ticks[2] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);

            let naval_tables = [TableId::NavalCombats, TableId::NavalCombatParticipants];
            reads.select(naval_tables[0]);
            let naval = cnaval_combat::walk_timed(
                memory,
                base,
                state,
                &mut self.naval_combats.rows,
                &mut self.naval_combat_participants.rows,
                &mut self.naval_groups,
                |phase| {
                    reads.select(naval_tables[phase]);
                    qpc() as u64
                },
            );
            self.naval_combats.outcome = naval.capture.combats;
            self.naval_combat_participants.outcome = naval.capture.participants;
            self.naval_groups_required = naval.capture.required_group_capacity;
            ticks[6..8].copy_from_slice(&naval.read_ticks);
        }
        reads.select(TableId::AirWings);
        let begin = qpc();
        if let Some(reason) = self.air_capture_failure {
            self.air_wings.outcome = Outcome::Unavailable { reason };
        } else if !self.owned_air_captured {
            let nested = std::cell::Cell::new(sources.begin_air());
            let score_limit = sources.air_score_limit();
            let source = std::cell::RefCell::new(&mut *sources);
            let mut names = cair_wing::NameArena {
                bytes: &mut self.air_wing_names,
                used: 0,
                required: 0,
                failures: 0,
            };
            let mut transport = cair_wing::transport_operands::Pass::new(
                base,
                self.air_wings.rows.len(),
                &mut self.air_transport,
            );
            let walked = cair_wing::walk_game_score_with_owners(
                memory,
                base,
                state,
                &mut self.air_wings.rows,
                Some(&mut names),
                score_limit,
                |pointer, pool, row| {
                    transport.capture(memory, pointer, pool, row);
                    nested.set(
                        nested
                            .get()
                            .saturating_add(source.borrow_mut().air(pointer, pool, row)),
                    )
                },
                |pointer, row| {
                    nested.set(
                        nested
                            .get()
                            .saturating_add(source.borrow_mut().air_score(pointer, row)),
                    )
                },
                |slot, entry| {
                    nested.set(
                        nested
                            .get()
                            .saturating_add(source.borrow_mut().air_owner(slot, entry)),
                    )
                },
            );
            self.air_wing_names_len = names.used;
            self.air_wing_names_required = names.required;
            self.air_wing_name_failures = names.failures;
            AIR_WING_TRANSPORT_REFUSED.fetch_add(
                self.air_transport.failures as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            observe_air_wing_name_refusals(names.failures);
            self.air_wings.outcome = walked.shared.map;
            let source = source.into_inner();
            let nested = nested
                .get()
                .saturating_add(source.finish_air_score(walked.score))
                .saturating_add(source.finish_air(self.air_wings.outcome));
            ticks[8] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);
        }

        ticks
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Hourly players raw-copy split (Players, HumanControllers).
//
// The acquisition worker copies raw bytes: live roster header with per-player
// identity/flag/string spans plus heap name bytes, and the human-status
// array with an authenticated country census plus per-canonical count bytes.
// No row is decoded on the game thread. The single FIFO decode worker replays
// the growing `chuman` readers against the recorded spans, so rows,
// ordinals, arenas and
// failure reasons are identical by construction (see `crate::raw`).
//
// The mirror retains original field/gate order and stops each independent
// source at its first failure. Worker growth never restarts native reads.
// ===========================================================================

/// Fault-attribution bits, one per players table in family order.
const P_PLAYERS: u32 = 0x01;
const P_CONTROLLERS: u32 = 0x02;

/// One hour of raw players payload. Worker growth extends each original
/// span before acquisition; decode grows rows and text before publication.
#[derive(Debug, Default)]
pub(crate) struct PlayersJob {
    pub log: RawLog<2>,
    pub game_state: usize,
}

impl PlayersJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(0, 0),
            game_state: 0,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Retain acquired demand for reusable storage off the game thread.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PlayersCopyReport {
    /// False only when worker-owned staging cannot finish; no native retry.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlayersCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn ppool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. Bulk spans serve enclosed sub-reads (e.g., 4-byte
/// keys inside roster elements) via `Recorded` containment.
fn pspan(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Original player fields and string gates, without retrying a failed span.
fn copy_player(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    pointer: usize,
    vtable: usize,
    keys: &mut Vec<i32>,
) -> bool {
    use engine::{chuman, msvc_string};
    let Some(key_address) = pointer.checked_add(chuman::PLAYER_KEY) else {
        return false;
    };
    let Some(country_address) = pointer.checked_add(chuman::COUNTRY_TAG) else {
        return false;
    };
    let Some(flags_address) = pointer.checked_add(chuman::FLAGS) else {
        return false;
    };
    let Some(strings_address) = pointer.checked_add(chuman::FIRST_STRING) else {
        return false;
    };
    // Fields::read acquires these five fields before validating their values.
    let Some(identity) = pspan(log, memory, Some(pointer), 8, P_PLAYERS) else {
        return false;
    };
    let Some(key) = pspan(log, memory, Some(key_address), 4, P_PLAYERS) else {
        return false;
    };
    if pspan(log, memory, Some(country_address), 4, P_PLAYERS).is_none() {
        return false;
    }
    if pspan(log, memory, Some(flags_address), 1, P_PLAYERS).is_none() {
        return false;
    }
    let Some(strings) = pspan(log, memory, Some(strings_address), 64, P_PLAYERS) else {
        return false;
    };
    let key = i32::from_le_bytes(log.bytes[key..key + 4].try_into().unwrap());
    if ppool_u64(log, identity) != vtable as u64 || key == -1 || keys.contains(&key) {
        return false;
    }
    keys.push(key);
    let mut headers = [0_u8; 64];
    headers.copy_from_slice(&log.bytes[strings..strings + 64]);
    let mut payloads = [None; 2];
    for (index, chunk) in headers.chunks_exact(32).enumerate() {
        let len = u64::from_le_bytes(chunk[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(chunk[24..32].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return false;
        }
        let Ok(len) = usize::try_from(len) else {
            return false;
        };
        if capacity == msvc_string::INLINE_CAPACITY {
            if chunk[len] != 0 {
                return false;
            }
            continue;
        }
        let Some(source) = usize::try_from(u64::from_le_bytes(chunk[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
        else {
            return false;
        };
        let Some(terminator_address) = source.checked_add(len) else {
            return false;
        };
        let Some(terminator) = pspan(log, memory, Some(terminator_address), 1, P_PLAYERS) else {
            return false;
        };
        if log.bytes[terminator] != 0 {
            return false;
        }
        payloads[index] = Some((source, len));
    }
    // Player descriptors use exact stored string payload lengths. Both
    // original descriptor terminator gates precede either payload copy.
    for (source, len) in payloads.into_iter().flatten() {
        if pspan(log, memory, Some(source), len, P_PLAYERS).is_none() {
            return false;
        }
    }
    true
}

/// Independent original passes: live roster, then the controller census.
/// A failed source preserves the other table's healthy acquisition.
fn copy_players_pass(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    copy_player_roster(log, memory, base, game_state);
    if !log.overflowed {
        copy_controller_pass(log, memory, base, game_state);
    }
}

fn copy_player_roster(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{cgame_state, chuman, cpdx_array::Array};
    log.has_bulk = true;
    // Live roster.
    let roster_start = match pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::PLAYER_RECORDS),
        16,
        P_PLAYERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[roster_start..roster_start + 16]);
    let roster = match Array::from_header(raw, chuman::STRIDE) {
        Some(a) => a,
        None => return,
    };
    let Some(vtable) = base.checked_add(chuman::VTABLE_RVA) else {
        return;
    };
    let Ok(mut keys) = engine::allocation::vec(roster.count as usize) else {
        return;
    };
    for index in 0..roster.count {
        let Some(address) = roster.element(index, chuman::STRIDE) else {
            return;
        };
        if !copy_player(log, memory, address, vtable, &mut keys) {
            return;
        }
    }
}

fn copy_controller_pass(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use std::cell::RefCell;
    struct ControllerCopy<'a, M> {
        memory: &'a M,
        log: RefCell<&'a mut RawLog<2>>,
    }
    impl<M: engine::Memory> engine::Memory for ControllerCopy<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut log = self.log.borrow_mut();
            let Some(start) = log.take(self.memory, address, output.len(), P_CONTROLLERS) else {
                return false;
            };
            output.copy_from_slice(&log.bytes[start..start + output.len()]);
            true
        }
    }
    // The fixed walk still visits every country and count with no row storage.
    // Reuse its first-failure and reserved-alias gates without another mirror.
    let _ = chuman::walk_controllers(
        &ControllerCopy {
            memory,
            log: RefCell::new(log),
        },
        base,
        game_state,
        &mut [],
    );
}

/// Copy one hourly players payload and record copy-only costs. Worker storage
/// grows before each original span; failures are never retried live.
pub(crate) fn copy_players_refresh(
    memory: &impl engine::Memory,
    ctx: PlayersCopyContext,
    job: &mut PlayersJob,
) -> PlayersCopyReport {
    // The shared raw payload is acquired once; its controller projection is owned work.
    let _reads = crate::read_accounting::table(TableId::Players);
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    job.log.enable_worker_growth();
    copy_players_pass(&mut job.log, memory, ctx.base, ctx.game_state);
    PlayersCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl MapCaptured {
    /// Apply a failed worker capture after `sample` has skipped the players
    /// walks with `players_raw_mode` set, preventing a retry on the callback.
    pub(crate) fn fail_players_capture(&mut self, reason: &'static str) {
        self.players.outcome = Outcome::Unavailable { reason };
        self.human_controllers.outcome = Outcome::Unavailable { reason };
        self.player_names_len = 0;
        self.players_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walks exactly
    /// because the identical walks run against the recorded spans. Clears
    /// `players_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_players_payload(
        &mut self,
        base: usize,
        job: &PlayersJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let walked = chuman::walk_growing(
            &recorded,
            base,
            job.game_state,
            &mut self.players.rows,
            &mut self.player_names,
        );
        self.players.outcome = walked.outcome;
        self.player_names_len = walked.bytes_len;
        self.player_names_required = walked.required_bytes;
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.human_controllers.outcome = chuman::walk_controllers_growing(
            &recorded,
            base,
            job.game_state,
            &mut self.human_controllers.rows,
        );
        decode_ticks[1] += elapsed(started);
        self.players_raw_mode = false;
    }
}

/// Hourly players split harness: one country with one inline player and one
/// controller count, plus empty and fault-injection coverage.
#[cfg(test)]
mod players_sizing_tests;

#[cfg(test)]
mod players_split_tests {
    use super::NoSources;
    use super::{
        MapCapacities, MapCaptured, PlayersCopyContext, PlayersCopyReport, PlayersJob,
        copy_players_refresh,
    };
    use engine::{ccountry, cgame_state, chuman, memory::SliceMemory, walk::Outcome};

    const BASE: usize = 0;
    const GS: usize = 0x10000;
    const ROSTER: usize = 0x20000;
    const COUNTRY_PTR: usize = 0x11000;
    const LINKS: usize = 0x12000;
    const COUNTRY: usize = 0x13000;
    const COUNTS: usize = 0x14000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - GS..address - GS + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn inline_string(bytes: &mut [u8], at: usize, value: &[u8]) {
        assert!(value.len() <= 15);
        let mut header = [0u8; 32];
        header[..value.len()].copy_from_slice(value);
        header[value.len()] = 0;
        header[16..24].copy_from_slice(&(value.len() as u64).to_le_bytes());
        header[24..32].copy_from_slice(&15_u64.to_le_bytes());
        put(bytes, at, &header);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = engine::allocation::owned_vec(engine::allocation::default_vec(0x20000));
        // Country census: 1 canonical country (slot1, id1).
        array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRY_PTR, 2);
        put(&mut bytes, COUNTRY_PTR + 8, &(COUNTRY as u64).to_le_bytes());
        array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 2);
        put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        array(&mut bytes, GS + cgame_state::HUMAN_STATUS, COUNTS, 2);
        put(&mut bytes, COUNTS + 1, &[7]);
        // One player with inline names.
        array(&mut bytes, GS + cgame_state::PLAYER_RECORDS, ROSTER, 1);
        put(
            &mut bytes,
            ROSTER,
            &(chuman::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            ROSTER + chuman::PLAYER_KEY,
            &42_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            ROSTER + chuman::COUNTRY_TAG,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, ROSTER + chuman::FLAGS, &[3]);
        inline_string(&mut bytes, ROSTER + chuman::FIRST_STRING, b"ab");
        inline_string(&mut bytes, ROSTER + chuman::NAME, b"c");
        bytes
    }

    fn capacities() -> MapCapacities {
        MapCapacities::default()
    }

    fn direct(memory: &SliceMemory<'_>) -> MapCaptured {
        let mut captured = MapCaptured::with_capacities(&capacities());
        captured.sample(memory, BASE, GS, &mut NoSources);
        captured
    }

    fn split(memory: &SliceMemory<'_>) -> (MapCaptured, PlayersCopyReport) {
        let mut captured = MapCaptured::with_capacities(&capacities());
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut job = PlayersJob::new();
        let report = copy_players_refresh(memory, ctx, &mut job);
        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        captured.decode_players_payload(BASE, &job, &mut [0; 2]);
        (captured, report)
    }

    #[test]
    fn players_split_copy_matches_direct_walk() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let direct = direct(&memory);
        let (split, report) = split(&memory);
        assert!(report.raw_ok);
        assert_eq!(split.players.outcome, direct.players.outcome);
        assert_eq!(
            split.human_controllers.outcome,
            direct.human_controllers.outcome
        );
        assert_eq!(split.players.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.human_controllers.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.players.rows[0].id, 42);
        assert_eq!(split.human_controllers.rows[0].count, 7);
        assert_eq!(&split.player_names[..split.player_names_len], b"abc");
    }

    #[test]
    fn players_cold_and_reusable_storage_match_direct() {
        // Original headers grow both direct and owned decoding before output,
        // independently of each reusable row buffer's prior length.
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        for (players, controllers) in [(0, 0), (1, 1), (0, 1), (1, 0), (64, 64)] {
            let mut caps = capacities();
            caps.rows[0] = players;
            caps.rows[1] = controllers;
            let mut direct = MapCaptured::with_capacities(&caps);
            let _ = direct.sample(&memory, BASE, GS, &mut NoSources);
            let mut captured = MapCaptured::with_capacities(&caps);
            let ctx = PlayersCopyContext {
                base: BASE,
                game_state: GS,
            };
            let mut first = PlayersJob::new();

            let report = copy_players_refresh(&memory, ctx, &mut first);

            assert!(report.raw_ok, "players={players} controllers={controllers}");
            captured.players_raw_mode = true;
            let mut ticks = [0u64; 2];
            captured.decode_players_payload(BASE, &first, &mut ticks);
            assert_eq!(
                captured.players.outcome, direct.players.outcome,
                "players outcome p={players} c={controllers}"
            );
            assert_eq!(
                captured.human_controllers.outcome, direct.human_controllers.outcome,
                "controllers outcome p={players} c={controllers}"
            );
            assert_eq!(
                &captured.players.rows[..captured.players.outcome.len()],
                &direct.players.rows[..direct.players.outcome.len()],
                "player rows"
            );
        }
        // Exact-fit arm pins Rows on both tables.
        let mut caps = capacities();
        caps.rows[0] = 1;
        caps.rows[1] = 1;
        let mut direct = MapCaptured::with_capacities(&caps);
        let _ = direct.sample(&memory, BASE, GS, &mut NoSources);
        assert_eq!(direct.players.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn players_bulk_string_pair_serves_enclosed_sub_reads() {
        // S3 containment: the 64 B player string pair is a bulk span, so the
        // walk's enclosed 32 B halves and unaligned sub-reads inside it are
        // served from the same recorded bytes. Proves bulk containment plus
        // bulk-wins-for-sub-ranges; straddling reads fail.
        use engine::Memory;
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut job = PlayersJob::new();
        let report = copy_players_refresh(
            &memory,
            PlayersCopyContext {
                base: BASE,
                game_state: GS,
            },
            &mut job,
        );
        assert!(report.raw_ok);
        assert!(job.log.has_bulk);
        let recorded = crate::raw::Recorded::build(&job.log);
        let pair = job
            .log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk && span.len == 64)
            .expect("bulk string pair");
        // Enclosed aligned 32 B half.
        let mut half = [0u8; 32];
        assert!(recorded.copy(pair.addr, &mut half));
        let mut expected = [0u8; 32];
        assert!(memory.copy(pair.addr, &mut expected));
        assert_eq!(half, expected);
        // Deliberately unaligned enclosed sub-range.
        let mut unaligned = [0u8; 7];
        assert!(recorded.copy(pair.addr + 3, &mut unaligned));
        let mut expected7 = [0u8; 7];
        assert!(memory.copy(pair.addr + 3, &mut expected7));
        assert_eq!(unaligned, expected7);
        // Straddling the pair edge fails.
        let mut straddle = [0u8; 32];
        assert!(!recorded.copy(pair.addr + 48, &mut straddle));
    }

    #[test]
    fn players_split_copy_matches_direct_walk_from_empty_rows() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut capacities = capacities();
        capacities.rows[0] = 0;
        let mut direct = MapCaptured::with_capacities(&capacities);
        direct.sample(&memory, BASE, GS, &mut NoSources);
        let mut captured = MapCaptured::with_capacities(&capacities);
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut first = PlayersJob::new();
        let report = copy_players_refresh(&memory, ctx, &mut first);

        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        let mut ticks = [0u64; 2];
        captured.decode_players_payload(BASE, &first, &mut ticks);
        assert_eq!(captured.players.outcome, direct.players.outcome);
    }

    #[test]
    fn players_staging_grows_from_cold_storage_before_publication() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
        };
        let mut tiny = PlayersJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
        };
        let report = copy_players_refresh(&memory, ctx, &mut tiny);

        assert!(report.raw_ok);
        assert!(!tiny.log.overflowed);
        let mut captured = MapCaptured::with_capacities(&capacities());
        captured.decode_players_payload(BASE, &tiny, &mut [0; 2]);
        assert_eq!(captured.players.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.human_controllers.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn players_start_without_chosen_population_extents() {
        let job = PlayersJob::new();
        assert_eq!(job.log.spans.capacity(), 0);
        assert_eq!(job.log.bytes.capacity(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downstream_consumers_receive_one_census_and_failed_air_is_not_retried() {
        #[derive(Default)]
        struct Unreadable(std::cell::Cell<usize>);
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        #[derive(Default)]
        struct Sources {
            begins: [usize; 2],
            finished: Vec<(usize, Outcome)>,
        }
        impl MapSources for Sources {
            fn begin_air(&mut self) -> u64 {
                self.begins[0] += 1;
                0
            }
            fn air(&mut self, _: usize, _: usize, _: &cair_wing::Row) -> u64 {
                panic!("unreadable census visited wing")
            }
            fn finish_air(&mut self, outcome: Outcome) -> u64 {
                self.finished.push((0, outcome));
                0
            }
            fn begin_land(&mut self) -> u64 {
                self.begins[1] += 1;
                0
            }
            fn land(&mut self, _: cland_combat::Source) -> u64 {
                panic!("unreadable census visited combat")
            }
            fn finish_land(&mut self, outcome: Outcome) -> u64 {
                self.finished.push((1, outcome));
                0
            }
        }
        let mut sources = Sources::default();
        let mut captured = MapCaptured::with_capacities(&MapCapacities::default());
        let memory = Unreadable::default();
        captured.sample(&memory, 0, 0x1000, &mut sources);
        assert_eq!(sources.begins, [1, 1]);
        assert_eq!(sources.finished.len(), 2);
        for source in 0..2 {
            assert!(
                sources
                    .finished
                    .iter()
                    .any(|&(kind, outcome)| kind == source
                        && matches!(outcome, Outcome::Unavailable { .. }))
            );
        }
        let ordinary_reads = memory.0.get();
        let ordinary_outcomes = captured.outcomes();
        memory.0.set(0);
        cair_wing::walk_with(
            &memory,
            0,
            0x1000,
            &mut captured.air_wings.rows,
            |_, _, _| {},
        );
        let wing_reads = memory.0.get();
        assert!(wing_reads > 0);
        captured.reset();
        let reason = "accepted wing worker failed";
        captured.fail_air_capture(reason);
        memory.0.set(0);
        let mut sources = Sources::default();
        captured.sample(&memory, 0, 0x1000, &mut sources);
        assert_eq!(captured.air_wings.outcome, Outcome::Unavailable { reason });
        assert_eq!(memory.0.get(), ordinary_reads - wing_reads);
        assert_eq!(sources.begins, [0, 1]);
        assert_eq!(sources.finished.len(), 1);
        for (index, outcome) in captured.outcomes().into_iter().enumerate() {
            if index != 8 {
                assert_eq!(outcome, ordinary_outcomes[index]);
            }
        }
        // The failure belongs to this callback and clears at the next reset.
        captured.reset();
        memory.0.set(0);
        let mut sources = Sources::default();
        captured.sample(&memory, 0, 0x1000, &mut sources);
        assert_eq!(memory.0.get(), ordinary_reads);
        assert_eq!(sources.begins, [1, 1]);
    }

    #[test]
    fn capacity_pressure_adds_headroom_without_shrinking_or_overflowing() {
        let mut capacity = 64;
        grow_capacity(&mut capacity, 133);
        assert_eq!(capacity, 256);
        for required in [134, 201, 256, 0] {
            grow_capacity(&mut capacity, required);
            assert_eq!(capacity, 256);
        }
        grow_capacity(&mut capacity, 257);
        assert_eq!(capacity, 512);
        grow_capacity(&mut capacity, usize::MAX);
        assert_eq!(capacity, usize::MAX);
        let mut empty = 0;
        grow_capacity(&mut empty, 3);
        assert_eq!(empty, 4);
    }

    #[test]
    fn new_buffers_use_current_primary_capacities() {
        let mut capacity = MapCapacities::default();
        capacity.rows[5] += 31;
        capacity.player_names += 19;
        capacity.land_participants += 13;
        capacity.order_geometry += 17;
        let captured = MapCaptured::with_capacities(&capacity);
        assert_eq!(captured.naval.ships.rows.len(), capacity.rows[5]);
        assert_eq!(captured.player_names.len(), capacity.player_names);
        assert_eq!(captured.land_participants.len(), capacity.land_participants);
        assert_eq!(captured.land_classes.len(), capacity.land_participants);
        assert_eq!(captured.orders.geometry.len(), capacity.order_geometry);
    }

    #[test]
    fn order_failure_keeps_storage_and_growth_demand_but_hides_sources() {
        let mut orders = Orders::new(&MapCapacities::default());
        orders.rows.outcome = Outcome::Rows { len: 1 };
        orders.root_outcome = Outcome::Rows { len: 1 };
        orders.geometry_len = 3;
        orders.geometry_required = 17;
        orders.groups_required = 23;
        orders.visits_required = 41;
        orders.ticks = 99;
        let bytes = orders.allocated_bytes();
        orders.fail("worker failed");
        let unavailable = Outcome::Unavailable {
            reason: "worker failed",
        };
        assert_eq!(orders.rows.outcome, unavailable);
        assert_eq!(orders.root_outcome, unavailable);
        assert!(orders.roots().is_empty());
        assert_eq!((orders.geometry_len, orders.ticks), (0, 0));
        assert_eq!(
            (
                orders.geometry_required,
                orders.groups_required,
                orders.visits_required
            ),
            (17, 23, 41)
        );
        assert_eq!(orders.allocated_bytes(), bytes);
    }

    #[test]
    fn pooled_growth_preserves_complete_rows_and_counts_whole_omissions() {
        let mut capacity = MapCapacities {
            player_names: 4096,
            land_participants: 8192,
            order_geometry: 16384,
            ..MapCapacities::default()
        };
        capacity.rows[5] = 4096;
        let mut captured = MapCaptured::with_capacities(&capacity);
        captured.naval.ships.rows[0].id = u64::MAX;
        captured.naval.ships.outcome = Outcome::Truncated {
            len: capacity.rows[5],
            unread_rows: 7,
        };
        captured.land_combats.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        captured.land_participants_required = capacity.land_participants + 11;
        captured.player_names_required = capacity.player_names + 17;
        captured.orders.geometry_required = capacity.order_geometry + 23;
        captured.grow(&mut capacity);
        assert_eq!(captured.naval.ships.rows.len(), 8192);

        assert_eq!(captured.naval.ships.rows[0].id, u64::MAX);
        assert_eq!(captured.land_participants.len(), 16384);
        assert_eq!(captured.land_classes.len(), 16384);

        assert_eq!(captured.player_names.len(), 8192);

        assert_eq!(captured.orders.geometry.len(), 16384 + 23);

        let mut idle = MapCaptured::new();
        idle.grow(&mut capacity);
        assert_eq!(idle.naval.ships.rows.len(), captured.naval.ships.rows.len());
        assert_eq!(
            idle.land_participants.len(),
            captured.land_participants.len()
        );
        assert_eq!(idle.player_names.len(), captured.player_names.len());
        assert_eq!(idle.orders.geometry.len(), captured.orders.geometry.len());
        assert!(idle.allocated_bytes() <= captured.allocated_bytes());
    }

    #[test]
    fn gradually_growing_tables_fit_without_first_losing_an_hour() {
        let mut capacity = 8;
        let mut table = CaptureRows::<u32>::new(capacity);
        for count in 1..=129 {
            assert!(count <= table.rows.len());
            table.rows[count - 1] = count as u32;
            table.outcome = Outcome::Rows { len: count };
            table.grow(&mut capacity);
            assert_eq!(table.rows[count - 1], count as u32);
        }
        assert!(capacity <= 129 * 3);
    }

    /// The census over the callback's registry census of the pass is the
    /// census that walks the registry itself, complete and with too little
    /// root scratch, and the walked roots serve one census only.
    #[test]
    fn orders_census_over_walked_roots_matches_its_own_registry_walk() {
        use crate::extended::bench_world::{self, Shape};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        let memory = world.slice();
        for capacity in [
            MapCapacities::default(),
            MapCapacities {
                order_groups: 3,
                ..MapCapacities::default()
            },
        ] {
            let mut own = Orders::new(&capacity);
            own.walk(&memory, world.base);
            let mut reused = Orders::new(&capacity);
            reused.walk_roots(&memory, world.base);
            assert!(reused.roots_walked);
            reused.walk(&memory, world.base);
            assert!(!reused.roots_walked);
            let observed = |orders: &Orders| {
                format!(
                    "{:?}",
                    (
                        orders.rows.outcome,
                        orders.root_outcome,
                        &orders.rows.rows[..orders.rows.outcome.len()],
                        &orders.geometry[..orders.geometry_len],
                        orders.roots(),
                        &orders.visits,
                        &orders.successors,
                        (
                            orders.geometry_required,
                            orders.groups_required,
                            orders.visits_required
                        ),
                    )
                )
            };
            assert_eq!(observed(&reused), observed(&own));
        }
        let mut orders = Orders::new(&MapCapacities::default());
        orders.walk_roots(&memory, world.base);
        orders.reset();
        assert!(!orders.roots_walked);
    }

    /// The census gathers the plans' order records, and those requests are
    /// ArmyOrderPlans work: ArmyOrders is charged exactly the requests of a
    /// census that gathers nothing, and ArmyOrderPlans the gathers' own.
    #[test]
    fn census_plan_gathers_are_charged_to_the_plans() {
        use crate::extended::bench_world::{self, Shape};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        let mut orders = Orders::new(&MapCapacities::default());
        let ((), counts) = world.counted(|memory| {
            orders.walk_roots_growing(memory, world.base);
            orders.walk(memory, world.base);
        });
        assert_eq!(
            orders.rows.outcome,
            Outcome::Rows {
                len: world.orders as usize
            }
        );
        let ((), plain) = world.counted(|memory| {
            let _reads = crate::read_accounting::table(TableId::ArmyOrders);
            corder_instance::walk(
                memory,
                world.base,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut vec![corders_group::Root::default(); world.groups as usize],
                &mut Vec::new(),
            );
        });
        let orders_index = TableId::ArmyOrders.index();
        let plans_index = TableId::ArmyOrderPlans.index();
        assert_eq!(counts[orders_index], plain[orders_index]);
        let (gathers, _) = orders.successor_lists().plan_gathers();
        assert_eq!(counts[plans_index], gathers);
        assert!(gathers.record_calls != 0 && gathers.spans >= u64::from(world.orders) * 7);
    }

    /// Each hour's order census split is the one the last stitched hour
    /// planned, whichever pooled frame either hour uses: a frame that served
    /// an earlier hour starts from the latest plan, not its own. The parts it
    /// prepares count their boxes.
    #[test]
    fn the_next_hour_takes_the_split_the_last_stitched_hour_planned() {
        use crate::extended::bench_world::{self, Shape};
        use crate::orders_parts::{MAX_PARTS, Part, Split};
        let world = bench_world::build(
            40,
            Shape {
                named: true,
                theatre_links: true,
            },
        );
        let memory = world.slice();
        let frequency = 10_000_000;
        // One hour on `orders`, from the callback's `split`, whose parts
        // measured `millis` of work; returns the split it planned.
        let hour = |orders: &mut Orders, split: Split, millis: u64| {
            orders.reset();
            orders.begin_parts(split, frequency);
            orders.walk_roots_growing(&memory, world.base);
            orders.prepare_parts();
            for index in 0..orders.part_count() {
                let mut part = orders.take_part(index).unwrap();
                part.run(&memory, world.base);
                part.census_ticks = if index == 0 {
                    frequency * millis / 1_000
                } else {
                    0
                };
                part.plan_ticks = 0;
                orders.return_part(index, part);
            }
            orders.stitch();
            assert!(!orders.failed());
            orders.plan_split();
            orders.next_split()
        };
        let mut frames = [
            Orders::for_test(&MapCapacities::default()),
            Orders::for_test(&MapCapacities::default()),
        ];
        let split = hour(&mut frames[0], Split::default(), 3);
        assert_eq!(split.parts(), MAX_PARTS);
        // The next hour, on the other frame, takes it.
        let next = hour(&mut frames[1], split, 0);
        assert_eq!(frames[1].part_count(), MAX_PARTS);
        assert!(frames[1].parts_allocated_bytes() >= MAX_PARTS * size_of::<Part>());
        assert_eq!(next.parts(), 1);
        // The first frame again: the latest plan, not the one it made.
        hour(&mut frames[0], next, 0);
        assert_eq!(frames[0].part_count(), 1);
    }

    /// A pass whose root census job failed before it prepared any part
    /// refuses its census with the roots' reason, as the single census does,
    /// rather than failing as a job.
    #[test]
    fn failed_roots_without_parts_refuse_the_census_as_the_single_census_does() {
        struct NoMemory;
        impl Memory for NoMemory {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let reason = "orders group identity unreadable";
        let mut single = Orders::for_test(&MapCapacities::default());
        single.fail_roots(reason);
        single.walk(&NoMemory, 0);
        let mut parted = Orders::for_test(&MapCapacities::default());
        parted.begin_parts(crate::orders_parts::Split::default(), 10_000_000);
        parted.fail_roots(reason);
        assert_eq!(parted.stitch(), 0);
        assert!(!parted.failed());
        assert_eq!(parted.rows.outcome, Outcome::Unavailable { reason });
        let state = |orders: &Orders| {
            (
                orders.rows.outcome,
                orders.root_outcome,
                orders.geometry_len,
                orders.geometry_required,
                orders.groups_required,
                orders.visits_required,
                orders.successor_lists().census().len(),
            )
        };
        assert_eq!(state(&parted), state(&single));
    }

    #[test]
    fn worker_order_demand_replenishes_reuse_without_allocator_headroom() {
        let mut capacity = MapCapacities::default();
        let mut orders = Orders::new(&capacity);
        orders
            .visits
            .resize(5_000, corder_instance::Visit::default());
        orders.visits.reserve_exact(100_003);
        orders.visits_required = 5_000;
        let worker_capacity = orders.visits.capacity();
        orders.grow(&mut capacity);
        assert!(capacity.order_visits >= 5_000);
        assert!(capacity.order_visits < worker_capacity);
        assert_eq!(Orders::new(&capacity).visits.len(), capacity.order_visits);
    }

    #[test]
    fn unavailable_scratch_walks_grow_without_inventing_truncated_row_counts() {
        let mut capacity = MapCapacities {
            order_groups: 2048,
            ..MapCapacities::default()
        };
        let mut captured = MapCaptured::with_capacities(&capacity);
        captured.orders.rows.outcome = Outcome::Unavailable {
            reason: "orders scratch full",
        };
        let initial_group_capacity = capacity.order_groups;
        captured.orders.groups_required = initial_group_capacity + 1;
        captured.orders.visits_required = 4096;
        captured.naval_combat_participants.outcome = Outcome::Unavailable {
            reason: "naval groups scratch full",
        };
        captured.naval_groups_required = 129;
        captured.grow(&mut capacity);
        assert_eq!(captured.orders.groups.len(), initial_group_capacity * 2);
        assert_eq!(captured.orders.visits.len(), 4096);
        assert_eq!(captured.naval_groups.len(), 256);
        // Slowly increasing native counts reuse headroom instead of making
        // every newly created group or order unavailable for another hour.
        let grown = capacity;
        captured.orders.groups_required = initial_group_capacity + 47;
        captured.orders.visits_required = 3500;
        captured.naval_groups_required = 201;
        captured.grow(&mut capacity);
        assert!(capacity == grown);
        assert!(matches!(
            captured.orders.rows.outcome,
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            captured.naval_combat_participants.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn map_pack_tables_are_hourly_tier_h_with_no_refresh_gate() {
        // The eight map-pack tables keep hourly cadence: no stagger slot,
        // every hour walks. This pins the unchanged typed cadence.
        use replay::rows::TableId;
        for table in [
            TableId::LandCombats,
            TableId::Fleets,
            TableId::TaskForces,
            TableId::Ships,
            TableId::NavalCombats,
            TableId::NavalCombatParticipants,
            TableId::AirWings,
            TableId::ArmyOrders,
        ] {
            assert!(!replay::cadence::is_staggered(table), "{table:?}");
            for hour in [0, 1, 2, 3, 60_820_531] {
                assert!(
                    replay::cadence::is_refresh(table, hour),
                    "{table:?} at {hour}"
                );
            }
        }
    }
}
