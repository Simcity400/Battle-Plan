//! Hourly battle-plan operands over the shared ArmyOrders/group source census,
//! and the naval invasions' preparation operands over the same census.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{
    Memory, ccountry, cfront, cfront_section, corder_instance, corders_group,
    invasion_preparations, modifier_definition::DefinitionMemo, order_execution, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        ArmyOrderPlanRow, FrontRow, InvasionPreparationRow, OrderChildFront, OrderChildFrontInputs,
        OrdersGroupPlanRow, Recorded, TableId, tables,
    },
};
use std::mem::size_of;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupPlans,
    TableId::Fronts,
    TableId::InvasionPreparations,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

fn project_execution(
    inputs: order_execution::OrderExecutionInputs,
) -> replay::rows::OrderExecutionInputs {
    replay::rows::OrderExecutionInputs {
        route_is_ok: inputs.route_is_ok,
        paradrop: inputs
            .paradrop
            .map(|prep| replay::rows::ParadropPreparationInputs {
                progress: prep.progress,
                terms: prep
                    .terms
                    .map(|terms| replay::rows::ParadropPreparationTerms {
                        target: terms.target,
                        country_term: terms.country_term,
                        relation_term: terms.relation_term,
                        leader_term: terms.leader_term,
                        days_term: terms.days_term,
                    }),
            }),
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    plans: usize,
    groups: usize,
    fronts: usize,
    assigned: usize,
    child_sections: usize,
    links: usize,
    names: usize,
    root_orders: usize,
    sections: usize,
    pairs: usize,
    handles: usize,
    /// InvasionPreparations rows: the census's naval invasion orders.
    invasions: usize,
    /// Their route unit owners' nonzero co-belligerent target slots.
    co_belligerents: usize,
    execution: order_execution::Capacity,
    country_references: usize,
    country_links: usize,
}
impl Capacities {
    pub(crate) fn presize_execution(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let observed = order_execution::demand(memory, base, state);
        let demand = observed.capacity;
        grow_capacity(&mut self.execution.units, demand.units);
        grow_capacity(&mut self.execution.groups, demand.groups);
        grow_capacity(&mut self.execution.orders, demand.orders);
        grow_capacity(&mut self.execution.memberships, demand.memberships);
        grow_capacity(
            &mut self.execution.scalar_observations,
            demand.scalar_observations,
        );
        grow_capacity(&mut self.execution.modifiers.nodes, demand.modifiers.nodes);
        grow_capacity(
            &mut self.execution.modifiers.entries,
            demand.modifiers.entries,
        );
        grow_capacity(&mut self.groups, demand.groups);
        grow_capacity(&mut self.plans, demand.orders);
        grow_capacity(&mut self.fronts, demand.orders);
        grow_capacity(&mut self.handles, demand.orders);
        grow_capacity(&mut self.invasions, demand.orders);
        observed.failure.map_or(Ok(()), Err)
    }
    /// Existing country_scale observation already bounds native country,
    /// link and tag widths. Reuse that bound without another source census.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.country_references = self.country_references.max(slots);
        self.country_links = self.country_links.max(slots);
    }
}
pub struct Captured {
    plans: CaptureRows<corder_instance::PlanRow>,
    colors: CaptureRows<corders_group::PlanRow>,
    fronts: CaptureRows<cfront::Row>,
    invasions: CaptureRows<invasion_preparations::Row>,
    co_belligerents: Arena<u32>,
    /// The invasion terms' modifier metadata, read once per pass.
    memo: DefinitionMemo,
    assigned: Arena<u64>,
    child_sections: Arena<corder_instance::child_fronts::Section>,
    child_scratch: corder_instance::child_fronts::Scratch,
    /// Area/header/payload failures are field-local; the core row still stands.
    pub(crate) child_front_failures: [usize; 3],
    links: Arena<u32>,
    /// ArmyOrderPlans' order-name bytes and OrdersGroupPlans' group-name bytes,
    /// each in its own arena that the walker grows from `len`. Rows hold
    /// `(start, len)` spans that tile each arena's used prefix in walk order;
    /// bytes past `len` are stale and never published. Both share
    /// `Capacities::names`.
    names: Arena<u8>,
    group_names: Arena<u8>,
    root_orders: Arena<u32>,
    sections: Arena<cfront_section::Row>,
    pairs: Arena<u32>,
    handles: Vec<u64>,
    /// Sort scratch for the ArmyOrderPlans rows; grows to the row count.
    order: Vec<(u128, u32)>,
    required_handles: usize,
    ticks: [u64; N],
    supplied: bool,
    /// Original assigned-host provenance and execution observations move
    /// intact from Orders to Late. No native pointer enters a replay row.
    assigned_hosts: corder_instance::AssignedSources,
    execution: order_execution::Prepared,
    execution_countries: ccountry::NativeReferenceMemo,
}

/// Bounded cumulative optional-field diagnostics, observed off-thread before growth.
#[derive(Default)]
pub(crate) struct ChildFrontWork {
    samples: u64,
    core_rows: u64,
    refused: [u64; 3],
    last_sections: usize,
    last_capacity_bytes: usize,
}
impl ChildFrontWork {
    pub(crate) fn observe(&mut self, capture: &Captured) {
        if !capture.supplied {
            return;
        }
        self.samples = self.samples.saturating_add(1);
        if let Outcome::Rows { len } = capture.plans.outcome {
            self.core_rows = self.core_rows.saturating_add(len as u64);
        }
        for (total, current) in self.refused.iter_mut().zip(capture.child_front_failures) {
            *total = total.saturating_add(current as u64);
        }
        self.last_sections = capture.child_sections.len;
        self.last_capacity_bytes =
            capture.child_sections.allocated_bytes() + capture.child_scratch.allocated_bytes();
    }
    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "scope": "cumulative consumed Orders acquisitions; optional failures preserve core rows",
            "samples": self.samples, "core_rows": self.core_rows,
            "area_refused": self.refused[0], "header_refused": self.refused[1],
            "payload_refused": self.refused[2], "last_used_sections": self.last_sections,
            "last_retained_capacity_bytes": self.last_capacity_bytes,
        })
    }
}

/// Spool-side cumulative work over consumed captures, before off-hook growth.
/// Plain counters mirror the engine packet; no live observer or event log.
#[derive(Default)]
pub(crate) struct ExecutionWork {
    samples: u64,
    total: order_execution::Work,
    /// Latest consumed pass; zero scalar refusals do not certify independent
    /// modifier/country storage, source completeness or native field success.
    last_scalar_observations: usize,
    last_scalar_failed: usize,
    last_scalar_capacity_refused: usize,
    required: order_execution::Capacity,
    capacity: order_execution::Capacity,
    hosts_used: usize,
    hosts_required: usize,
    hosts_failure: Option<&'static str>,
    raw_used: usize,
    raw_required: usize,
    raw_capacity: usize,
    raw_lower_bound: bool,
    canonical_used: usize,
    links_used: usize,
    links_required: usize,
    links_capacity: usize,
    retained_bytes: usize,
    output_bytes: usize,
    output_failures: u64,
    output_failure: Option<&'static str>,
}
impl ExecutionWork {
    pub(crate) fn observe(&mut self, capture: &Captured, output_bytes: usize, output: Outcome) {
        if !capture.supplied {
            return;
        }
        self.samples = self.samples.saturating_add(1);
        let current = capture.execution.work();
        self.last_scalar_observations = current.scalar_reads;
        self.last_scalar_failed = current.scalar_failed;
        self.last_scalar_capacity_refused = current.scalar_capacity;
        macro_rules! add { ($($field:ident),+ $(,)?) => {$(
            self.total.$field = self.total.$field.saturating_add(current.$field);
        )+}; }
        add!(
            route_demand,
            route_available,
            progress_demand,
            progress_available,
            terms_demand,
            terms_available,
            assigned_occurrences,
            scoped_units,
            identity_refused,
            naval_demand,
            naval_available,
            eligibility_demand,
            eligibility_available,
            modifiers_demand,
            modifiers_available,
            province_demand,
            province_available,
            root_queries,
            root_refused,
            selectors,
            selectors_some,
            selectors_none,
            selectors_refused,
            stale_clean,
            scalar_reads,
            scalar_failed,
            scalar_capacity
        );
        for (total, count) in self
            .total
            .stat_refusals
            .iter_mut()
            .zip(current.stat_refusals)
        {
            *total = total.saturating_add(count);
        }
        self.required = capture.execution.required_capacity();
        self.capacity = capture.execution.capacity();
        let sources = capture.execution_sources();
        self.hosts_used = sources.hosts.used();
        self.hosts_required = sources.hosts.required();
        self.hosts_failure = match sources.hosts.outcome() {
            Outcome::Unavailable { reason } => Some(reason),
            _ => None,
        };
        self.raw_used = capture.execution_countries.raw_used();
        self.raw_required = capture.execution_countries.raw_required();
        self.raw_capacity = capture.execution_countries.raw_capacity();
        self.raw_lower_bound = capture.execution_countries.raw_required_is_lower_bound();
        self.canonical_used = capture.execution_countries.canonical_used();
        self.links_used = capture.execution_countries.link_used();
        self.links_required = capture.execution_countries.link_required();
        self.links_capacity = capture.execution_countries.link_capacity();
        self.output_bytes = output_bytes;
        self.retained_bytes = capture.execution.allocated_bytes()
            + capture.execution_countries.allocated_bytes()
            + capture.assigned_hosts.allocated_bytes()
            + output_bytes;
        self.output_failure = match output {
            Outcome::Unavailable { reason } => {
                self.output_failures = self.output_failures.saturating_add(1);
                Some(reason)
            }
            _ => None,
        };
    }
    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        let total = &self.total;
        crate::status::status!({
            "scope": "consumed hourly plan execution captures; optional refusals preserve established prefixes",
            "samples": self.samples,
            "route_demand": total.route_demand, "route_available": total.route_available,
            "progress_demand": total.progress_demand, "progress_available": total.progress_available,
            "terms_demand": total.terms_demand, "terms_available": total.terms_available,
            "assigned_occurrences": total.assigned_occurrences, "scoped_units": total.scoped_units,
            "identity_refused": total.identity_refused,
            "naval_demand": total.naval_demand, "naval_available": total.naval_available,
            "eligibility_demand": total.eligibility_demand, "eligibility_available": total.eligibility_available,
            "stat_refusals": crate::status::map(order_execution::STAT_REFUSALS.into_iter().zip(total.stat_refusals)),
            "modifiers_demand": total.modifiers_demand, "modifiers_available": total.modifiers_available,
            "province_demand": total.province_demand, "province_available": total.province_available,
            "root_queries": total.root_queries, "root_refused": total.root_refused,
            "selectors": total.selectors, "selectors_some": total.selectors_some,
            "selectors_none": total.selectors_none, "selectors_refused": total.selectors_refused,
            "stale_clean": total.stale_clean,
            "scalar_observations": total.scalar_reads, "scalar_failed": total.scalar_failed,
            "scalar_capacity_refused": total.scalar_capacity,
            "last_scalar_observations": self.last_scalar_observations,
            "last_scalar_failed": self.last_scalar_failed,
            "last_scalar_capacity_refused": self.last_scalar_capacity_refused,
            "last_scalar_requirement_is_lower_bound": self.last_scalar_capacity_refused != 0,
            "last_assigned_hosts_used": self.hosts_used, "last_assigned_hosts_required": self.hosts_required,
            "last_assigned_hosts_failure": self.hosts_failure,
            "last_raw_countries_used": self.raw_used, "last_raw_countries_required": self.raw_required,
            "last_raw_countries_capacity": self.raw_capacity, "last_raw_requirement_is_lower_bound": self.raw_lower_bound,
            "last_canonical_countries_used": self.canonical_used,
            "last_links_used": self.links_used, "last_links_required": self.links_required,
            "last_links_capacity": self.links_capacity,
            "last_required": capacity_status(&self.required), "last_capacity": capacity_status(&self.capacity),
            "last_retained_execution_bytes": self.retained_bytes,
            "last_presentation_output_bytes": self.output_bytes,
            "output_failures": self.output_failures, "last_output_failure": self.output_failure,
        })
    }
}
fn capacity_status(capacity: &order_execution::Capacity) -> impl serde::Serialize + '_ {
    crate::status::status!({
        "units": capacity.units, "groups": capacity.groups, "orders": capacity.orders,
        "memberships": capacity.memberships, "scalar_observations": capacity.scalar_observations,
        "modifier_nodes": capacity.modifiers.nodes, "modifier_entries": capacity.modifiers.entries,
    })
}

impl Captured {
    /// Placeholder while the Orders job owns the buffers.
    pub fn vacant() -> Self {
        Self::new(
            &Capacities {
                plans: 0,
                groups: 0,
                fronts: 0,
                assigned: 0,
                child_sections: 0,
                links: 0,
                names: 0,
                root_orders: 0,
                sections: 0,
                pairs: 0,
                handles: 0,
                invasions: 0,
                co_belligerents: 0,
                execution: order_execution::Capacity::default(),
                country_references: 0,
                country_links: 0,
            },
            true,
        )
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self::new(c, false)
    }
    fn new(c: &Capacities, placeholder: bool) -> Self {
        let mut capture = Self {
            plans: CaptureRows::new(c.plans),
            colors: CaptureRows::new(c.groups),
            fronts: CaptureRows::new(c.fronts),
            invasions: CaptureRows::new(c.invasions),
            co_belligerents: Arena::new(c.co_belligerents),
            memo: DefinitionMemo::with_capacity(if placeholder { 0 } else { 3 }),
            assigned: Arena::new(c.assigned),
            child_sections: Arena::new(c.child_sections),
            child_scratch: Default::default(),
            child_front_failures: [0; 3],
            links: Arena::new(c.links),
            names: Arena::new(c.names),
            group_names: Arena::new(c.names),
            root_orders: Arena::new(c.root_orders),
            sections: Arena::new(c.sections),
            pairs: Arena::new(c.pairs),
            handles: owned_vec(engine::allocation::filled(c.handles, 0)),
            order: Vec::new(),
            required_handles: 0,
            ticks: [0; N],
            supplied: false,
            assigned_hosts: if placeholder {
                corder_instance::AssignedSources::default()
            } else {
                corder_instance::AssignedSources::try_with_capacity(c.assigned).unwrap_or_default()
            },
            execution: if placeholder {
                order_execution::Prepared::vacant()
            } else {
                order_execution::Prepared::try_new(order_execution::Capacity {
                    groups: c.execution.groups.max(c.groups),
                    orders: c.execution.orders.max(c.plans),
                    memberships: c.execution.memberships.max(c.assigned),
                    ..c.execution
                })
                .unwrap_or_else(|_| order_execution::Prepared::vacant())
            },
            execution_countries: if placeholder {
                ccountry::NativeReferenceMemo::vacant()
            } else {
                ccountry::NativeReferenceMemo::try_with_capacity(
                    c.country_references,
                    c.country_links,
                )
                .unwrap_or_default()
            },
        };
        if !placeholder {
            capture.execution.enable_worker_growth();
            capture.assigned_hosts.enable_worker_growth();
            capture.execution_countries.enable_worker_growth();
        }
        capture
    }
    pub fn reset(&mut self) {
        self.plans.reset();
        self.colors.reset();
        self.fronts.reset();
        self.invasions.reset();
        self.co_belligerents.reset();
        self.memo.clear();
        self.assigned.reset();
        self.child_sections.reset();
        self.child_front_failures = [0; 3];
        self.links.reset();
        self.names.reset();
        self.group_names.reset();
        self.root_orders.reset();
        self.sections.reset();
        self.pairs.reset();
        self.required_handles = 0;
        self.ticks = [0; N];
        self.supplied = false;
        self.assigned_hosts.reset();
        self.execution.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.plans.allocated_bytes()
            + self.colors.allocated_bytes()
            + self.fronts.allocated_bytes()
            + self.invasions.allocated_bytes()
            + self.co_belligerents.allocated_bytes()
            + self.memo.allocated_bytes()
            + self.assigned.allocated_bytes()
            + self.child_sections.allocated_bytes()
            + self.child_scratch.allocated_bytes()
            + self.links.allocated_bytes()
            + self.names.allocated_bytes()
            + self.group_names.allocated_bytes()
            + self.root_orders.allocated_bytes()
            + self.sections.allocated_bytes()
            + self.pairs.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
            + self.order.capacity() * size_of::<(u128, u32)>()
            + self.assigned_hosts.allocated_bytes()
            + self.execution.allocated_bytes()
            + self.execution_countries.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.plans.grow(&mut c.plans);
        self.colors.grow(&mut c.groups);
        self.fronts.grow(&mut c.fronts);
        self.invasions.grow(&mut c.invasions);
        self.co_belligerents.grow(&mut c.co_belligerents);
        self.assigned.grow(&mut c.assigned);
        self.child_sections.grow(&mut c.child_sections);
        self.links.grow(&mut c.links);
        let mut group_names = c.names;
        self.names.grow(&mut c.names);
        self.group_names.grow(&mut group_names);
        c.names = c.names.max(group_names);
        self.root_orders.grow(&mut c.root_orders);
        self.sections.grow(&mut c.sections);
        self.pairs.grow(&mut c.pairs);
        grow_capacity(&mut c.handles, self.required_handles);
        grow_vec(&mut self.handles, c.handles);
        grow_capacity(&mut c.assigned, self.assigned_hosts.required());
        let _ = self.assigned_hosts.try_grow(c.assigned);
        let required = self.execution.required_capacity();
        let admitted = self.execution.capacity();
        c.execution.units = c.execution.units.max(admitted.units);
        c.execution.groups = c.execution.groups.max(admitted.groups);
        c.execution.orders = c.execution.orders.max(admitted.orders);
        c.execution.memberships = c.execution.memberships.max(admitted.memberships);
        c.execution.scalar_observations = c
            .execution
            .scalar_observations
            .max(admitted.scalar_observations);
        c.execution.modifiers.nodes = c.execution.modifiers.nodes.max(admitted.modifiers.nodes);
        c.execution.modifiers.entries = c
            .execution
            .modifiers
            .entries
            .max(admitted.modifiers.entries);
        grow_capacity(&mut c.execution.units, required.units);
        grow_capacity(&mut c.execution.groups, required.groups);
        grow_capacity(&mut c.execution.orders, required.orders);
        grow_capacity(&mut c.execution.memberships, required.memberships);
        grow_capacity(
            &mut c.execution.scalar_observations,
            required.scalar_observations,
        );
        grow_capacity(&mut c.execution.modifiers.nodes, required.modifiers.nodes);
        grow_capacity(
            &mut c.execution.modifiers.entries,
            required.modifiers.entries,
        );
        let _ = self.execution.try_grow(c.execution);
        grow_capacity(
            &mut c.country_references,
            self.execution_countries.raw_required(),
        );
        grow_capacity(
            &mut c.country_links,
            self.execution_countries.link_required(),
        );
        let _ = self
            .execution_countries
            .reserve(c.country_references, c.country_links);
    }

    /// Uses the already prepared shared division source bound; no extra native
    /// census or arbitrary modifier-entry multiplier is introduced.
    pub(crate) fn reserve_units(&mut self, units: usize) {
        let mut capacity = self.execution.capacity();
        capacity.units = capacity.units.max(units);
        let _ = self.execution.try_grow(capacity);
    }
    /// The borrowed parallel sources keep their separate completeness verdicts.
    /// A healthy plan prefix cannot certify a short assigned-host lane.
    fn assigned_outcome(&self) -> Outcome {
        match self.plans.outcome {
            Outcome::Rows { .. } => Outcome::Rows {
                len: self.assigned.len,
            },
            Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
            Outcome::Truncated { .. } => Outcome::Unavailable {
                reason: "plan assigned source incomplete",
            },
        }
    }

    pub(crate) fn execution_sources(&self) -> order_execution::SourceView<'_> {
        order_execution::SourceView {
            plans: &self.plans.rows[..self.plans.outcome.len()],
            source: self.plans.outcome,
            groups: &self.colors.rows[..self.colors.outcome.len()],
            group_source: self.colors.outcome,
            primary: &self.root_orders.first[..self.root_orders.len],
            assigned: &self.assigned.first[..self.assigned.len],
            assigned_source: self.assigned_outcome(),
            hosts: &self.assigned_hosts,
        }
    }

    /// The completed Orders payload and full current unit census are borrowed
    /// once by Late. Its country observations and metadata memo are not reset.
    pub(crate) fn capture_unit_transports(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        inputs: Option<&crate::division_census::UnitInputs>,
        orders: &crate::map_capture::Orders,
    ) -> u64 {
        if !self.supplied {
            self.execution
                .fail_units("plan execution source not supplied");
            return 0;
        }
        let _reads = crate::read_accounting::table(TableId::DivisionMovement);
        let started = qpc();
        let countries = self.execution_countries.view(memory, base, game_state);
        // Borrow the fields individually: Prepared mutates only its own bounded
        // arenas; none of the finished plans or parallel source spans change.
        let sources = order_execution::SourceView {
            plans: &self.plans.rows[..self.plans.outcome.len()],
            source: self.plans.outcome,
            groups: &self.colors.rows[..self.colors.outcome.len()],
            group_source: self.colors.outcome,
            primary: &self.root_orders.first[..self.root_orders.len],
            assigned: &self.assigned.first[..self.assigned.len],
            assigned_source: self.assigned_outcome(),
            hosts: &self.assigned_hosts,
        };
        self.execution.capture_units(
            memory,
            base,
            game_state,
            inputs.map_or(&[], crate::division_census::UnitInputs::seeds),
            inputs.map_or(super::MISSING, crate::division_census::UnitInputs::outcome),
            order_execution::Orders {
                roots: orders.roots(),
                root_source: orders.root_outcome,
                visits: orders.visits(),
                rows: &orders.rows.rows[..orders.rows.outcome.len()],
                source: orders.rows.outcome,
                geometry: &orders.geometry[..orders.geometry_len],
            },
            sources,
            &countries,
        );
        qpc().saturating_sub(started).max(0) as u64
    }

    pub(crate) fn fail_unit_transports(&mut self, reason: &'static str) {
        self.execution.fail_units(reason);
    }

    /// Only the completed sparse output moves; native pointer/memo arenas stay
    /// private to plans. The returned output buffer is grown off-hook if needed.
    pub(crate) fn install_order_transport(
        &mut self,
        presentation: &mut super::division_presentation::Captured,
    ) {
        presentation.install_order_transport(&mut self.execution);
    }
    /// Called on the Orders worker, which may grow payloads from owned census
    /// lengths and the original native headers. `visits` and `successors` are
    /// the census of this same job; nothing from another hour is passed.
    /// `game_state` is this pass's, for the invasions' countries and provinces.
    #[allow(clippy::too_many_arguments)]
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        roots: &[corders_group::Root],
        visits: &[corder_instance::Visit],
        successors: &corder_instance::SuccessorLists,
        root_source: Outcome,
        order_source: Outcome,
        owned_orders: &[corder_instance::Row],
        owned_paths: &[u32],
        group_source: Option<&super::naval_history::NavalJob>,
    ) -> u64 {
        self.supplied = true;
        self.execution.enable_worker_growth();
        self.assigned_hosts.enable_worker_growth();
        self.execution_countries.enable_worker_growth();
        self.execution_countries.reset(base, game_state);
        let begin_all = qpc();
        let before = self.ticks.iter().sum::<u64>();
        let mut read_phase = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let color_capacity = if let Outcome::Rows { len } = root_source
            && len == roots.len()
            && len > self.colors.rows.len()
        {
            let additional = len - self.colors.rows.len();
            engine::allocation::reserve(&mut self.colors.rows, additional)
                .map(|()| self.colors.rows.resize(len, Default::default()))
        } else {
            Ok(())
        };
        self.colors.outcome = match color_capacity {
            Ok(()) => {
                let report = corders_group::walk_plans(
                    memory,
                    base,
                    roots,
                    root_source,
                    visits,
                    order_source,
                    &mut self.colors.rows,
                    &mut self.root_orders.first,
                    &mut self.group_names.first,
                );
                self.root_orders.len = report.orders;
                self.root_orders.required = report.orders;
                self.group_names.len = report.names;
                self.group_names.required = report.names;
                report.outcome
            }
            Err(_) => Outcome::Unavailable {
                reason: "order color allocation failed",
            },
        };
        self.ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[0]);
        let begin = qpc();
        let report = corder_instance::walk_plans_ordered_with_sources(
            memory,
            base,
            roots,
            visits,
            successors,
            order_source,
            &mut self.plans.rows,
            &mut self.assigned.first,
            &mut self.links.first,
            &mut self.names.first,
            &mut self.order,
            Some(&mut self.assigned_hosts),
        );
        self.plans.outcome = report.outcome;
        self.assigned.len = report.assigned;
        self.assigned.required = report.required_assigned;
        self.links.len = report.links;
        self.links.required = report.links;
        self.names.len = report.names;
        self.names.required = report.names;
        let countries = self.execution_countries.view(memory, base, game_state);
        self.execution.capture_orders(
            memory,
            base,
            game_state,
            order_execution::Orders {
                roots,
                root_source,
                visits,
                rows: owned_orders,
                source: order_source,
                geometry: owned_paths,
            },
            &mut self.plans.rows[..report.outcome.len()],
            report.outcome,
            &countries,
        );
        if let Outcome::Rows { len } = report.outcome {
            // Owned OOB index preparation is charged to this optional plan
            // phase. Failed/empty core plans need no index; successful queries
            // preserve exact root-batch faults and return the index for Naval.
            let group_source = group_source
                .filter(|_| len != 0)
                .map(super::naval_history::OrderGroupSource::new);
            let children = corder_instance::child_fronts::walk(
                memory,
                base,
                game_state,
                roots,
                visits,
                owned_orders,
                owned_paths,
                order_source,
                &mut self.plans.rows[..len],
                &mut self.child_sections.first,
                &mut self.child_scratch,
                group_source.as_ref(),
            );
            self.child_sections.len = children.sections;
            self.child_sections.required = children.required_sections;
            self.child_front_failures = [
                children.area_failures,
                children.header_failures,
                children.section_failures,
            ];
        }
        self.ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[2]);
        let begin = qpc();
        let front = cfront::walk(
            memory,
            base,
            &self.plans.rows,
            report.outcome,
            &mut self.handles,
            &mut self.fronts.rows,
            &mut self.sections.first,
            &mut self.pairs.first,
        );
        self.required_handles = self.required_handles.max(front.required_handles);
        self.fronts.outcome = front.outcome;
        self.sections.len = front.sections;
        self.sections.required = front.required_sections;
        self.pairs.len = front.pairs;
        self.pairs.required = front.required_pairs;
        self.ticks[2] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[3]);
        let begin = qpc();
        let invasions = invasion_preparations::walk_growing(
            memory,
            base,
            game_state,
            visits,
            order_source,
            &self.memo,
            &countries,
            &mut self.invasions.rows,
            &mut self.co_belligerents.first,
        );
        self.invasions.outcome = invasions.outcome;
        self.co_belligerents.len = invasions.slots;
        self.co_belligerents.required = invasions.required_slots;
        self.ticks[3] += qpc().saturating_sub(begin).max(0) as u64;
        let total = qpc().saturating_sub(begin_all).max(0) as u64;
        self.ticks[0] += total.saturating_sub(self.ticks.iter().sum::<u64>() - before);
        // The order records this job's census gathered for these plans.
        let (_, census_ticks) = successors.plan_gathers();
        self.ticks[0] += census_ticks;
        total + census_ticks
    }
    /// Terminal worker/admission failure; no partial table may be published.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.plans.outcome = outcome;
        self.colors.outcome = outcome;
        self.fronts.outcome = outcome;
        self.invasions.fail(outcome);
        self.co_belligerents.len = 0;
        self.assigned.len = 0;
        self.child_sections.len = 0;
        self.child_front_failures = [0; 3];
        self.links.len = 0;
        self.names.len = 0;
        self.group_names.len = 0;
        self.root_orders.len = 0;
        self.sections.len = 0;
        self.pairs.len = 0;
        self.ticks = [0; N];
        self.supplied = true;
        self.assigned_hosts.fail(reason);
        self.execution.fail_units(reason);
    }
    pub fn capture(&mut self) -> [u64; N] {
        if !self.supplied {
            let outcome = Outcome::Unavailable {
                reason: "plan source not supplied",
            };
            self.plans.outcome = outcome;
            self.colors.outcome = outcome;
            self.fronts.outcome = outcome;
            self.invasions.outcome = outcome;
        }
        self.ticks
    }
    fn name(&self, (start, len): (usize, usize)) -> Vec<u8> {
        owned_vec(copy_vec(&self.names.first[start..start + len]))
    }
    fn child_fronts(
        &self,
        inputs: Option<corder_instance::child_fronts::Inputs>,
    ) -> Option<OrderChildFrontInputs> {
        inputs.map(|inputs| OrderChildFrontInputs {
            area_province_count: inputs.area_province_count,
            count: inputs.count,
            group_child_count: inputs.group_child_count,
            sections: inputs.sections.map(|(start, len)| {
                let source = &self.child_sections.first[start..start + len];
                let mut sections = owned_vec(engine::allocation::vec(source.len()));
                sections.extend(source.iter().map(|section| OrderChildFront {
                    group: section.group,
                    segment_start: section.segment_start,
                    segment_end: section.segment_end,
                }));
                sections
            }),
        })
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.plans.outcome,
            self.colors.outcome,
            self.fronts.outcome,
            self.invasions.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::ArmyOrderPlans, _>(outcomes[0], &self.plans.rows, |r| {
                ArmyOrderPlanRow {
                    owner_group: r.owner_group,
                    instance: r.instance,
                    front: r.front,
                    front_valid: r.front_valid,
                    section: r.section,
                    segment_start: r.segment_start,
                    segment_end: r.segment_end,
                    blitz: r.blitz,
                    withdraw: r.withdraw,
                    fallback: r.fallback,
                    virtual_order: r.virtual_order,
                    has_child: r.has_child,
                    owner_override: r.owner_override,
                    predecessors: owned_vec(copy_vec(
                        &self.links.first
                            [r.predecessors_start..r.predecessors_start + r.predecessors_len],
                    )),
                    successors: owned_vec(copy_vec(
                        &self.links.first
                            [r.successors_start..r.successors_start + r.successors_len],
                    )),
                    assigned_divisions: owned_vec(copy_vec(
                        &self.assigned.first[r.assigned_start..r.assigned_start + r.assigned_len],
                    )),
                    name_prefix: self.name(r.names[0]),
                    name: self.name(r.names[1]),
                    name_first: self.name(r.names[2]),
                    name_second: self.name(r.names[3]),
                    name_unique: self.name(r.names[4]),
                    name_postfix: self.name(r.names[5]),
                    invasion_transports: r.invasion_transports.into(),
                    child_fronts: Recorded::Value(self.child_fronts(r.child_fronts)),
                    execution: Recorded::Value(r.execution.map(project_execution)),
                }
            }),
            table::<tables::OrdersGroupPlans, _>(outcomes[1], &self.colors.rows, |r| {
                OrdersGroupPlanRow {
                    group: r.group,
                    red: r.red,
                    green: r.green,
                    blue: r.blue,
                    alpha: r.alpha,
                    primary_orders: owned_vec(copy_vec(
                        &self.root_orders.first[r.primary_start..r.primary_start + r.primary_len],
                    )),
                    fallback_orders: owned_vec(copy_vec(
                        &self.root_orders.first
                            [r.fallback_start..r.fallback_start + r.fallback_len],
                    )),
                    name: owned_vec(copy_vec(
                        &self.group_names.first[r.name_start..r.name_start + r.name_len],
                    )),
                }
            }),
            table::<tables::Fronts, _>(outcomes[2], &self.fronts.rows, |r| {
                let mut row = FrontRow {
                    front: r.front,
                    ..Default::default()
                };
                let sections =
                    &self.sections.first[r.section_start..r.section_start + r.section_len];
                let pairs = sections
                    .iter()
                    .try_fold(0_usize, |count, section| {
                        count.checked_add(section.pair_len)
                    })
                    .unwrap_or(usize::MAX);
                if engine::allocation::reserve_exact(&mut row.section_ids, sections.len()).is_err()
                    || engine::allocation::reserve_exact(&mut row.section_ends, sections.len())
                        .is_err()
                    || engine::allocation::reserve_exact(&mut row.province_pairs, pairs).is_err()
                {
                    return row;
                }
                // One reserve covers all sections and their captured pairs.
                for section in sections {
                    row.section_ids.push(section.id);
                    row.province_pairs.extend_from_slice(
                        &self.pairs.first
                            [section.pair_start..section.pair_start + section.pair_len],
                    );
                    row.section_ends.push(row.province_pairs.len() as u32);
                }
                row
            }),
            table::<tables::InvasionPreparations, _>(outcomes[3], &self.invasions.rows, |r| {
                InvasionPreparationRow {
                    owner_group: r.owner_group,
                    instance: r.instance,
                    progress: r.progress,
                    target: r.target,
                    country_term: r.country_term,
                    relation_term: r.relation_term,
                    relation_bonus: r.relation_bonus,
                    leader_term: r.leader_term,
                    leader_applies: r.leader_applies,
                    days_term: r.days_term,
                    unit_owner: r.unit_owner,
                    zero_war_present: r.zero_relation.war_present,
                    zero_war_retired: r.zero_relation.war_retired,
                    zero_co_belligerent: r.zero_relation.co_belligerent,
                    co_belligerents: owned_vec(copy_vec(
                        &self.co_belligerents.first
                            [r.co_belligerent_start..r.co_belligerent_start + r.co_belligerent_len],
                    )),
                }
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{creference_registry as registry, reference_bucket as bucket};
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Image(BTreeMap<usize, u8>);
    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, v)| (at + i, *v)));
        }
        fn u64(&mut self, at: usize, v: u64) {
            self.put(at, &v.to_le_bytes());
        }
        fn u32(&mut self, at: usize, v: u32) {
            self.put(at, &v.to_le_bytes());
        }
    }
    impl Memory for Image {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            for (i, v) in out.iter_mut().enumerate() {
                let Some(b) = self.0.get(&(at + i)) else {
                    return false;
                };
                *v = *b;
            }
            true
        }
    }
    const GROUP: usize = 0x3000;
    const ORDER: usize = 0x4000;
    fn image() -> Image {
        let mut m = Image::default();
        m.u64(registry::INVALID_REFERENCE_RVA, 0);
        m.u64(registry::LOW_ROOTS_RVA + 53 * 8, 0x1000);
        m.put(0x1000, &[0; 32]);
        m.u64(0x1008, 0x2000);
        m.u32(0x1010, 1);
        m.u32(0x1014, 7);
        m.put(0x1018, &[5]);
        m.put(0x2000, &[0; 14 * bucket::STRIDE]);
        m.put(0x2000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        m.put(0x2000 + bucket::DISTANCE, &[1]);
        m.u64(0x2000 + bucket::TYPE, (7 << 32) | 53);
        m.u64(0x2000 + bucket::OBJECT, GROUP as u64);
        m.put(GROUP, &[0; 0x240]);
        m.u64(GROUP, corders_group::VTABLE_RVA as u64);
        m.u64(GROUP + 8, (7 << 32) | 53);
        m.u32(GROUP + corders_group::COUNTRY, 1);
        // An empty inline name: capacity 15, length 0.
        m.u64(GROUP + corders_group::NAME + 0x18, 15);
        let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
        m.u64(root, 0x3800);
        m.u32(root + 8, 1);
        m.u32(root + 12, 1);
        m.u64(0x3800, ORDER as u64);
        m.put(ORDER, &[0; 0x400]);
        m.u64(ORDER, corder_instance::VTABLE_RVA as u64);
        m.u64(ORDER + corder_instance::OWNER, GROUP as u64);
        m.u32(ORDER + corder_instance::ID, 9);
        m
    }
    fn pass(m: &impl Memory, c: &mut Captured) {
        let mut groups = [corders_group::Root::default(); 4];
        let mut visits = vec![corder_instance::Visit::default(); 8];
        let mut successors = corder_instance::SuccessorLists::default();
        let mut orders = vec![corder_instance::Row::default(); 4];
        let mut paths = vec![0; 4];
        let r = corder_instance::walk_listing_successors(
            m,
            0,
            &mut orders,
            &mut paths,
            &mut groups,
            &mut visits,
            &mut successors,
        );
        assert!(matches!(r.outcome, Outcome::Rows { len } if len != 0));
        c.visit(
            m,
            0,
            0,
            &groups[..r.root_outcome.len()],
            &visits,
            &successors,
            r.root_outcome,
            r.outcome,
            &orders[..r.outcome.len()],
            &paths[..r.geometry_len],
            None,
        );
    }
    fn execution_image() -> Image {
        let mut image = image();
        image.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_PARADROP as u32,
        );
        for offset in corder_instance::NAME_STRINGS {
            image.u64(ORDER + offset + 0x18, 15);
        }
        image.put(ORDER + corder_instance::CONDITION_CACHE, &[7]);
        image.u64(
            ORDER + corder_instance::ACCUMULATED_PREPARATION,
            i64::MIN as u64,
        );
        image
    }

    fn execution_capture() -> Captured {
        let mut capacities = Capacities::default();
        capacities.raise_for_countries(5);
        capacities.execution.units = 1;
        capacities.execution.scalar_observations = 32;
        Captured::with_capacities(&capacities)
    }

    #[test]
    fn attach_execution_hint_follows_native_graph_with_five_thousand_countries() {
        let mut world =
            crate::unit_scale_tests::fixture::build(crate::unit_scale_tests::fixture::Shape {
                countries: 5_000,
                divisions: 0,
                wings: 1,
                planes: 1,
                ships: 0,
            });
        world.bytes[engine::cgame_state::SINGLETON_RVA..engine::cgame_state::SINGLETON_RVA + 8]
            .copy_from_slice(&(world.state as u64).to_le_bytes());
        let mut image = execution_image();
        image.u64(GROUP, (world.base + corders_group::VTABLE_RVA) as u64);
        image.u64(ORDER, (world.base + corder_instance::VTABLE_RVA) as u64);
        image.u64(world.base + registry::LOW_ROOTS_RVA + 53 * 8, 0x1000);
        image.u64(world.base + registry::INVALID_REFERENCE_RVA, 0);
        struct Combined<'a> {
            image: &'a Image,
            world: &'a crate::unit_scale_tests::fixture::World,
        }
        impl Memory for Combined<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if self.image.0.contains_key(&address) {
                    self.image.copy(address, output)
                } else {
                    self.world.live().copy(address, output)
                }
            }
        }
        let memory = Combined {
            image: &image,
            world: &world,
        };
        let mut capacity = Capacities::default();
        assert_eq!(capacity.execution.scalar_observations, 0);
        capacity
            .presize_execution(&memory, world.base, world.state)
            .unwrap();
        assert!(capacity.execution.scalar_observations >= 1);
        assert!(capacity.execution.modifiers.nodes >= 1);
        let before = capacity;
        let broken = Image::default();
        assert!(
            capacity
                .presize_execution(&broken, world.base, world.state)
                .is_err()
        );
        assert!(capacity == before);
    }

    #[test]
    fn live_country_bound_presizes_aliases_and_retains_reserved_success_and_failure_across_views() {
        use crate::sampling::wars_test_kit as kit;
        use std::{cell::Cell, sync::Arc};
        struct Counted<'a> {
            bytes: &'a [u8],
            calls: Cell<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                kit::memory(self.bytes).copy(at, output)
            }
        }
        for reserved_present in [false, true] {
            let mut bytes = kit::fixture();
            const RESERVED: usize = 0x24000;
            if reserved_present {
                bytes[kit::CTAB..kit::CTAB + 8].copy_from_slice(&(RESERVED as u64).to_le_bytes());
                bytes[RESERVED..RESERVED + 8]
                    .copy_from_slice(&(ccountry::VTABLE_RVA as u64).to_le_bytes());
                bytes[RESERVED + ccountry::ID..RESERVED + ccountry::ID + 4]
                    .copy_from_slice(&0_u32.to_le_bytes());
            }
            let memory = Counted {
                bytes: &bytes,
                calls: Cell::new(0),
            };
            let mut storage =
                Arc::new(engine::country_memory::Cache::try_with_capacity(64).unwrap());
            let countries = engine::country_memory::Cache::prepare_shared(
                &mut storage,
                &memory,
                kit::BASE,
                kit::GAME_STATE,
            )
            .unwrap();
            let mut capacities = Capacities::default();
            capacities.raise_for_countries(5);
            let mut capture = Captured::with_capacities(&capacities);
            assert_eq!(capture.execution_countries.raw_capacity(), 5);
            assert_eq!(capture.execution_countries.link_capacity(), 5);
            capture
                .execution_countries
                .reset(kit::BASE, kit::GAME_STATE);
            let cached = countries.roster_view(&memory);
            let context = capture
                .execution_countries
                .view(&cached, kit::BASE, kit::GAME_STATE);
            assert_eq!(context.resolve(4).unwrap().pointer, kit::C0);
            assert_eq!(context.resolve(1).unwrap().pointer, kit::C0);
            let root = context.resolve(0);
            assert_eq!(root.is_ok(), reserved_present);
            if let Ok(root) = root {
                assert_eq!((root.pointer, root.id), (RESERVED, 0));
            }
            let calls = memory.calls.get();
            // Orders and Late reconstruct their borrowed views over this same
            // owned memo. Neither success nor a reserved failure retries bytes.
            let next = capture
                .execution_countries
                .view(&cached, kit::BASE, kit::GAME_STATE);
            assert_eq!(next.resolve(4).unwrap().pointer, kit::C0);
            assert_eq!(next.resolve(0).is_ok(), reserved_present);
            assert_eq!(memory.calls.get(), calls);
            assert!(!capture.execution_countries.raw_required_is_lower_bound());
        }
    }

    #[test]
    fn execution_scalar_faults_keep_the_core_plan_and_earlier_progress() {
        struct Fault<'a> {
            image: &'a Image,
            at: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at <= self.at && self.at < at.saturating_add(output.len()) {
                    return false;
                }
                self.image.copy(at, output)
            }
        }
        let image = execution_image();
        for fault in [
            None,
            Some(corder_instance::CONDITION_CACHE),
            Some(corder_instance::ACCUMULATED_PREPARATION),
        ] {
            let mut capture = execution_capture();
            match fault {
                Some(offset) => pass(
                    &Fault {
                        image: &image,
                        at: ORDER + offset,
                    },
                    &mut capture,
                ),
                None => pass(&image, &mut capture),
            }
            let TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(rows)) =
                capture.tables()[0].clone()
            else {
                panic!("optional scalar faults must preserve the plan prefix");
            };
            assert_eq!(
                (rows[0].owner_group, rows[0].instance),
                ((7_u64 << 32) | 53, 9)
            );
            let Recorded::Value(Some(inputs)) = rows[0].execution else {
                panic!("known type4 keeps its observation");
            };
            assert_eq!(
                inputs.route_is_ok,
                (fault != Some(corder_instance::CONDITION_CACHE)).then_some(7)
            );
            if fault.is_none() {
                let prep = inputs
                    .paradrop
                    .expect("progress survives an unavailable country term");
                assert_eq!(prep.progress, i64::MIN);
                assert_eq!(prep.terms, None);
            } else {
                assert_eq!(inputs.paradrop, None);
            }
            assert_eq!(capture.execution.work().route_demand, 1);
        }
    }

    #[test]
    fn first_pass_saved_execution_scalars_need_no_terms_memo_and_keep_native_faults() {
        struct Fault<'a> {
            image: &'a Image,
            at: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at <= self.at && self.at < at.saturating_add(output.len()) {
                    return false;
                }
                self.image.copy(at, output)
            }
        }
        let row = |capture: &Captured| {
            let [
                TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(mut rows)),
                _,
                _,
                _,
            ] = capture.tables()
            else {
                panic!("saved scalar acquisition must keep the core plan");
            };
            assert_eq!(rows.len(), 1);
            rows.pop().unwrap()
        };
        let image = execution_image();
        let mut capacities = Capacities::default();
        capacities.raise_for_countries(5);
        assert_eq!(capacities.execution.scalar_observations, 0);
        let mut capture = Captured::with_capacities(&capacities);
        pass(&image, &mut capture);
        let ready = row(&capture);
        let mut cold = Captured::vacant();
        pass(&image, &mut cold);
        assert_eq!(row(&cold), ready);
        let Recorded::Value(Some(inputs)) = &ready.execution else {
            panic!("first pass needs the readable execution prefix");
        };
        let prep = inputs.paradrop.as_ref().unwrap();
        assert_eq!((inputs.route_is_ok, prep.progress), (Some(7), i64::MIN));
        assert_eq!(prep.terms, None); // Independent unavailable country source.
        for (offset, expected) in [
            (
                corder_instance::CONDITION_CACHE,
                replay::rows::OrderExecutionInputs::default(),
            ),
            (
                corder_instance::ACCUMULATED_PREPARATION,
                replay::rows::OrderExecutionInputs {
                    route_is_ok: Some(7),
                    paradrop: None,
                },
            ),
        ] {
            capture.reset();
            pass(
                &Fault {
                    image: &image,
                    at: ORDER + offset,
                },
                &mut capture,
            );
            let mut original = ready.clone();
            original.execution = Recorded::Value(Some(expected));
            assert_eq!(row(&capture), original);
        }
        capture.reset();
        pass(&image, &mut capture);
        assert_eq!(row(&capture), ready);
    }

    #[test]
    fn completed_plans_survive_late_success_panic_stop_and_admission_rejection() {
        use crate::acquisition::{Failure, SubmitError};
        use crate::acquisition_jobs::{Job, LateReport, Report};
        use std::sync::Arc;
        for mode in 0..4 {
            let image = execution_image();
            let mut captured =
                crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
            captured.plans = execution_capture();
            pass(&image, &mut captured.plans);
            let expected = captured.plans.tables();
            let mut orders =
                crate::map_capture::Orders::for_test(&crate::map_capture::MapCapacities::default());
            orders.walk_roots(&image, 0);
            orders.walk(&image, 0);
            let orders_outcome = orders.rows.outcome;
            let order_storage = orders.geometry.as_ptr();
            let countries = Arc::new(engine::country_memory::Cache::try_with_capacity(0).unwrap());
            let bytes = captured.allocated_bytes() + orders.allocated_bytes();
            let pool = crate::acquisition::Pool::start(0, 1, move |job: &mut Job| {
                let Job::Late(request) = job else {
                    panic!("only Late is submitted");
                };
                assert!(mode != 1, "synthetic Late unwind");
                request
                    .plans
                    .fail_unit_transports("synthetic optional transport refusal");
                Report::Late(LateReport)
            })
            .unwrap();
            if mode == 3 {
                pool.shutdown().unwrap();
            }
            let request = captured.take_late_request(
                &countries,
                &mut orders,
                true,
                0,
                0,
                1,
                false,
                Box::default(),
            );
            assert!(Arc::ptr_eq(&request.countries, &countries));
            assert_eq!(
                captured.plans.allocated_bytes(),
                0,
                "the parked plan owner has no duplicate payload"
            );
            assert_eq!(
                captured.allocated_bytes()
                    + orders.allocated_bytes()
                    + request.packs.allocated_bytes()
                    + request.plans.allocated_bytes(),
                bytes
            );
            let mut pending = Some(Job::Late(request));
            // SAFETY: the fixture runner reads only owned Rust payloads.
            let submitted = unsafe { pool.submit(&mut pending) };
            let (request, completed) = if mode == 3 {
                assert_eq!(submitted, Err(SubmitError::Stopped));
                let Job::Late(request) = pending.take().unwrap() else {
                    unreachable!();
                };
                (request, false)
            } else {
                let ticket = submitted.unwrap();
                if mode == 2 {
                    pool.shutdown().unwrap();
                }
                let completion = pool.finish(ticket).unwrap();
                match mode {
                    0 => assert!(completion.report.is_ok()),
                    1 => assert!(matches!(completion.report, Err(Failure::Panicked))),
                    2 => assert!(matches!(completion.report, Err(Failure::Stopped))),
                    _ => unreachable!(),
                }
                let completed = completion.report.is_ok();
                let Job::Late(request) = completion.request else {
                    unreachable!();
                };
                (request, completed)
            };
            captured.restore_late_request(request, completed, &mut orders);
            assert_eq!(
                captured.plans.tables(),
                expected,
                "Late cannot erase a completed prefix/route/progress"
            );
            assert_eq!(orders.rows.outcome, orders_outcome);
            assert_eq!(orders.geometry.as_ptr(), order_storage);
            assert_eq!(captured.allocated_bytes() + orders.allocated_bytes(), bytes);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert!(matches!(
                captured.division_presentation.order_transport_storage().1,
                Outcome::Unavailable { .. }
            ));
            let output_bytes = captured.division_presentation.order_transport_storage().0;
            assert!(
                output_bytes > 0,
                "the fixture transferred a real sparse output buffer"
            );
            let mut capacities = Capacities::default();
            captured.plans.grow(&mut capacities);
            assert_eq!(
                captured.allocated_bytes() + orders.allocated_bytes(),
                bytes + output_bytes,
                "off-hook growth adds exactly the returned output spare, counting each owner once"
            );
            pool.shutdown().unwrap();
        }
    }
    #[test]
    fn primary_plan_styles_and_colors_spool_exact_bytes() {
        let mut m = image();
        let mut c = Captured::with_capacities(&Capacities::default());
        m.put(ORDER + corder_instance::MOVE_SPECIAL, &[255]);
        m.u32(GROUP + corders_group::COLOUR, 0x7fc01234);
        pass(&m, &mut c);
        c.capture();
        assert!(
            matches!(&c.tables()[0], TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(rows)) if rows[0].blitz == 255)
        );
        assert!(
            matches!(&c.tables()[1], TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(rows)) if rows[0].red == 0x7fc01234)
        );
        assert_eq!(c.outcomes()[2], Outcome::Rows { len: 0 });
    }
    #[test]
    fn empty_payload_storage_grows_and_terminal_failure_hides_all_tables() {
        let m = image();
        let mut c = Captured::vacant();
        pass(&m, &mut c);
        assert_eq!(
            c.outcomes(),
            [
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 0 }
            ]
        );
        let bytes = c.allocated_bytes();
        assert!(bytes > 0);
        c.fail("worker failed");
        c.capture();
        assert_eq!(
            c.outcomes(),
            [Outcome::Unavailable {
                reason: "worker failed"
            }; N]
        );
        assert_eq!(c.allocated_bytes(), bytes);
        c.reset();
        pass(&m, &mut c);
        assert_eq!(c.outcomes()[0], Outcome::Rows { len: 1 });
    }
    #[test]
    fn growth_preserves_primary_arenas_and_missing_source_fails() {
        let m = image();
        let mut capacity = Capacities {
            assigned: 16384,
            pairs: 65536,
            handles: 2048,
            ..Capacities::default()
        };
        let mut c = Captured::with_capacities(&capacity);
        pass(&m, &mut c);
        c.capture();
        c.assigned.required = capacity.assigned + 1;
        c.pairs.required = capacity.pairs + 1;
        c.required_handles = capacity.handles + 1;
        c.grow(&mut capacity);
        assert!(capacity.assigned > 16384 && capacity.pairs > 65536 && capacity.handles > 2048);
        assert_eq!(c.assigned.first.len(), capacity.assigned);
        assert_eq!(c.pairs.first.len(), capacity.pairs);
        assert_eq!(c.handles.len(), capacity.handles);
        c.reset();
        c.capture();
        assert!(
            c.outcomes()
                .iter()
                .all(|v| matches!(v, Outcome::Unavailable { .. }))
        );
    }

    /// A naval invasion whose inputs cannot be read withholds only
    /// InvasionPreparations; the plans of the same census are published.
    #[test]
    fn an_unreadable_invasion_withholds_only_its_own_table() {
        let mut m = image();
        m.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_INVASION as u32,
        );
        // An invasion is a named arrow: its name strings are read too.
        for offset in corder_instance::NAME_STRINGS {
            string(&mut m, ORDER + offset, b"", None);
        }
        let mut c = Captured::with_capacities(&Capacities::default());
        pass(&m, &mut c);
        c.capture();
        let outcomes = c.outcomes();
        assert_eq!(
            &outcomes[..3],
            &[
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 0 }
            ]
        );
        assert!(matches!(outcomes[3], Outcome::Unavailable { .. }));
        assert!(matches!(c.tables()[3], TableView::Unavailable { .. }));
    }

    /// A std::string header at `at`: inline when `heap` is `None`.
    fn string(m: &mut Image, at: usize, text: &[u8], heap: Option<usize>) {
        match heap {
            Some(data) => {
                m.put(data, text);
                m.put(data + text.len(), &[0]);
                m.u64(at, data as u64);
                m.u64(at + 0x18, (text.len() as u64).max(31));
            }
            None => {
                m.put(at, text);
                m.put(at + text.len(), &[0]);
                m.u64(at + 0x18, 15);
            }
        }
        m.u64(at + 0x10, text.len() as u64);
    }

    /// Every published list and string is exactly its own span of the shared
    /// arenas: consecutive, non-overlapping, ending at the arena's used length.
    /// A second hour reuses the grown arenas with shorter text, and no stale
    /// byte past a span leaks into a row.
    #[test]
    fn arena_spans_are_exact_across_rows_growth_and_reuse() {
        const SECOND: usize = 0x4800;
        let build = |group_name: &[u8], unique: &[u8]| {
            let mut m = image();
            // Two roots; the first succeeds into the second.
            let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
            m.u64(root, 0x3800);
            m.u32(root + 8, 2);
            m.u32(root + 12, 2);
            m.u64(0x3808, SECOND as u64);
            m.put(SECOND, &[0; 0x400]);
            m.u64(SECOND, corder_instance::VTABLE_RVA as u64);
            m.u64(SECOND + corder_instance::OWNER, GROUP as u64);
            m.u32(SECOND + corder_instance::ID, 10);
            m.u64(ORDER + corder_instance::SUCCESSOR_DATA, 0x3900);
            m.u32(ORDER + corder_instance::SUCCESSOR_DATA + 8, 1);
            m.u32(ORDER + corder_instance::SUCCESSOR_DATA + 12, 1);
            m.u64(0x3900, SECOND as u64);
            m.u64(SECOND + corder_instance::PREDECESSOR_DATA, 0x3a00);
            m.u32(SECOND + corder_instance::PREDECESSOR_DATA + 8, 1);
            m.u32(SECOND + corder_instance::PREDECESSOR_DATA + 12, 1);
            m.u64(0x3a00, ORDER as u64);
            // Both are named moves: inline keys and a heap unique text.
            for (order, heap) in [(ORDER, 0x5000), (SECOND, 0x5100)] {
                m.u32(
                    order + corder_instance::TYPE,
                    corder_instance::TYPE_MOVE as u32,
                );
                for (index, offset) in corder_instance::NAME_STRINGS.into_iter().enumerate() {
                    let at = order + offset;
                    match index {
                        0 => string(&mut m, at, b"prefix_key", None),
                        4 => string(&mut m, at, unique, Some(heap)),
                        _ => string(&mut m, at, b"", None),
                    }
                }
            }
            string(
                &mut m,
                GROUP + corders_group::NAME,
                group_name,
                Some(0x5200),
            );
            m
        };
        let rows = |c: &Captured| {
            let [
                TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(groups)),
                _,
                _,
            ] = c.tables()
            else {
                panic!("plans unavailable: {:?}", c.outcomes());
            };
            (plans, groups)
        };
        let mut c = Captured::vacant();
        let long = b"Heeresgruppe Mitte, Second Panzer";
        pass(&build(long, b"Operation Barbarossa"), &mut c);
        c.capture();
        let (plans, groups) = rows(&c);
        assert_eq!(plans.len(), 2);
        let (first, second) = (&plans[0], &plans[1]);
        assert_eq!((first.instance, second.instance), (9, 10));
        assert_eq!(
            (&first.successors[..], &first.predecessors[..]),
            (&[10][..], &[][..])
        );
        assert_eq!(
            (&second.successors[..], &second.predecessors[..]),
            (&[][..], &[9][..])
        );
        for plan in &plans {
            assert_eq!(plan.name_prefix, b"prefix_key");
            assert_eq!(plan.name_unique, b"Operation Barbarossa");
            assert!(plan.name.is_empty() && plan.name_postfix.is_empty());
        }
        assert_eq!(groups[0].name, long);
        assert_eq!(groups[0].primary_orders, [9, 10]);
        // The nonempty spans tile the used prefix of each arena: rows are
        // sorted by key after the walk, so order the spans by start first.
        let mut spans: Vec<_> = c.plans.rows[..2]
            .iter()
            .flat_map(|plan| plan.names)
            .filter(|&(_, len)| len != 0)
            .collect();
        spans.sort_unstable();
        let end = spans.iter().fold(0, |end, &(start, len)| {
            assert_eq!(start, end);
            start + len
        });
        assert_eq!(end, c.names.len);
        assert_eq!(c.links.len, 2);
        assert_eq!(c.group_names.len, long.len());

        // The next hour reuses the grown arenas: shorter text must not carry
        // the previous hour's trailing bytes.
        let mut capacities = Capacities {
            plans: 0,
            groups: 0,
            fronts: 0,
            assigned: 0,
            child_sections: 0,
            links: 0,
            names: 0,
            root_orders: 0,
            sections: 0,
            pairs: 0,
            handles: 0,
            invasions: 0,
            co_belligerents: 0,
            execution: order_execution::Capacity::default(),
            country_references: 0,
            country_links: 0,
        };
        c.grow(&mut capacities);
        c.reset();
        pass(&build(b"Army 1", b"Op"), &mut c);
        c.capture();
        let (plans, groups) = rows(&c);
        assert!(plans.iter().all(|plan| plan.name_unique == b"Op"));
        assert_eq!(groups[0].name, b"Army 1");
        assert_eq!(c.names.len, 2 * (b"prefix_key".len() + 2));
    }

    /// No writer of the order or group names marks a change: a heap buffer
    /// is rewritten in place when the new text fits, so every hour reads the
    /// stored bytes again. A capture reused across hours records each state
    /// byte for byte as a fresh capture of the same memory does: an
    /// unchanged hour, a same-length rename in the same buffer, a length
    /// change in that buffer, and inline to heap and back.
    #[test]
    fn reused_captures_record_every_rename_as_a_fresh_capture_does() {
        const UNIQUE: usize = 0x5000;
        const GROUP_NAME: usize = 0x5100;
        let unique_at = ORDER + corder_instance::NAME_STRINGS[4];
        let group_at = GROUP + corders_group::NAME;
        // One image carried across the hours, so a rename leaves the old
        // tail bytes behind its terminator exactly as an in-place assign does.
        let mut m = image();
        m.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_MOVE as u32,
        );
        for (index, offset) in corder_instance::NAME_STRINGS.into_iter().enumerate() {
            let text: &[u8] = if index == 0 { b"o_operation" } else { b"" };
            string(&mut m, ORDER + offset, text, None);
        }
        m.put(UNIQUE, &[0xa5; 64]);
        m.put(GROUP_NAME, &[0xa5; 64]);
        let names = |c: &Captured| {
            let [
                TableView::Complete(replay::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay::rows::TableRows::OrdersGroupPlans(groups)),
                _,
                _,
            ] = c.tables()
            else {
                panic!("plans unavailable: {:?}", c.outcomes());
            };
            (plans, groups)
        };
        let hours: [(&[u8], bool, &[u8], bool); 6] = [
            (b"Operation Barbarossa", true, b"Heeresgruppe Mitte", true),
            // Unchanged.
            (b"Operation Barbarossa", true, b"Heeresgruppe Mitte", true),
            // Same length, same buffer, same capacity: only the bytes differ.
            (b"Operation Zitadelle!", true, b"Heeresgruppe Sued!", true),
            // Shorter and longer in the same buffer.
            (
                b"Operation Blau",
                true,
                b"Heeresgruppe Nord, 16. Armee",
                true,
            ),
            // Heap to inline.
            (b"Op Uranus", false, b"Army 1", false),
            // Inline to heap.
            (b"Operation Bagration", true, b"1st Belorussian Front", true),
        ];
        let mut reused = Captured::vacant();
        let mut capacities = Capacities {
            plans: 0,
            groups: 0,
            fronts: 0,
            assigned: 0,
            child_sections: 0,
            links: 0,
            names: 0,
            root_orders: 0,
            sections: 0,
            pairs: 0,
            handles: 0,
            invasions: 0,
            co_belligerents: 0,
            execution: order_execution::Capacity::default(),
            country_references: 0,
            country_links: 0,
        };
        for (hour, (unique, unique_heap, group, group_heap)) in hours.into_iter().enumerate() {
            string(&mut m, unique_at, unique, unique_heap.then_some(UNIQUE));
            string(&mut m, group_at, group, group_heap.then_some(GROUP_NAME));
            // The heap buffers keep one capacity, as an in-place assign does.
            if unique_heap {
                m.u64(unique_at + 0x18, 63);
            }
            if group_heap {
                m.u64(group_at + 0x18, 63);
            }
            reused.reset();
            pass(&m, &mut reused);
            reused.capture();
            let mut fresh = Captured::with_capacities(&Capacities::default());
            pass(&m, &mut fresh);
            fresh.capture();
            let (plans, groups) = names(&reused);
            assert_eq!(
                (&plans, &groups),
                (&names(&fresh).0, &names(&fresh).1),
                "hour {hour}"
            );
            assert_eq!(plans[0].name_unique, unique, "hour {hour}");
            assert_eq!(plans[0].name_prefix, b"o_operation");
            assert_eq!(groups[0].name, group, "hour {hour}");
            reused.grow(&mut capacities);
        }
    }
}

/// Ignored capture-cost bench; see `bench_world` for the assumed scale. Each
/// case times the Orders job's pass through the plain slice and through the
/// production guarded source: the ArmyOrders census, which gathers the plans'
/// order records, then the plan visit. Each table's time is its own ticks,
/// the census's plan gathers included in ArmyOrderPlans; `order-census` is
/// the census without them. It also counts one pass's source requests per
/// table, charged as the Orders job charges them.
#[cfg(test)]
mod l3b_bench {
    use super::*;
    use crate::extended::bench_world::{self, Shape, World};

    /// The census and plan visit of one Orders job pass, with its reusable
    /// scratch. Returns the census's own ticks, without its plan gathers,
    /// and the gathers' ticks.
    struct Pass {
        captured: Captured,
        rows: Vec<corder_instance::Row>,
        geometry: Vec<u32>,
        visits: Vec<corder_instance::Visit>,
        successors: corder_instance::SuccessorLists,
    }

    impl Pass {
        fn new() -> Self {
            Self {
                captured: Captured::with_capacities(&Capacities::default()),
                rows: Vec::new(),
                geometry: Vec::new(),
                visits: Vec::new(),
                successors: corder_instance::SuccessorLists::default(),
            }
        }

        fn run(
            &mut self,
            memory: &impl Memory,
            base: usize,
            roots: &[corders_group::Root],
            root_outcome: Outcome,
        ) -> (u64, u64) {
            self.captured.reset();
            let mut reads = crate::read_accounting::table(TableId::ArmyOrders);
            let begin = qpc();
            let census = corder_instance::walk_listing_successors_of(
                memory,
                base,
                &mut self.rows,
                &mut self.geometry,
                roots,
                root_outcome,
                &mut self.visits,
                &mut self.successors,
            );
            let (plan_reads, plan_ticks) = self.successors.plan_gathers();
            let census_ticks =
                (qpc().saturating_sub(begin).max(0) as u64).saturating_sub(plan_ticks);
            reads.select(TableId::ArmyOrderPlans);
            plan_reads.add_current();
            drop(reads);
            self.captured.visit(
                memory,
                base,
                0,
                roots,
                &self.visits,
                &self.successors,
                census.root_outcome,
                census.outcome,
                &self.rows[..census.outcome.len()],
                &self.geometry[..census.geometry_len],
                None,
            );
            (census_ticks, plan_ticks)
        }
    }

    fn micros(ticks: u64) -> f64 {
        let mut frequency = 0;
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency);
        }
        ticks as f64 * 1e6 / frequency as f64
    }

    fn median(mut ticks: Vec<u64>) -> f64 {
        ticks.sort_unstable();
        micros(ticks[ticks.len() / 2])
    }

    /// Median and p95 of the whole pass, each table's median, and the
    /// medians of the census without its plan gathers and of the gathers,
    /// in µs.
    fn time_passes(
        memory: &impl Memory,
        world: &World,
        roots: &[corders_group::Root],
        root_outcome: Outcome,
    ) -> (f64, f64, Vec<f64>, f64, f64) {
        let mut pass = Pass::new();
        let mut tables = Vec::new();
        let mut censuses = Vec::new();
        let mut gathers = Vec::new();
        let mut run = || {
            let (census, gathered) = pass.run(memory, world.base, roots, root_outcome);
            censuses.push(census);
            gathers.push(gathered);
            tables.push(pass.captured.ticks);
        };
        // The first pass grows every arena; later passes reuse them.
        run();
        let (median_pass, p95) = bench_world::time(50, &mut run);
        let per_table = (0..N)
            .map(|table| median(tables.iter().map(|t| t[table]).collect()))
            .collect();
        pass.captured.capture();
        assert_eq!(
            &pass.captured.outcomes()[..2],
            &[
                Outcome::Rows {
                    len: world.orders as usize
                },
                Outcome::Rows {
                    len: world.groups as usize
                }
            ]
        );
        (
            median_pass,
            p95,
            per_table,
            median(censuses),
            median(gathers),
        )
    }

    #[test]
    #[ignore = "timing bench"]
    fn l3b_bench_hourly_plan_links_and_names() {
        for countries in [1_500, 5_000] {
            for named in [true, false] {
                let world = bench_world::build(
                    countries,
                    Shape {
                        named,
                        theatre_links: true,
                    },
                );
                let slice = world.slice();
                let mut roots = vec![corders_group::Root::default(); world.groups as usize];
                let root_outcome = corders_group::walk_roots(&slice, world.base, &mut roots);
                // One warm pass's source requests, per table.
                let mut pass = Pass::new();
                let mut counts = crate::read_accounting::ZERO;
                for _ in 0..2 {
                    ((), counts) = world.counted(|memory| {
                        pass.run(memory, world.base, &roots, root_outcome);
                    });
                }
                pass.captured.capture();
                let work: Vec<String> = TABLES
                    .iter()
                    .chain([&TableId::ArmyOrders])
                    .map(|table| bench_world::work(counts[table.index()]))
                    .collect();
                println!(
                    "plans-work countries={countries} named={named}: ArmyOrderPlans {}; OrdersGroupPlans {}; Fronts {}; InvasionPreparations {}; ArmyOrders census {}",
                    work[0], work[1], work[2], work[3], work[4]
                );
                for source in ["slice", "live"] {
                    let (median, p95, per_table, census, gathers) = if source == "slice" {
                        time_passes(&slice, &world, &roots, root_outcome)
                    } else {
                        time_passes(&world.live(), &world, &roots, root_outcome)
                    };
                    println!(
                        "plans countries={countries} groups={} orders={} named={named} source={source}: pass {median:.1} us (p95 {p95:.1}); ArmyOrderPlans {:.1} us, OrdersGroupPlans {:.1} us, Fronts {:.1} us; order-census {census:.1} us, census-gathers {gathers:.1} us; links {}, order-name bytes {}, group-name bytes {}",
                        world.groups,
                        world.orders,
                        per_table[0],
                        per_table[1],
                        per_table[2],
                        pass.captured.links.len,
                        pass.captured.names.len,
                        pass.captured.group_names.len
                    );
                }
                // The census alone, gathering the plan records, over the
                // callback's registry census, as the Orders job runs it.
                let live = world.live();
                let (mut rows, mut geometry, mut visits) = (Vec::new(), Vec::new(), Vec::new());
                let mut successors = corder_instance::SuccessorLists::default();
                let root_outcome = corders_group::walk_roots(&live, world.base, &mut roots);
                let (walked, walked_p95) = bench_world::time(50, || {
                    corder_instance::walk_listing_successors_of(
                        &live,
                        world.base,
                        &mut rows,
                        &mut geometry,
                        &roots,
                        root_outcome,
                        &mut visits,
                        &mut successors,
                    );
                });
                println!(
                    "census countries={countries} named={named} source=live: census of walked roots {walked:.1} us (p95 {walked_p95:.1})"
                );
            }
        }
    }
}
