pub use crate::corder_instance::{DivisionOrderTransportInputs, OrderExecutionInputs, ParadropPreparationInputs, ParadropPreparationTerms, ParadropTransportModifiers};

use crate::{
    Memory,
    carmy,
    carmy_leader,
    ccountry,
    cdiplomacy_status,
    corder_instance::{self as order, AssignedSources, PlanRow},
    corders_group,
    corders_group_member as member,
    cpdx_array::Array,
    cprovince,
    crelation::status as crelation_status,
    cstate,
    csub_unit_definition,
    cunit,
    modifier_accumulator::{SnapshotArena, SnapshotCapacity},
    modifier_definition::{self, DefinitionMemo},
    modifier_keys,
    read,
    walk::Outcome,
};
use engine_sites::Rva;
use std::{
    cell::{Cell, RefCell},
    mem::size_of,
};

mod demand;
pub use demand::{Demand, demand};

const NAVAL_WEIGHT: usize = 0x2b8;
const PARADROP_ELIGIBLE: usize = 0x5b5;

pub const STAT_REFUSALS: [&str; 5] = [
    "execution stat source unavailable",
    "execution stat identity unreadable",
    "execution stat identity mismatch",
    "execution naval weight unreadable",
    "execution paradrop eligibility unreadable",
];


#[derive(Clone, Copy)]
pub struct Orders<'a> {
    pub roots: &'a [corders_group::Root],
    pub root_source: Outcome,
    pub visits: &'a [order::Visit],
    pub rows: &'a [order::Row],
    pub source: Outcome,
    pub geometry: &'a [u32],
    pub plan_rows: &'a [u32],
    pub census: &'a [u32],
}
#[derive(Clone, Copy)]
pub struct SourceView<'a> {
    pub plans: &'a [PlanRow],
    pub source: Outcome,
    pub groups: &'a [corders_group::PlanRow],
    pub group_source: Outcome,
    pub primary: &'a [u32],
    pub assigned: &'a [u64],
    pub assigned_source: Outcome,
    pub hosts: &'a AssignedSources,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub units: usize,
    pub groups: usize,
    pub orders: usize,
    pub memberships: usize,
    pub scalar_observations: usize,
    pub modifiers: SnapshotCapacity,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Work {
    pub route_demand: usize,
    pub route_available: usize,
    pub progress_demand: usize,
    pub progress_available: usize,
    pub terms_demand: usize,
    pub terms_available: usize,
    pub assigned_occurrences: usize,
    pub scoped_units: usize,
    pub identity_refused: usize,
    pub naval_demand: usize,
    pub naval_available: usize,
    pub eligibility_demand: usize,
    pub eligibility_available: usize,
    pub stat_refusals: [usize; STAT_REFUSALS.len()],
    pub province_demand: usize,
    pub province_available: usize,
    pub modifiers_demand: usize,
    pub modifiers_available: usize,
    pub root_queries: usize,
    pub root_refused: usize,
    pub selectors: usize,
    pub selectors_some: usize,
    pub selectors_none: usize,
    pub selectors_refused: usize,
    pub stale_clean: usize,
    pub scalar_reads: usize,
    pub scalar_failed: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitRow {
    pub division: u64,
    pub inputs: Option<DivisionOrderTransportInputs>,
}
pub struct UnitPacket {
    rows: Vec<UnitRow>,
    outcome: Outcome,
}
impl UnitPacket {
    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            outcome: Outcome::Unavailable {
                reason: "order transports not captured",
            },
        }
    }
    pub fn rows(&self) -> &[UnitRow] {
        &self.rows
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn reset(&mut self) {
        self.rows.clear();
        self.fail("order transports not captured");
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.clear();
        self.outcome = Outcome::Unavailable { reason };
    }
    pub fn try_reserve(&mut self, units: usize) -> std::io::Result<()> {
        self.reset();
        crate::allocation::reserve_exact(&mut self.rows, units)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<UnitRow>()
    }
}

#[derive(Clone, Copy)]
struct ScalarObservation {
    address: usize,
    len: usize,
    bytes: [u8; 16],
    ok: bool,
}
#[derive(Default)]
struct ScalarState {
    observations: Vec<ScalarObservation>,
    index: Vec<usize>,
    required: usize,
    failed: usize,
    limit: usize,
}
#[derive(Default)]
struct Scalars(RefCell<ScalarState>);
impl Scalars {
    fn index_slot(state: &ScalarState, address: usize, len: usize) -> usize {
        let mut slot =
            (address >> 2).wrapping_mul(0x9e37_79b9).wrapping_add(len) & (state.index.len() - 1);
        while state.index[slot] != 0 {
            let value = state.observations[state.index[slot] - 1];
            if (value.address, value.len) == (address, len) {
                break;
            }
            slot = (slot + 1) & (state.index.len() - 1);
        }
        slot
    }

    fn grow(state: &mut ScalarState) -> std::io::Result<()> {
        let limit = state
            .limit
            .checked_mul(2)
            .ok_or_else(crate::allocation::oom)?
            .max(crate::allocation::add(state.observations.len(), 1)?);
        let slots = limit
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(crate::allocation::oom)?;
        let additional = limit - state.observations.len();
        crate::allocation::reserve_exact(&mut state.observations, additional)?;
        let index = crate::allocation::filled(slots, 0)?;
        state.index = index;
        state.limit = limit;
        for (code, value) in state.observations.iter().enumerate() {
            let slot = Self::index_slot(state, value.address, value.len);
            state.index[slot] = code + 1;
        }
        Ok(())
    }
    fn new(capacity: usize) -> std::io::Result<Self> {
        let slots = if capacity == 0 {
            0
        } else {
            capacity
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(crate::allocation::oom)?
        };
        Ok(Self(RefCell::new(ScalarState {
            observations: crate::allocation::vec(capacity)?,
            index: crate::allocation::filled(slots, 0)?,
            limit: capacity,
            ..Default::default()
        })))
    }
    fn clear(&mut self) {
        let state = self.0.get_mut();
        state.observations.clear();
        state.index.fill(0);
        state.required = 0;
        state.failed = 0;
    }
    fn copy(&self, memory: &impl Memory, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        if destination.len() > 16 {
            return false;
        }
        let mut state = self.0.borrow_mut();
        let mut slot = if state.index.is_empty() {
            0
        } else {
            (address >> 2)
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(destination.len())
                & (state.index.len() - 1)
        };
        if !state.index.is_empty() {
            while state.index[slot] != 0 {
                let value = state.observations[state.index[slot] - 1];
                if (value.address, value.len) == (address, destination.len()) {
                    if value.ok {
                        destination.copy_from_slice(&value.bytes[..value.len]);
                    }
                    return value.ok;
                }
                slot = (slot + 1) & (state.index.len() - 1);
            }
        }
        state.required = state
            .required
            .max(state.observations.len().saturating_add(1));
        if state.observations.len() == state.limit {
            if Self::grow(&mut state).is_err() {
                return false;
            }
            slot = Self::index_slot(&state, address, destination.len());
        }
        let mut value = ScalarObservation {
            address,
            len: destination.len(),
            bytes: [0; 16],
            ok: false,
        };
        value.ok = address != 0
            && address.checked_add(value.len).is_some()
            && memory.copy(address, &mut value.bytes[..value.len]);
        if value.ok {
            destination.copy_from_slice(&value.bytes[..value.len]);
        } else {
            state.failed += 1;
        }
        let code = state.observations.len() + 1;
        state.observations.push(value);
        state.index[slot] = code;
        value.ok
    }
    fn allocated_bytes(&self) -> usize {
        let state = self.0.borrow();
        state.observations.capacity() * size_of::<ScalarObservation>()
            + state.index.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy)]
struct Membership {
    group: u64,
    pointer: usize,
    position: usize,
    plan: u128,
}
type RootResult = Option<Result<usize, &'static str>>;
type CommanderResult = Option<Result<Option<usize>, &'static str>>;
type ProvinceResult = Option<Result<u32, &'static str>>;

pub struct Prepared {
    capacity: Capacity,
    required: Capacity,
    base: usize,
    state: usize,
    bound: bool,
    groups: Vec<(usize, usize)>,
    visits: Vec<(u128, usize)>,
    pointers: Vec<usize>,
    handles: Vec<usize>,
    invalid: Vec<Option<&'static str>>,
    memberships: Vec<Membership>,
    membership_error: Option<&'static str>,
    index_error: Option<&'static str>,
    roots: RefCell<Vec<RootResult>>,
    commanders: RefCell<Vec<CommanderResult>>,
    provinces: RefCell<Vec<ProvinceResult>>,
    attempted: Vec<u8>,
    units: UnitPacket,
    scalars: Scalars,
    snapshots: RefCell<SnapshotArena>,
    definitions: DefinitionMemo,
    definition_errors: Cell<[Option<&'static str>; 3]>,
    work: Cell<Work>,
}
impl Prepared {
    pub fn vacant() -> Self {
        Self {
            capacity: Capacity::default(),
            required: Capacity::default(),
            base: 0,
            state: 0,
            bound: false,
            groups: Vec::new(),
            visits: Vec::new(),
            pointers: Vec::new(),
            handles: Vec::new(),
            invalid: Vec::new(),
            memberships: Vec::new(),
            membership_error: None,
            index_error: None,
            roots: RefCell::new(Vec::new()),
            commanders: RefCell::new(Vec::new()),
            provinces: RefCell::new(Vec::new()),
            attempted: Vec::new(),
            units: UnitPacket::vacant(),
            scalars: Scalars::default(),
            snapshots: RefCell::new(SnapshotArena::vacant()),
            definitions: DefinitionMemo::default(),
            definition_errors: Cell::new([None; 3]),
            work: Cell::new(Work::default()),
        }
    }
    pub fn try_new(capacity: Capacity) -> std::io::Result<Self> {
        let mut units = UnitPacket::vacant();
        units.try_reserve(capacity.units)?;
        Ok(Self {
            capacity,
            groups: crate::allocation::vec(capacity.groups)?,
            visits: crate::allocation::vec(capacity.orders)?,
            pointers: crate::allocation::vec(capacity.units)?,
            handles: crate::allocation::vec(capacity.units)?,
            invalid: crate::allocation::filled(capacity.units, None)?,
            memberships: crate::allocation::vec(capacity.memberships)?,
            roots: RefCell::new(crate::allocation::filled(capacity.units, None)?),
            commanders: RefCell::new(crate::allocation::filled(capacity.units, None)?),
            provinces: RefCell::new(crate::allocation::filled(capacity.units, None)?),
            attempted: crate::allocation::filled(capacity.units, 0)?,
            units,
            scalars: Scalars::new(capacity.scalar_observations)?,
            snapshots: RefCell::new(SnapshotArena::try_new(capacity.modifiers)?),
            definitions: DefinitionMemo::try_with_capacity(3)?,
            ..Self::vacant()
        })
    }
    pub fn capacity(&self) -> Capacity {
        Capacity {
            scalar_observations: self.scalars.0.borrow().limit,
            modifiers: self.snapshots.borrow().capacity(),
            ..self.capacity
        }
    }
    pub fn required_capacity(&self) -> Capacity {
        let mut required = self.required;
        required.scalar_observations = self.scalars.0.borrow().required;
        required.modifiers = self.snapshots.borrow().required_capacity();
        required
    }
    pub fn reset(&mut self) {
        self.bound = false;
        self.required = Capacity::default();
        self.groups.clear();
        self.visits.clear();
        self.invalid.fill(None);
        self.memberships.clear();
        self.membership_error = None;
        self.index_error = None;
        self.roots.get_mut().fill(None);
        self.commanders.get_mut().fill(None);
        self.attempted.fill(0);
        self.units.reset();
        self.provinces.get_mut().fill(None);
        self.scalars.clear();
        self.snapshots.get_mut().clear();
        self.definitions.clear();
        self.definition_errors.set([None; 3]);
        self.work.set(Work::default());
    }
    pub fn try_grow(&mut self, mut capacity: Capacity) -> std::io::Result<()> {
        capacity.units = capacity.units.max(self.capacity.units);
        capacity.groups = capacity.groups.max(self.capacity.groups);
        capacity.orders = capacity.orders.max(self.capacity.orders);
        capacity.memberships = capacity.memberships.max(self.capacity.memberships);
        capacity.scalar_observations = capacity
            .scalar_observations
            .max(self.capacity.scalar_observations);
        capacity.modifiers.nodes = capacity.modifiers.nodes.max(self.capacity.modifiers.nodes);
        capacity.modifiers.entries = capacity
            .modifiers
            .entries
            .max(self.capacity.modifiers.entries);
        if capacity != self.capacity {
            *self = Self::try_new(capacity)?;
        } else {
            self.reset();
            self.units.try_reserve(capacity.units)?;
        }
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.groups.capacity() * size_of::<(usize, usize)>()
            + self.visits.capacity() * size_of::<(u128, usize)>()
            + (self.pointers.capacity() + self.handles.capacity()) * size_of::<usize>()
            + self.invalid.capacity() * size_of::<Option<&'static str>>()
            + self.memberships.capacity() * size_of::<Membership>()
            + self.roots.borrow().capacity() * size_of::<RootResult>()
            + self.commanders.borrow().capacity() * size_of::<CommanderResult>()
            + self.provinces.borrow().capacity() * size_of::<ProvinceResult>()
            + self.attempted.capacity()
            + self.units.allocated_bytes()
            + self.scalars.allocated_bytes()
            + self.snapshots.borrow().allocated_bytes()
            + self.definitions.allocated_bytes()
    }
    pub fn work(&self) -> Work {
        let mut work = self.work.get();
        let scalars = self.scalars.0.borrow();
        work.scalar_reads = scalars.observations.len();
        work.scalar_failed = scalars.failed;
        work
    }
    fn count(&self, update: impl FnOnce(&mut Work)) {
        let mut work = self.work.get();
        update(&mut work);
        self.work.set(work);
    }
    pub fn swap_units(&mut self, units: &mut UnitPacket) {
        std::mem::swap(&mut self.units, units);
    }
    pub fn fail_units(&mut self, reason: &'static str) {
        self.units.fail(reason);
    }

    fn effective(
        &self,
        memory: &impl Memory,
        base: usize,
        accumulator: usize,
        key: u32,
    ) -> Result<i64, &'static str> {
        let slot = match key {
            modifier_keys::AIR_INVASION_PREPARATION => 0,
            modifier_keys::AIR_INVASION_PREP_DAYS => 1,
            modifier_keys::PARATROOPER_WEIGHT_FACTOR => 2,
            _ => return Err("unexpected execution modifier key"),
        };
        let value = {
            let mut arena = self.snapshots.borrow_mut();
            let observation = arena.capture_at(memory, accumulator, key)?;
            arena.value(observation)?
        };
        if !value.present {
            return Ok(0);
        }
        if let Some(reason) = self.definition_errors.get()[slot] {
            return Err(reason);
        }
        let result =
            modifier_definition::effective_memo(memory, base, key, value.raw, &self.definitions);
        if let Err(reason) = result {
            let mut errors = self.definition_errors.get();
            errors[slot] = Some(reason);
            self.definition_errors.set(errors);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_orders<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        orders: Orders<'_>,
        plans: &mut [PlanRow],
        source: Outcome,
        countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    ) {
        self.base = base;
        self.state = state;
        self.bound = true;
        for plan in plans.iter_mut() {
            plan.execution = None;
        }
        self.index_orders(orders);
        if complete(source, plans.len()).is_err()
            || complete(orders.source, orders.rows.len()).is_err()
            || complete(orders.root_source, orders.roots.len()).is_err()
        {
            return;
        }
        let observed = Observed {
            memory,
            prepared: self,
        };
        for (ordinal, (_, visit)) in order::census_orders(orders.visits, orders.census).enumerate()
        {
            let Some(group) = orders.roots.get(visit.owner) else {
                continue;
            };
            let key = (u128::from(group.handle) << 32) | u128::from(visit.instance());
            let guess = orders.plan_rows.get(ordinal).copied();
            let Ok(row_index) = order::keyed_row(orders.rows, guess, key, |row| row.key()) else {
                continue;
            };
            let row = &orders.rows[row_index];
            if !matches!(row.order_type, order::TYPE_INVASION | order::TYPE_PARADROP) {
                continue;
            }
            let Ok(plan_index) = order::keyed_row(plans, guess, key, |plan| plan.key()) else {
                continue;
            };
            self.count(|work| work.route_demand += 1);
            let route_is_ok = field::<u8>(memory, visit.pointer, order::CONDITION_CACHE).ok();
            if route_is_ok.is_some() {
                self.count(|work| work.route_available += 1);
            }
            let mut inputs = OrderExecutionInputs {
                route_is_ok,
                paradrop: None,
            };
            if row.order_type == order::TYPE_PARADROP && route_is_ok.is_some_and(|raw| raw != 0) {
                self.count(|work| work.progress_demand += 1);
                if let Ok(progress) = field(memory, visit.pointer, order::ACCUMULATED_PREPARATION) {
                    self.count(|work| {
                        work.progress_available += 1;
                        work.terms_demand += 1;
                    });
                    let terms = self
                        .preparation(&observed, base, state, orders, row, *group, countries)
                        .ok();
                    if terms.is_some() {
                        self.count(|work| work.terms_available += 1);
                    }
                    inputs.paradrop = Some(ParadropPreparationInputs { progress, terms });
                }
            }
            plans[plan_index].execution = Some(inputs);
        }
    }
    fn index_orders(&mut self, orders: Orders<'_>) {
        self.groups.clear();
        self.visits.clear();
        self.required.groups = orders.roots.len();
        self.required.orders = orders.source.len();
        let result = (|| {
            complete(orders.root_source, orders.roots.len())?;
            complete(orders.source, orders.rows.len())?;
            crate::allocation::reserve_exact(&mut self.groups, orders.roots.len())
                .map_err(|_| "execution order index allocation failed")?;
            crate::allocation::reserve_exact(&mut self.visits, orders.rows.len())
                .map_err(|_| "execution order index allocation failed")?;
            self.capacity.groups = self.capacity.groups.max(orders.roots.len());
            self.capacity.orders = self.capacity.orders.max(orders.rows.len());
            self.groups.extend(
                orders
                    .roots
                    .iter()
                    .enumerate()
                    .map(|(index, root)| (root.pointer(), index)),
            );
            self.groups.sort_unstable_by_key(|entry| entry.0);
            if self.groups.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err("execution group census aliases a pointer");
            }
            if !self.index_visits(orders, true)? {
                self.index_visits(orders, false)?;
            }
            if self.visits.len() != orders.source.len() {
                return Err("execution visit census count mismatch");
            }
            self.visits.sort_unstable_by_key(|entry| entry.0);
            if self.visits.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err("execution census repeats an order key");
            }
            Ok(())
        })();
        self.index_error = result.err();
    }
    fn index_visits(&mut self, orders: Orders<'_>, placed: bool) -> Result<bool, &'static str> {
        const UNPLACED: (u128, usize) = (u128::MAX, usize::MAX);
        self.visits.clear();
        let placed = placed
            && orders.plan_rows.len() == orders.source.len()
            && orders.source.len() <= self.capacity.orders;
        if placed {
            self.visits.resize(orders.source.len(), UNPLACED);
        }
        let mut count = 0;
        for (index, visit) in order::census_orders(orders.visits, orders.census) {
            let group = orders
                .roots
                .get(visit.owner)
                .ok_or("execution order owner missing")?;
            if count == self.capacity.orders {
                return Err("execution visit census count mismatch");
            }
            let entry = (
                (u128::from(group.handle) << 32) | u128::from(visit.instance()),
                index,
            );
            if !placed {
                self.visits.push(entry);
            } else {
                match orders
                    .plan_rows
                    .get(count)
                    .and_then(|&row| self.visits.get_mut(row as usize))
                {
                    Some(slot) if *slot == UNPLACED => *slot = entry,
                    _ => return Ok(false),
                }
            }
            count += 1;
        }
        if placed && count != orders.source.len() {
            return Err("execution visit census count mismatch");
        }
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    fn preparation(
        &self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        orders: Orders<'_>,
        row: &order::Row,
        group: corders_group::Root,
        countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    ) -> Result<ParadropPreparationTerms, &'static str> {
        let country = countries.resolve(group.country)?;
        let target = target(memory, base, state, orders.geometry, row)?;
        let country_term = self.effective(
            countries.memory(),
            base,
            at(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
            modifier_keys::AIR_INVASION_PREPARATION,
        )?;
        let relation_term = if (target as i32) > 0
            && target != country.id
            && (country.id == 0
                || countries.canonical_slot(target)? != countries.canonical_slot(country.id)?)
        {
            let relation = relation(
                memory,
                base,
                country.pointer,
                countries.canonical_slot(target)?,
            )?;
            if relation == 0 {
                None
            } else {
                Some(self.effective(
                    countries.memory(),
                    base,
                    at(relation, crelation_status::MODIFIER + 0x10)?,
                    modifier_keys::AIR_INVASION_PREPARATION,
                )?)
            }
        } else {
            None
        };
        let leader = pointer(memory, group.pointer(), corders_group::LEADER)?;
        let leader_term = if leader == 0 {
            None
        } else {
            authenticate(memory, base, leader, carmy_leader::VTABLE_RVA)?;
            if field::<u32>(memory, leader, crate::cunit_leader::TYPE)?
                != crate::cunit_leader::REFERENCE_TYPE
            {
                return Err("execution preparation leader identity mismatch");
            }
            Some(self.effective(
                countries.memory(),
                base,
                at(leader, carmy_leader::MOVEMENT_MODIFIER + 0x10)?,
                modifier_keys::AIR_INVASION_PREPARATION,
            )?)
        };
        let days_term = self.effective(
            countries.memory(),
            base,
            at(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
            modifier_keys::AIR_INVASION_PREP_DAYS,
        )?;
        Ok(ParadropPreparationTerms {
            target,
            country_term,
            relation_term,
            leader_term,
            days_term,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_units<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        seeds: &[carmy::DivisionSeed],
        source: Outcome,
        orders: Orders<'_>,
        plans: SourceView<'_>,
        countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    ) {
        self.required.units = seeds.len();
        let mut packet = std::mem::replace(&mut self.units, UnitPacket::vacant());
        let mut attempted = std::mem::take(&mut self.attempted);
        packet.reset();
        attempted.fill(0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !self.bound || (self.base, self.state) != (base, state) {
                return Err("execution sources belong to another capture pass");
            }
            complete(source, seeds.len())?;
            complete(orders.source, orders.rows.len())?;
            complete(plans.source, plans.plans.len())?;
            complete(plans.assigned_source, plans.assigned.len())?;
            complete(plans.hosts.outcome(), plans.assigned.len())?;
            if plans.hosts.pointers().len() != plans.assigned.len()
                || plans.plans.len() != orders.rows.len()
                || plans
                    .plans
                    .iter()
                    .zip(orders.rows)
                    .any(|(plan, row)| plan.key() != row.key())
            {
                return Err("execution plan/assigned source identity mismatch");
            }
            let units = seeds.len();
            packet
                .try_reserve(units)
                .map_err(|_| "execution unit allocation failed")?;
            crate::allocation::resize_default(&mut attempted, units)
                .map_err(|_| "execution unit allocation failed")?;
            let additional = units.saturating_sub(self.pointers.len());
            crate::allocation::reserve_exact(&mut self.pointers, additional)
                .map_err(|_| "execution unit allocation failed")?;
            let additional = units.saturating_sub(self.handles.len());
            crate::allocation::reserve_exact(&mut self.handles, additional)
                .map_err(|_| "execution unit allocation failed")?;
            crate::allocation::resize_default(&mut self.invalid, units)
                .map_err(|_| "execution unit allocation failed")?;
            crate::allocation::resize_default(self.roots.get_mut(), units)
                .map_err(|_| "execution unit allocation failed")?;
            crate::allocation::resize_default(self.commanders.get_mut(), units)
                .map_err(|_| "execution unit allocation failed")?;
            crate::allocation::resize_default(self.provinces.get_mut(), units)
                .map_err(|_| "execution unit allocation failed")?;
            self.capacity.units = self.capacity.units.max(units);
            self.index_units(seeds);
            let primary_demand = |row: &order::Row, plan: &PlanRow| {
                row.order_type == order::TYPE_PARADROP
                    && (row.paradrop_target as i32) > 0
                    && plan
                        .execution
                        .and_then(|value| value.route_is_ok)
                        .is_some_and(|raw| raw != 0)
            };
            if orders
                .rows
                .iter()
                .zip(plans.plans)
                .any(|(row, plan)| primary_demand(row, plan))
            {
                self.index_memberships(seeds, orders, plans);
            } else {
                self.memberships.clear();
                self.membership_error = None;
                self.required.memberships = 0;
            }
            packet.rows.resize(seeds.len(), UnitRow::default());
            let observed = UnitMemory {
                memory,
                prepared: self,
                seeds,
                orders,
                plans,
            };
            for (row, plan) in orders.rows.iter().zip(plans.plans) {
                let naval = row.order_type == order::TYPE_INVASION;
                let para = primary_demand(row, plan);
                if !naval && !para {
                    continue;
                }
                let end = plan
                    .assigned_start
                    .checked_add(plan.assigned_len)
                    .ok_or("execution assigned span overflow")?;
                let handles = plans
                    .assigned
                    .get(plan.assigned_start..end)
                    .ok_or("execution assigned span incomplete")?;
                let hosts = plans
                    .hosts
                    .pointers()
                    .get(plan.assigned_start..end)
                    .ok_or("execution host span incomplete")?;
                for (&handle, &host) in handles.iter().zip(hosts) {
                    self.count(|work| work.assigned_occurrences += 1);
                    let index = self
                        .handles
                        .binary_search_by_key(&handle, |&index| seeds[index].division)
                        .map(|slot| self.handles[slot]);
                    let Ok(index) = index else {
                        self.count(|work| work.identity_refused += 1);
                        continue;
                    };
                    if packet.rows[index].division == 0 {
                        packet.rows[index].division = handle;
                        self.count(|work| work.scoped_units += 1);
                    }
                    if self.checked_unit_index(seeds, index, host).is_err()
                        || authenticate_seed(base, &seeds[index]).is_err()
                    {
                        if attempted[index] & 8 == 0 {
                            self.count(|work| work.identity_refused += 1);
                            attempted[index] |= 8;
                        }
                        continue;
                    }
                    let inputs = packet.rows[index]
                        .inputs
                        .get_or_insert_with(DivisionOrderTransportInputs::default);
                    let seed = &seeds[index];
                    if naval && attempted[index] & 1 == 0 {
                        attempted[index] |= 1;
                        if seed.naval_transfer_membership == Some(0) {
                            self.count(|work| work.naval_demand += 1);
                            inputs.naval_weight = observed.stat::<i64>(seed, NAVAL_WEIGHT).ok();
                            if inputs.naval_weight.is_some() {
                                self.count(|work| work.naval_available += 1);
                            }
                        } else if seed.naval_transfer_membership.is_none() {
                            self.count(|work| work.naval_demand += 1);
                        }
                    }
                    if !para {
                        continue;
                    }
                    let Ok(member_pointer) = at(seed.pointer, member::HOST_OFFSET) else {
                        self.count(|work| work.root_refused += 1);
                        continue;
                    };
                    let expected = observed
                        .movement_primary_root(base, member_pointer)
                        .expect("execution adapter always supplies a root result");
                    let Ok(expected) = expected else {
                        continue;
                    };
                    let Ok(order_pointer) = self.order_pointer(orders, row.key()) else {
                        self.count(|work| work.root_refused += 1);
                        continue;
                    };
                    if expected != order_pointer {
                        continue;
                    }
                    if attempted[index] & 2 == 0 {
                        attempted[index] |= 2;
                        self.count(|work| work.eligibility_demand += 1);
                        inputs.paradrop_eligible =
                            observed.stat::<u8>(seed, PARADROP_ELIGIBLE).ok();
                        if inputs.paradrop_eligible.is_some() {
                            self.count(|work| work.eligibility_available += 1);
                        }
                    }
                    if inputs.paradrop_eligible.is_none_or(|raw| raw == 0) {
                        continue;
                    }
                    let current = self.current_province(&observed, base, index, seed);
                    let Ok(current) = current else {
                        continue;
                    };
                    if current != row.paradrop_target || attempted[index] & 4 != 0 {
                        continue;
                    }
                    attempted[index] |= 4;
                    self.count(|work| work.modifiers_demand += 1);
                    inputs.paradrop_modifiers = (|| {
                        let owner = seed
                            .roster_owner
                            .ok_or("execution roster owner unavailable")?;
                        let country = countries.resolve(owner)?;
                        let country_term = self.effective(
                            countries.memory(),
                            base,
                            at(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                            modifier_keys::PARATROOPER_WEIGHT_FACTOR,
                        )?;
                        let commander_term = match self.commander(&observed, base, index, seed)? {
                            Some(accumulator) => Some(self.effective(
                                countries.memory(),
                                base,
                                accumulator,
                                modifier_keys::PARATROOPER_WEIGHT_FACTOR,
                            )?),
                            None => None,
                        };
                        Ok::<_, &'static str>(ParadropTransportModifiers {
                            country_term,
                            commander_term,
                        })
                    })()
                    .ok();
                    if inputs.paradrop_modifiers.is_some() {
                        self.count(|work| work.modifiers_available += 1);
                    }
                }
            }
            packet.rows.retain(|row| row.division != 0);
            packet.rows.sort_unstable_by_key(|row| row.division);
            packet.rows.dedup_by_key(|row| row.division);
            packet.outcome = Outcome::Rows {
                len: packet.rows.len(),
            };
            Ok(())
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => packet.fail(reason),
            Err(payload) => {
                self.units = packet;
                self.attempted = attempted;
                std::panic::resume_unwind(payload);
            }
        }
        self.units = packet;
        self.attempted = attempted;
    }
    fn index_units(&mut self, seeds: &[carmy::DivisionSeed]) {
        self.invalid.fill(None);
        if self.pointers.len() != seeds.len()
            || self
                .pointers
                .windows(2)
                .any(|pair| seeds[pair[0]].pointer >= seeds[pair[1]].pointer)
        {
            self.pointers.clear();
            self.pointers.extend(0..seeds.len());
            self.pointers
                .sort_unstable_by_key(|&index| seeds[index].pointer);
        }
        if self.handles.len() != seeds.len()
            || self
                .handles
                .windows(2)
                .any(|pair| seeds[pair[0]].division >= seeds[pair[1]].division)
        {
            self.handles.clear();
            self.handles.extend(0..seeds.len());
            self.handles
                .sort_unstable_by_key(|&index| seeds[index].division);
        }
        for pair in self.pointers.windows(2) {
            if seeds[pair[0]].pointer == seeds[pair[1]].pointer {
                self.invalid[pair[0]] = Some("execution census aliases a unit pointer");
                self.invalid[pair[1]] = Some("execution census aliases a unit pointer");
            }
        }
        for pair in self.handles.windows(2) {
            if seeds[pair[0]].division == seeds[pair[1]].division {
                self.invalid[pair[0]] = Some("execution census aliases a unit handle");
                self.invalid[pair[1]] = Some("execution census aliases a unit handle");
            }
        }
    }
    fn unit_index(
        &self,
        seeds: &[carmy::DivisionSeed],
        handle: u64,
        pointer: usize,
    ) -> Result<usize, &'static str> {
        let slot = self
            .handles
            .binary_search_by_key(&handle, |&index| seeds[index].division)
            .map_err(|_| "execution assigned unit absent from full census")?;
        let index = self.handles[slot];
        self.checked_unit_index(seeds, index, pointer)
    }
    fn checked_unit_index(
        &self,
        seeds: &[carmy::DivisionSeed],
        index: usize,
        pointer: usize,
    ) -> Result<usize, &'static str> {
        if let Some(reason) = self.invalid[index] {
            return Err(reason);
        }
        if seeds[index].division == 0 || pointer == 0 || seeds[index].pointer != pointer {
            return Err("execution assigned handle/host identity mismatch");
        }
        Ok(index)
    }
    fn host_index(
        &self,
        seeds: &[carmy::DivisionSeed],
        pointer: usize,
    ) -> Result<usize, &'static str> {
        let slot = self
            .pointers
            .binary_search_by_key(&pointer, |&index| seeds[index].pointer)
            .map_err(|_| "execution member host absent from full census")?;
        let index = self.pointers[slot];
        self.checked_unit_index(seeds, index, pointer)
    }
    fn order_pointer(&self, orders: Orders<'_>, key: u128) -> Result<usize, &'static str> {
        if let Some(reason) = self.index_error {
            return Err(reason);
        }
        let index = self
            .visits
            .binary_search_by_key(&key, |entry| entry.0)
            .map_err(|_| "execution root order absent from current visits")?;
        let visit = orders
            .visits
            .get(self.visits[index].1)
            .ok_or("execution visit index invalid")?;
        let owner = orders
            .roots
            .get(visit.owner)
            .ok_or("execution root owner missing")?;
        if (u128::from(owner.handle) << 32) | u128::from(visit.instance()) != key
            || visit.pointer == 0
        {
            return Err("execution root owner identity mismatch");
        }
        Ok(visit.pointer)
    }
    fn index_memberships(
        &mut self,
        seeds: &[carmy::DivisionSeed],
        orders: Orders<'_>,
        plans: SourceView<'_>,
    ) {
        self.memberships.clear();
        self.required.memberships = 0;
        let result = (|| {
            if let Some(reason) = self.index_error {
                return Err(reason);
            }
            complete(orders.root_source, orders.roots.len())?;
            complete(plans.group_source, plans.groups.len())?;
            if plans
                .groups
                .windows(2)
                .any(|pair| pair[0].group >= pair[1].group)
            {
                return Err("execution primary groups not uniquely ordered");
            }
            for group in plans.groups {
                let end = group
                    .primary_start
                    .checked_add(group.primary_len)
                    .ok_or("execution primary span overflow")?;
                for &instance in plans
                    .primary
                    .get(group.primary_start..end)
                    .ok_or("execution primary span incomplete")?
                {
                    let key = (u128::from(group.group) << 32) | u128::from(instance);
                    let index = plans
                        .plans
                        .binary_search_by_key(&key, |plan| plan.key())
                        .map_err(|_| "execution primary order missing")?;
                    self.required.memberships = self
                        .required
                        .memberships
                        .checked_add(plans.plans[index].assigned_len)
                        .ok_or("execution membership count overflow")?;
                }
            }
            if self.required.memberships > self.capacity.memberships {
                crate::allocation::reserve_exact(&mut self.memberships, self.required.memberships)
                    .map_err(|_| "execution membership allocation failed")?;
                self.capacity.memberships = self.required.memberships;
            }
            for group in plans.groups {
                let end = group.primary_start + group.primary_len;
                for (position, &instance) in
                    plans.primary[group.primary_start..end].iter().enumerate()
                {
                    let key = (u128::from(group.group) << 32) | u128::from(instance);
                    self.order_pointer(orders, key)?;
                    let plan = &plans.plans[plans
                        .plans
                        .binary_search_by_key(&key, |plan| plan.key())
                        .map_err(|_| "execution primary order missing")?];
                    let end = plan
                        .assigned_start
                        .checked_add(plan.assigned_len)
                        .ok_or("execution assigned span overflow")?;
                    let handles = plans
                        .assigned
                        .get(plan.assigned_start..end)
                        .ok_or("execution assigned span incomplete")?;
                    let hosts = plans
                        .hosts
                        .pointers()
                        .get(plan.assigned_start..end)
                        .ok_or("execution host span incomplete")?;
                    for (&handle, &pointer) in handles.iter().zip(hosts) {
                        self.unit_index(seeds, handle, pointer)?;
                        self.memberships.push(Membership {
                            group: group.group,
                            pointer,
                            position,
                            plan: key,
                        });
                    }
                }
            }
            self.memberships
                .sort_unstable_by_key(|entry| (entry.group, entry.pointer, entry.position));
            Ok(())
        })();
        self.membership_error = result.err();
    }
    fn first_primary(
        &self,
        orders: Orders<'_>,
        plans: SourceView<'_>,
        owner: usize,
        host: usize,
    ) -> Result<usize, &'static str> {
        let group = self
            .groups
            .binary_search_by_key(&owner, |entry| entry.0)
            .map_err(|_| "execution member group absent from current roots")?;
        let root = orders.roots[self.groups[group].1];
        plans
            .groups
            .binary_search_by_key(&root.handle, |group| group.group)
            .map_err(|_| "execution member group primary source missing")?;
        let first = self
            .memberships
            .partition_point(|entry| (entry.group, entry.pointer) < (root.handle, host));
        match self
            .memberships
            .get(first)
            .filter(|entry| (entry.group, entry.pointer) == (root.handle, host))
        {
            Some(entry) => self.order_pointer(orders, entry.plan),
            None => Ok(0),
        }
    }
    fn commander(
        &self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        seed: &carmy::DivisionSeed,
    ) -> Result<Option<usize>, &'static str> {
        if let Some(result) = self.commanders.borrow()[index] {
            return result;
        }
        self.count(|work| work.selectors += 1);
        let result = carmy_leader::movement_accumulator_seeded(memory, base, seed);
        self.count(|work| match result {
            Ok(Some(_)) => work.selectors_some += 1,
            Ok(None) => work.selectors_none += 1,
            Err(_) => work.selectors_refused += 1,
        });
        self.commanders.borrow_mut()[index] = Some(result);
        result
    }
    fn current_province(
        &self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        seed: &carmy::DivisionSeed,
    ) -> Result<u32, &'static str> {
        if let Some(result) = self.provinces.borrow()[index] {
            return result;
        }
        self.count(|work| work.province_demand += 1);
        let result = (|| {
            let province = seed
                .location
                .ok_or("execution current province unavailable")?;
            carmy::movement_definition(memory, base, province)?;
            field::<u32>(memory, province, cprovince::ID)
        })();
        if result.is_ok() {
            self.count(|work| work.province_available += 1);
        }
        self.provinces.borrow_mut()[index] = Some(result);
        result
    }
}

fn authenticate_seed(base: usize, seed: &carmy::DivisionSeed) -> Result<(), &'static str> {
    if seed.pointer == 0
        || seed.division == 0
        || seed.handle != Some(seed.division)
        || seed.primary_vtable
            != engine_sites::at(base, carmy::VTABLE_RVA).map(|value| value as u64)
        || seed.secondary_vtable
            != engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA).map(|value| value as u64)
    {
        return Err("execution unit identity unavailable or mismatched");
    }
    if seed.class != Some(0) {
        return Err("execution unit class unavailable or unsupported");
    }
    Ok(())
}
struct UnitMemory<'a, M> {
    memory: &'a M,
    prepared: &'a Prepared,
    seeds: &'a [carmy::DivisionSeed],
    orders: Orders<'a>,
    plans: SourceView<'a>,
}
impl<M: Memory> UnitMemory<'_, M> {
    fn stat<T: crate::memory::Raw>(
        &self,
        seed: &carmy::DivisionSeed,
        offset: usize,
    ) -> Result<T, &'static str> {
        let result = (|| {
            let pointer = seed
                .stat_block
                .filter(|&pointer| pointer != 0)
                .ok_or(STAT_REFUSALS[0])?;
            let vtable = field::<u64>(self, pointer, 0).map_err(|_| STAT_REFUSALS[1])?;
            if Some(vtable)
                != engine_sites::at(self.prepared.base, csub_unit_definition::VTABLE_RVA)
                    .map(|at| at as u64)
            {
                return Err(STAT_REFUSALS[2]);
            }
            field(self, pointer, offset).map_err(|_| match offset {
                NAVAL_WEIGHT => STAT_REFUSALS[3],
                PARADROP_ELIGIBLE => STAT_REFUSALS[4],
                _ => unreachable!("execution stat leaf"),
            })
        })();
        if let Err(reason) = result {
            let index = STAT_REFUSALS
                .iter()
                .position(|&name| name == reason)
                .unwrap();
            self.prepared.count(|work| work.stat_refusals[index] += 1);
        }
        result
    }
}
impl<M: Memory> Memory for UnitMemory<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let position = self
            .prepared
            .pointers
            .partition_point(|&index| self.seeds[index].pointer <= address);
        if let Some(&index) = position
            .checked_sub(1)
            .and_then(|index| self.prepared.pointers.get(index))
        {
            let seed = &self.seeds[index];
            if let Some(offset) = address.checked_sub(seed.pointer)
                && let Some(value) = seeded_scalar(seed, offset, destination.len())
            {
                return match value {
                    Some(bytes) => {
                        destination.copy_from_slice(&bytes[..destination.len()]);
                        true
                    }
                    None => false,
                };
            }
        }
        self.prepared
            .scalars
            .copy(self.memory, address, destination)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.memory.authenticated_countries(base, state)
    }
    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.memory.authenticated_reserved_country(base, state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::ReservedCountryObservation> {
        self.memory
            .authenticated_reserved_country_observation(base, state)
    }
    fn movement_primary_root(
        &self,
        base: usize,
        member_pointer: usize,
    ) -> Option<Result<usize, &'static str>> {
        Some((|| {
            if base != self.prepared.base {
                return Err("execution member module mismatch");
            }
            let host = member_pointer
                .checked_sub(member::HOST_OFFSET)
                .ok_or("execution member host underflow")?;
            let index = self.prepared.host_index(self.seeds, host)?;
            if let Some(result) = self.prepared.roots.borrow()[index] {
                return result;
            }
            self.prepared.count(|work| work.root_queries += 1);
            let result = (|| {
                authenticate_seed(base, &self.seeds[index])?;
                if let Some(reason) = self.prepared.membership_error {
                    return Err(reason);
                }
                let own = self.seeds[index]
                    .orders_group
                    .ok_or("execution member group unavailable")?;
                let dirty = field::<u8>(self, member_pointer, member::ORDER_CACHE_DIRTY)?;
                let cached = pointer(self, member_pointer, member::ROOT_ORDER)?;
                let mut expected = 0;
                if own != 0 {
                    expected = self
                        .prepared
                        .first_primary(self.orders, self.plans, own, host)?;
                    if expected == 0 {
                        let parent = pointer(self, own, corders_group::PARENT)?;
                        if parent != 0 {
                            expected = self.prepared.first_primary(
                                self.orders,
                                self.plans,
                                parent,
                                host,
                            )?;
                        }
                    }
                }
                if dirty == 0 && cached != expected {
                    self.prepared.count(|work| work.stale_clean += 1);
                    return Err("movement primary root clean cache is stale");
                }
                Ok(expected)
            })();
            if result.is_err() {
                self.prepared.count(|work| work.root_refused += 1);
            }
            self.prepared.roots.borrow_mut()[index] = Some(result);
            result
        })())
    }
}
fn seeded_scalar(seed: &carmy::DivisionSeed, offset: usize, len: usize) -> Option<Option<[u8; 8]>> {
    let word = match (offset, len) {
        (0, 8) => seed.primary_vtable,
        (carmy::SECONDARY_BASE, 8) => seed.secondary_vtable,
        (cunit::HANDLE, 8) => seed.handle,
        (carmy::ORDERS_GROUP, 8) => seed.orders_group.map(|value| value as u64),
        (cunit::CLASS, 4) => seed.class.map(u64::from),
        (cunit::ROSTER_OWNER, 4) => seed.roster_owner.map(u64::from),
        (cunit::LOCATION, 8) => seed.location.map(|value| value as u64),
        _ => return None,
    };
    Some(word.map(u64::to_le_bytes))
}

#[cfg(test)]
mod tests;

fn complete(source: Outcome, len: usize) -> Result<(), &'static str> {
    match source {
        Outcome::Rows { len: actual } if actual == len => Ok(()),
        Outcome::Unavailable { reason } => Err(reason),
        _ => Err("execution requires a complete current source"),
    }
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("execution input address overflow or null")
}
fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("execution input unreadable")
}
fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, object, offset).map(|value| value as usize)
}
fn authenticate(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    rva: Rva,
) -> Result<(), &'static str> {
    let vtable = field::<u64>(memory, pointer, 0)?;
    if engine_sites::at(base, rva).map(|a| a as u64) == Some(vtable) {
        Ok(())
    } else {
        Err("execution object identity mismatch")
    }
}
fn target(
    memory: &impl Memory,
    base: usize,
    state: usize,
    geometry: &[u32],
    row: &order::Row,
) -> Result<u32, &'static str> {
    if row.path_len == 0 {
        return Ok(0);
    }
    let end = row
        .path_start
        .checked_add(row.path_len)
        .ok_or("execution path length overflow")?;
    let id = *geometry
        .get(row.path_start..end)
        .and_then(|path| path.first())
        .ok_or("execution path source incomplete")?;
    let provinces = Array::read(memory, at(state, crate::cgame_state::PROVINCES)?, 8)
        .ok_or("execution province array invalid")?;
    let address = provinces
        .element(id, 8)
        .filter(|_| id != 0)
        .ok_or("execution path province outside roster")?;
    let province = pointer(memory, address, 0)?;
    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
    if field::<u32>(memory, province, cprovince::ID)? != id {
        return Err("execution path province identity mismatch");
    }
    let state = pointer(memory, province, cprovince::STATE)?;
    if state == 0 {
        return Ok(0);
    }
    authenticate(memory, base, state, cstate::VTABLE_RVA)?;
    field(memory, state, cstate::CONTROLLER)
}
fn relation(
    memory: &impl Memory,
    base: usize,
    country: usize,
    slot: u32,
) -> Result<usize, &'static str> {
    let diplomacy = pointer(memory, country, ccountry::DIPLOMACY_STATUS)?;
    authenticate(memory, base, diplomacy, cdiplomacy_status::VTABLE_RVA)?;
    let rows = Array::read(memory, at(diplomacy, cdiplomacy_status::RELATIONS)?, 8)
        .ok_or("execution relation matrix invalid")?;
    let relation = pointer(
        memory,
        rows.element(slot, 8)
            .ok_or("execution relation slot outside matrix")?,
        0,
    )?;
    if relation != 0 {
        authenticate(memory, base, relation, crelation_status::VTABLE_RVA)?;
    }
    Ok(relation)
}
struct Observed<'a, M> {
    memory: &'a M,
    prepared: &'a Prepared,
}
impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.prepared
            .scalars
            .copy(self.memory, address, destination)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::AuthenticatedRoster<'_>> {
        self.memory.authenticated_countries(base, state)
    }
    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.memory.authenticated_reserved_country(base, state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<ccountry::ReservedCountryObservation> {
        self.memory
            .authenticated_reserved_country_observation(base, state)
    }
}
