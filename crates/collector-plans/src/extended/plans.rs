use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use collector_projection::projection::{Projection, project_table, scalar_table};
use engine_allocation::{copy_vec, owned_vec};
use engine_land_inputs::{cfront, cfront_section};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{ccountry, corder_instance, corders_group};
use engine_native_parts::modifier_definition::DefinitionMemo;
use engine_native_land::{invasion_preparations, order_execution};
use replay_data::{
    rows::ArmyOrderPlanRow, rows::FrontRow, rows::InvasionPreparationRow, rows::OrderChildFront,
    rows::OrderChildFrontInputs, rows::OrdersGroupPlanRow, rows::Recorded, rows::RowCodec,
    rows::TableId, rows::tables,
};
use replay_frame::frame::TableInput;
#[cfg(test)]
use replay_frame::frame::TableView;
use std::mem::size_of;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupPlans,
    TableId::Fronts,
    TableId::InvasionPreparations,
];

fn project_execution(
    inputs: engine_native::corder_instance::OrderExecutionInputs,
) -> replay_data::rows::OrderExecutionInputs {
    replay_data::rows::OrderExecutionInputs {
        route_is_ok: inputs.route_is_ok,
        paradrop: inputs
            .paradrop
            .map(|prep| replay_data::rows::ParadropPreparationInputs {
                progress: prep.progress,
                terms: prep
                    .terms
                    .map(|terms| replay_data::rows::ParadropPreparationTerms {
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
    invasions: usize,
    execution: order_execution::Capacity,
    country_references: usize,
    country_links: usize,
}

collector_primitives::capacity_max!(Capacities {
    plans,
    groups,
    fronts,
    assigned,
    child_sections,
    links,
    names,
    root_orders,
    sections,
    pairs,
    handles,
    invasions,
    execution { units, groups, orders, memberships, scalar_observations, modifiers { nodes, entries } },
    country_references,
    country_links,
});
impl Capacities {
    pub fn presize_execution(
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
    pub fn raise_for_countries(&mut self, slots: usize) {
        self.country_references = self.country_references.max(slots);
        self.country_links = self.country_links.max(slots);
    }
}
pub struct Fronts {
    rows: CaptureRows<cfront::Row>,
    sections: Arena<cfront_section::Row>,
    pairs: Arena<u32>,
    handles: Vec<u64>,
    required_handles: usize,
    bound: Option<cfront::Bound>,
    failed: bool,
    ticks: u64,
}

impl Fronts {
    fn new(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.fronts),
            sections: Arena::new(c.sections),
            pairs: Arena::new(c.pairs),
            handles: owned_vec(engine_allocation::filled(c.handles, 0)),
            required_handles: 0,
            bound: None,
            failed: false,
            ticks: 0,
        }
    }
    fn vacant() -> Self {
        Self::new(&Capacities::default())
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.sections.reset();
        self.pairs.reset();
        self.required_handles = 0;
        self.bound = None;
        self.failed = false;
        self.ticks = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.sections.allocated_bytes()
            + self.pairs.allocated_bytes()
            + self.handles.capacity() * size_of::<u64>()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.fronts);
        self.sections.grow(&mut c.sections);
        self.pairs.grow(&mut c.pairs);
        grow_capacity(&mut c.handles, self.required_handles);
        grow_vec(&mut self.handles, c.handles);
    }
    fn bind(&mut self, plans: &[corder_instance::PlanRow], source: Outcome) {
        self.bound = Some(cfront::bound(plans, source, &mut self.handles));
        self.failed = false;
    }
    fn fail(&mut self, outcome: Outcome) {
        self.rows.outcome = outcome;
        self.sections.len = 0;
        self.pairs.len = 0;
        self.bound = None;
        self.failed = true;
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize) -> u64 {
        let Some(bound) = self.bound.take() else {
            return 0;
        };
        let _reads = collector_memory::read_accounting::table(TableId::Fronts);
        let begin = qpc();
        let front = cfront::walk_bound(
            memory,
            base,
            bound,
            &self.handles,
            &mut self.rows.rows,
            &mut self.sections.first,
            &mut self.pairs.first,
        );
        self.required_handles = self.required_handles.max(front.required_handles);
        self.rows.outcome = front.outcome;
        self.sections.len = front.sections;
        self.sections.required = front.required_sections;
        self.pairs.len = front.pairs;
        self.pairs.required = front.required_pairs;
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        self.ticks += ticks;
        ticks
    }
}

pub struct Captured {
    plans: CaptureRows<corder_instance::PlanRow>,
    colors: CaptureRows<corders_group::PlanRow>,
    fronts: Fronts,
    invasions: CaptureRows<invasion_preparations::Row>,
    memo: DefinitionMemo,
    assigned: Arena<u64>,
    child_sections: Arena<corder_instance::child_fronts::Section>,
    child_scratch: corder_instance::child_fronts::Scratch,
    pub child_front_failures: [usize; 3],
    child_walk: Option<corder_instance::child_fronts::Walk>,
    links: Arena<u32>,
    names: Arena<u8>,
    group_names: Arena<u8>,
    root_orders: Arena<u32>,
    order: corder_instance::PlanOrder,
    ticks: [u64; N],
    supplied: bool,
    assigned_hosts: corder_instance::AssignedSources,
    execution: order_execution::Prepared,
    execution_countries: ccountry::NativeReferenceMemo,
}

impl Captured {
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
        Self {
            plans: CaptureRows::new(c.plans),
            colors: CaptureRows::new(c.groups),
            fronts: Fronts::new(c),
            invasions: CaptureRows::new(c.invasions),
            memo: DefinitionMemo::try_with_capacity(if placeholder { 0 } else { 3 })
                .unwrap_or_default(),
            assigned: Arena::new(c.assigned),
            child_sections: Arena::new(c.child_sections),
            child_scratch: Default::default(),
            child_front_failures: [0; 3],
            child_walk: None,
            links: Arena::new(c.links),
            names: Arena::new(c.names),
            group_names: Arena::new(c.names),
            root_orders: Arena::new(c.root_orders),
            order: corder_instance::PlanOrder::default(),
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
        }
    }
    pub fn reset(&mut self) {
        self.plans.reset();
        self.colors.reset();
        self.fronts.reset();
        self.invasions.reset();
        self.memo.clear();
        self.assigned.reset();
        self.child_sections.reset();
        self.child_front_failures = [0; 3];
        self.child_walk = None;
        self.links.reset();
        self.names.reset();
        self.group_names.reset();
        self.root_orders.reset();
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
            + self.memo.allocated_bytes()
            + self.assigned.allocated_bytes()
            + self.child_sections.allocated_bytes()
            + self.child_scratch.allocated_bytes()
            + self.links.allocated_bytes()
            + self.names.allocated_bytes()
            + self.group_names.allocated_bytes()
            + self.root_orders.allocated_bytes()
            + self.order.allocated_bytes()
            + self.assigned_hosts.allocated_bytes()
            + self.execution.allocated_bytes()
            + self.execution_countries.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.plans.grow(&mut c.plans);
        self.colors.grow(&mut c.groups);
        self.fronts.grow(c);
        self.invasions.grow(&mut c.invasions);
        self.assigned.grow(&mut c.assigned);
        self.child_sections.grow(&mut c.child_sections);
        self.links.grow(&mut c.links);
        let mut group_names = c.names;
        self.names.grow(&mut c.names);
        self.group_names.grow(&mut group_names);
        c.names = c.names.max(group_names);
        self.root_orders.grow(&mut c.root_orders);
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

    pub fn reserve_units(&mut self, units: usize) {
        let mut capacity = self.execution.capacity();
        capacity.units = capacity.units.max(units);
        let _ = self.execution.try_grow(capacity);
    }
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

    pub fn capture_unit_transports(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        inputs: Option<&collector_divisions::division_census::UnitInputs>,
        orders: &collector_divisions::orders::Orders,
    ) -> u64 {
        if !self.supplied {
            self.execution
                .fail_units("plan execution source not supplied");
            return 0;
        }
        let _reads = collector_memory::read_accounting::table(TableId::DivisionMovement);
        let started = qpc();
        let countries = self.execution_countries.view(memory, base, game_state);
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
            inputs.map_or(&[], collector_divisions::division_census::UnitInputs::seeds),
            inputs.map_or(
                collector_extended_support::outcomes::MISSING,
                collector_divisions::division_census::UnitInputs::outcome,
            ),
            order_execution::Orders {
                roots: orders.roots(),
                root_source: orders.root_outcome,
                visits: orders.visits(),
                rows: &orders.rows.rows[..orders.rows.outcome.len()],
                source: orders.rows.outcome,
                geometry: &orders.geometry[..orders.geometry_len],
                plan_rows: self.order.rows(),
                census: orders.successor_lists().census(),
            },
            sources,
            &countries,
        );
        qpc().saturating_sub(started).max(0) as u64
    }

    pub fn fail_unit_transports(&mut self, reason: &'static str) {
        self.execution.fail_units(reason);
    }

    pub fn install_order_transport(
        &mut self,
        presentation: &mut collector_divisions::extended::division_presentation::Captured,
    ) {
        presentation.install_order_transport(&mut self.execution);
    }
    #[cfg(any(test, feature = "test-support"))]
    #[expect(
        clippy::too_many_arguments,
        reason = "the visit reads roots, visits and successors with both sources and both owned tables"
    )]
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
    ) -> u64 {
        self.visit_with(
            memory,
            base,
            game_state,
            roots,
            visits,
            successors,
            root_source,
            order_source,
            owned_orders,
            owned_paths,
            None,
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "the visit reads roots, visits and successors with both sources, both owned tables and the parts"
    )]
    pub fn visit_parts(
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
        parts: &[corder_instance::PlanPart<'_>],
        part_ticks: u64,
    ) -> u64 {
        self.visit_with(
            memory,
            base,
            game_state,
            roots,
            visits,
            successors,
            root_source,
            order_source,
            owned_orders,
            owned_paths,
            Some((parts, part_ticks)),
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "the visit reads roots, visits and successors with both sources, both owned tables and the parts"
    )]
    fn visit_with(
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
        parts: Option<(&[corder_instance::PlanPart<'_>], u64)>,
    ) -> u64 {
        self.supplied = true;
        self.child_walk = None;
        self.execution_countries.reset(base, game_state);
        let begin_all = qpc();
        let before = self.ticks.iter().sum::<u64>();
        let mut read_phase = collector_memory::read_accounting::table(TABLES[1]);
        let begin = qpc();
        let color_capacity = if let Outcome::Rows { len } = root_source
            && len == roots.len()
            && len > self.colors.rows.len()
        {
            let additional = len - self.colors.rows.len();
            engine_allocation::reserve(&mut self.colors.rows, additional)
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
        let report = match parts {
            None => corder_instance::walk_plans_ordered_with_sources(
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
            ),
            Some((parts, _)) => corder_instance::stitch_plans(
                parts,
                order_source,
                &mut self.plans.rows,
                &mut self.assigned.first,
                &mut self.links.first,
                &mut self.names.first,
                &mut self.order,
                Some(&mut self.assigned_hosts),
            ),
        };
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
                plan_rows: self.order.rows(),
                census: successors.census(),
            },
            &mut self.plans.rows[..report.outcome.len()],
            report.outcome,
            &countries,
        );
        if let Outcome::Rows { len } = report.outcome {
            let areas = corder_instance::child_fronts::walk_areas(
                memory,
                base,
                game_state,
                roots,
                visits,
                owned_orders,
                owned_paths,
                order_source,
                &mut self.plans.rows[..len],
                self.order.rows(),
                successors.census(),
                &mut self.child_scratch,
            );
            self.child_sections.len = 0;
            self.child_sections.required = 0;
            self.child_front_failures = [
                areas.area_failures,
                areas.header_failures,
                areas.section_failures,
            ];
            self.child_walk = Some(areas);
        }
        self.ticks[0] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[2]);
        let begin = qpc();
        self.fronts.bind(&self.plans.rows, report.outcome);
        self.ticks[2] += qpc().saturating_sub(begin).max(0) as u64;
        read_phase.select(TABLES[3]);
        let begin = qpc();
        let invasions = invasion_preparations::walk(
            memory,
            base,
            game_state,
            visits,
            successors.census(),
            order_source,
            &self.memo,
            &countries,
            &mut self.invasions.rows,
        );
        self.invasions.outcome = invasions.outcome;
        self.ticks[3] += qpc().saturating_sub(begin).max(0) as u64;
        let total = qpc().saturating_sub(begin_all).max(0) as u64;
        self.ticks[0] += total.saturating_sub(self.ticks.iter().sum::<u64>() - before);
        let (_, census_ticks) = successors.plan_gathers();
        let census_ticks = census_ticks + parts.map_or(0, |(_, part_ticks)| part_ticks);
        self.ticks[0] += census_ticks;
        total + census_ticks
    }
    pub fn visit_child_fronts(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        group_source: Option<&collector_naval_history::extended::naval_history::NavalJob>,
    ) -> u64 {
        let Some(areas) = self.child_walk.take() else {
            return 0;
        };
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let len = self.plans.outcome.len();
        let group_source = match group_source
            .filter(|_| self.child_scratch.needs_group_source())
            .map(collector_naval_history::extended::naval_history::OrderGroupSource::new)
            .transpose()
        {
            Ok(source) => source,
            Err(_) => {
                self.fail_child_fronts();
                let ticks = qpc().saturating_sub(begin).max(0) as u64;
                self.ticks[0] += ticks;
                return ticks;
            }
        };
        let children = corder_instance::child_fronts::walk_groups(
            memory,
            base,
            roots,
            &mut self.plans.rows[..len],
            &mut self.child_sections.first,
            &mut self.child_scratch,
            group_source.as_ref(),
            areas,
        );
        drop(group_source);
        self.child_sections.len = children.sections;
        self.child_sections.required = children.required_sections;
        self.child_front_failures = [
            children.area_failures,
            children.header_failures,
            children.section_failures,
        ];
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        self.ticks[0] += ticks;
        ticks
    }

    pub fn child_fronts_allocate(&self) -> bool {
        self.child_walk.is_some() && self.child_scratch.needs_group_source()
    }

    pub fn fail_child_fronts(&mut self) {
        let len = self.plans.outcome.len();
        for plan in &mut self.plans.rows[..len] {
            plan.child_fronts = None;
        }
        self.child_walk = None;
        self.child_sections.len = 0;
        self.child_sections.required = 0;
        self.child_front_failures = [len, 0, 0];
    }

    pub fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.plans.outcome = outcome;
        self.colors.outcome = outcome;
        self.fronts.fail(outcome);
        self.invasions.fail(outcome);
        self.assigned.len = 0;
        self.child_sections.len = 0;
        self.child_front_failures = [0; 3];
        self.child_walk = None;
        self.links.len = 0;
        self.names.len = 0;
        self.group_names.len = 0;
        self.root_orders.len = 0;
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
            self.fronts.fail(outcome);
            self.invasions.outcome = outcome;
        } else if self.child_walk.is_some() {
            self.fail_child_fronts();
        }
        if self.fronts.bound.is_some() {
            self.fronts.fail(Outcome::Unavailable {
                reason: "front acquisition did not run",
            });
        }
        let mut ticks = self.ticks;
        ticks[2] += self.fronts.ticks;
        ticks
    }
    pub fn take_fronts(&mut self) -> Fronts {
        std::mem::replace(&mut self.fronts, Fronts::vacant())
    }
    pub fn install_fronts(&mut self, mut fronts: Fronts, completed: bool) {
        if !completed && !fronts.failed {
            fronts.fail(Outcome::Unavailable {
                reason: "front acquisition failed",
            });
        }
        self.fronts = fronts;
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_fronts(&mut self, memory: &impl Memory, base: usize) -> u64 {
        self.fronts.capture(memory, base)
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
                let mut sections = owned_vec(engine_allocation::vec(source.len()));
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
            self.fronts.rows.outcome,
            self.invasions.outcome,
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [replay_frame::frame::TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("unprojected plan conversion returns sampled views"),
        })
    }
    pub fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        let outcomes = self.outcomes();
        let name_slice = |(start, len): (usize, usize)| &self.names.first[start..start + len];
        let same_child_fronts = |inputs: Option<corder_instance::child_fronts::Inputs>,
                                 previous: &Recorded<Option<OrderChildFrontInputs>>| {
            let Recorded::Value(previous) = previous else {
                return false;
            };
            match (inputs, previous) {
                (None, None) => true,
                (Some(inputs), Some(previous)) => {
                    inputs.area_province_count == previous.area_province_count
                        && inputs.count == previous.count
                        && inputs.group_child_count == previous.group_child_count
                        && match (inputs.sections, &previous.sections) {
                            (None, None) => true,
                            (Some((start, len)), Some(prev_sections)) => {
                                let source = &self.child_sections.first[start..start + len];
                                source.len() == prev_sections.len()
                                    && source.iter().zip(prev_sections).all(|(a, b)| {
                                        a.group == b.group
                                            && a.segment_start == b.segment_start
                                            && a.segment_end == b.segment_end
                                    })
                            }
                            _ => false,
                        }
                }
                _ => false,
            }
        };
        let invasion = |r: &invasion_preparations::Row| InvasionPreparationRow {
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
            ..Default::default()
        };
        [
            project_table::<tables::ArmyOrderPlans, _>(
                projection.as_deref_mut(),
                outcomes[0],
                &self.plans.rows,
                |r| ((r.owner_group as u128) << 32) | u128::from(r.instance),
                |r, previous| {
                    r.front == previous.front
                        && r.front_valid == previous.front_valid
                        && r.section == previous.section
                        && r.segment_start == previous.segment_start
                        && r.segment_end == previous.segment_end
                        && r.blitz == previous.blitz
                        && r.withdraw == previous.withdraw
                        && r.fallback == previous.fallback
                        && r.virtual_order == previous.virtual_order
                        && r.has_child == previous.has_child
                        && r.owner_override == previous.owner_override
                        && previous.invasion_transports == r.invasion_transports.into()
                        && &self.links.first
                            [r.predecessors_start..r.predecessors_start + r.predecessors_len]
                            == previous.predecessors.as_slice()
                        && &self.links.first
                            [r.successors_start..r.successors_start + r.successors_len]
                            == previous.successors.as_slice()
                        && &self.assigned.first[r.assigned_start..r.assigned_start + r.assigned_len]
                            == previous.assigned_divisions.as_slice()
                        && name_slice(r.names[0]) == previous.name_prefix.as_slice()
                        && name_slice(r.names[1]) == previous.name.as_slice()
                        && name_slice(r.names[2]) == previous.name_first.as_slice()
                        && name_slice(r.names[3]) == previous.name_second.as_slice()
                        && name_slice(r.names[4]) == previous.name_unique.as_slice()
                        && name_slice(r.names[5]) == previous.name_postfix.as_slice()
                        && same_child_fronts(r.child_fronts, &previous.child_fronts)
                        && previous.execution
                            == Recorded::Value(r.execution.map(project_execution))
                },
                |r| ArmyOrderPlanRow {
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
                },
            ),
            project_table::<tables::OrdersGroupPlans, _>(
                projection.as_deref_mut(),
                outcomes[1],
                &self.colors.rows,
                |r| u128::from(r.group),
                |r, previous| {
                    r.red == previous.red
                        && r.green == previous.green
                        && r.blue == previous.blue
                        && r.alpha == previous.alpha
                        && &self.root_orders.first[r.primary_start..r.primary_start + r.primary_len]
                            == previous.primary_orders.as_slice()
                        && &self.root_orders.first
                            [r.fallback_start..r.fallback_start + r.fallback_len]
                            == previous.fallback_orders.as_slice()
                        && &self.group_names.first[r.name_start..r.name_start + r.name_len]
                            == previous.name.as_slice()
                },
                |r| OrdersGroupPlanRow {
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
                },
            ),
            project_table::<tables::Fronts, _>(
                projection.as_deref_mut(),
                outcomes[2],
                &self.fronts.rows.rows,
                |r| u128::from(r.front),
                |r, previous| {
                    let sections = &self.fronts.sections.first
                        [r.section_start..r.section_start + r.section_len];
                    if sections.len() != previous.section_ids.len()
                        || sections.len() != previous.section_ends.len()
                    {
                        return false;
                    }
                    let mut offset = 0usize;
                    for (index, section) in sections.iter().enumerate() {
                        if section.id != previous.section_ids[index] {
                            return false;
                        }
                        let Some(end) = offset.checked_add(section.pair_len) else {
                            return false;
                        };
                        if end as u32 != previous.section_ends[index] {
                            return false;
                        }
                        let Some(prev_pairs) = previous.province_pairs.get(offset..end) else {
                            return false;
                        };
                        if &self.fronts.pairs.first
                            [section.pair_start..section.pair_start + section.pair_len]
                            != prev_pairs
                        {
                            return false;
                        }
                        offset = end;
                    }
                    offset == previous.province_pairs.len()
                },
                |r| {
                    let mut row = FrontRow {
                        front: r.front,
                        ..Default::default()
                    };
                    let sections = &self.fronts.sections.first
                        [r.section_start..r.section_start + r.section_len];
                    let pairs = sections
                        .iter()
                        .try_fold(0_usize, |count, section| {
                            count.checked_add(section.pair_len)
                        })
                        .unwrap_or(usize::MAX);
                    if engine_allocation::reserve_exact(&mut row.section_ids, sections.len())
                        .is_err()
                        || engine_allocation::reserve_exact(&mut row.section_ends, sections.len())
                            .is_err()
                        || engine_allocation::reserve_exact(&mut row.province_pairs, pairs).is_err()
                    {
                        return row;
                    }
                    for section in sections {
                        row.section_ids.push(section.id);
                        row.province_pairs.extend_from_slice(
                            &self.fronts.pairs.first
                                [section.pair_start..section.pair_start + section.pair_len],
                        );
                        row.section_ends.push(row.province_pairs.len() as u32);
                    }
                    row
                },
            ),
            scalar_table::<tables::InvasionPreparations, _>(
                projection,
                outcomes[3],
                &self.invasions.rows,
                |r| RowCodec::key(&invasion(r)),
                invasion,
            ),
        ]
    }
}

#[cfg(test)]
#[path = "plans/orders_parts_fixture.rs"]
mod orders_parts_fixture;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    use std::collections::BTreeMap;

    #[test]
    fn a_one_shot_fault_shared_by_fronts_and_invasions_refuses_exactly_one() {
        struct Domain<'a> {
            image: &'a Image,
            faults: [usize; 2],
            once: bool,
            spent: std::cell::Cell<bool>,
        }
        impl Memory for Domain<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                let covers = self
                    .faults
                    .iter()
                    .any(|&fault| at <= fault && fault < at + output.len());
                if covers && (!self.once || !self.spent.replace(true)) {
                    return false;
                }
                self.image.copy(at, output)
            }
        }
        const FRONT: u64 = (1 << 32) | cfront::REFERENCE_TYPE as u64;
        let mut image = image();
        image.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_INVASION as u32,
        );
        for offset in corder_instance::NAME_STRINGS {
            image.u64(ORDER + offset + 0x18, 15);
        }
        image.u64(ORDER + corder_instance::FRONT_BINDING, FRONT);
        let root = registry::LOW_ROOTS_RVA.0 + cfront::REFERENCE_TYPE as usize * 8;
        image.u64(root, 0);
        let provinces = engine_native::cgame_state::PROVINCES;
        image.u64(provinces, 0);
        image.u32(provinces + 8, 0);
        image.u32(provinces + 12, 0);
        let run = |once: bool| {
            let memory = Domain {
                image: &image,
                faults: [root, provinces],
                once,
                spent: Default::default(),
            };
            let mut c = Captured::with_capacities(&Capacities::default());
            pass(&memory, &mut c);
            c.capture();
            let outcomes = c.outcomes();
            assert_eq!(outcomes[0], Outcome::Rows { len: 1 }, "once {once}");
            (outcomes[2], outcomes[3])
        };
        let array = "invasion preparation array unreadable or invalid";
        let (fronts, invasions) = run(true);
        assert_eq!(
            matches!(fronts, Outcome::Unavailable { .. }) as u8
                + matches!(invasions, Outcome::Unavailable { .. }) as u8,
            1,
            "{fronts:?} {invasions:?}"
        );
        assert_eq!(invasions, Outcome::Unavailable { reason: array });
        assert_eq!(fronts, Outcome::Rows { len: 0 });
        assert_eq!(
            run(false),
            (
                Outcome::Unavailable {
                    reason: "reference root unreadable"
                },
                Outcome::Unavailable { reason: array }
            )
        );
    }

    #[test]
    fn invasion_preparations_are_published_in_key_order_whatever_the_walk_order() {
        let group = |id: u64| (id << 32) | u64::from(corders_group::REFERENCE_TYPE);
        let walked =
            [(group(9), 2), (group(3), 7), (group(9), 1)].map(|(owner_group, instance)| {
                invasion_preparations::Row {
                    owner_group,
                    instance,
                    ..Default::default()
                }
            });
        let TableView::Complete(replay_data::rows::TableRows::InvasionPreparations(published)) =
            collector_projection::table_conversion::table::<tables::InvasionPreparations, _>(
                Outcome::Rows { len: 3 },
                &walked,
                |r| InvasionPreparationRow {
                    owner_group: r.owner_group,
                    instance: r.instance,
                    ..Default::default()
                },
            )
        else {
            panic!("a complete table");
        };
        assert_eq!(
            published
                .iter()
                .map(|row| (row.owner_group, row.instance))
                .collect::<Vec<_>>(),
            [(group(3), 7), (group(9), 1), (group(9), 2)]
        );
    }
    #[derive(Default)]
    pub struct Image(BTreeMap<usize, u8>);
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
        m.u64(registry::INVALID_REFERENCE_RVA.0, 0);
        m.u64(registry::LOW_ROOTS_RVA.0 + 53 * 8, 0x1000);
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
        m.u64(GROUP, corders_group::VTABLE_RVA.0 as u64);
        m.u64(GROUP + 8, (7 << 32) | 53);
        m.u32(GROUP + corders_group::COUNTRY, 1);
        m.u64(GROUP + corders_group::NAME + 0x18, 15);
        let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
        m.u64(root, 0x3800);
        m.u32(root + 8, 1);
        m.u32(root + 12, 1);
        m.u64(0x3800, ORDER as u64);
        m.put(ORDER, &[0; 0x400]);
        m.u64(ORDER, corder_instance::VTABLE_RVA.0 as u64);
        m.u64(ORDER + corder_instance::OWNER, GROUP as u64);
        m.u32(ORDER + corder_instance::ID, 9);
        m
    }
    fn orders_pass(m: &impl Memory, c: &mut Captured) -> Vec<corders_group::Root> {
        let mut groups = vec![corders_group::Root::default(); 4];
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
        );
        groups[..r.root_outcome.len()].to_vec()
    }
    pub fn pass(m: &impl Memory, c: &mut Captured) {
        let roots = orders_pass(m, c);
        c.visit_child_fronts(m, 0, &roots, None);
        c.capture_fronts(m, 0);
    }

    #[test]
    fn orders_in_parts_capture_what_the_single_job_captures() {
        use collector_divisions::orders_parts::MAX_PARTS;
        use collector_extended_support::bench_world::{self, Shape};
        for (countries, counts) in [
            (40, (1..=MAX_PARTS).collect::<Vec<_>>()),
            (1_500, vec![1, 2, MAX_PARTS]),
            (5_000, vec![1, 2, MAX_PARTS]),
        ] {
            let world = bench_world::build(
                countries,
                Shape {
                    named: true,
                    theatre_links: true,
                },
            );
            parts_capture_what_the_single_job_captures(&world, &counts, false);
        }
    }

    #[test]
    fn orders_in_parts_capture_nonempty_fronts_children_and_invasions() {
        let world = orders_parts_fixture::build();
        let counts: Vec<_> = (1..=collector_divisions::orders_parts::MAX_PARTS).collect();
        parts_capture_what_the_single_job_captures(&world, &counts, true);
    }

    #[cfg(test)]
    fn assert_nonempty_order_parts_fixture(
        orders: &collector_divisions::orders::Orders,
        plans: &Captured,
    ) {
        use orders_parts_fixture::{FRONTS, GROUPS, ORDERS};
        assert_eq!(orders.root_outcome, Outcome::Rows { len: GROUPS });
        assert_eq!(orders.rows.outcome, Outcome::Rows { len: ORDERS });
        assert_eq!(orders.geometry_len, ORDERS * 3);
        assert_eq!(orders.successor_lists().census().len(), ORDERS);
        for row in &orders.rows.rows[..ORDERS] {
            assert_eq!((row.path_len, row.target_len), (2, 1));
            assert_eq!(
                &orders.geometry[row.path_start..row.path_start + row.path_len],
                &[1, 2]
            );
            let group_index = (row.owner_group >> 32) as u32 - 1;
            assert_eq!(orders.geometry[row.target_start], 17 + group_index);
        }
        assert_eq!(
            plans.outcomes(),
            [
                Outcome::Rows { len: ORDERS },
                Outcome::Rows { len: GROUPS },
                Outcome::Rows { len: FRONTS },
                Outcome::Rows { len: GROUPS },
            ]
        );
        assert_eq!(plans.assigned.len, GROUPS * 2);
        assert_eq!(
            plans.assigned_hosts.outcome(),
            Outcome::Rows { len: GROUPS * 2 }
        );
        assert_eq!(plans.links.len, GROUPS * 4);
        assert!(plans.names.len > 0);
        assert!(plans.group_names.len > 0);
        assert_eq!(plans.root_orders.len, ORDERS);
        assert_eq!(plans.child_front_failures, [0; 3]);
        assert_eq!(plans.child_sections.len, GROUPS * 2);
        assert_eq!(plans.fronts.sections.len, FRONTS * 2);
        assert_eq!(plans.fronts.pairs.len, FRONTS * 4);
        assert_eq!(
            &plans.fronts.pairs.first[..FRONTS * 4],
            &[1, 2, 2, 1, 1, 2, 2, 1]
        );
        for (index, sections) in plans.fronts.sections.first[..FRONTS * 2]
            .chunks_exact(2)
            .enumerate()
        {
            assert_eq!((sections[0].id, sections[1].id), (9, 3), "front {index}");
            assert_eq!((sections[0].pair_len, sections[1].pair_len), (4, 0));
        }
        let census: Vec<_> = plans
            .order
            .rows()
            .iter()
            .map(|&row| plans.plans.rows[row as usize].key())
            .collect();
        let expected: Vec<_> = (1..=GROUPS)
            .flat_map(|group| {
                let handle = ((group as u64) << 32) | u64::from(corders_group::REFERENCE_TYPE);
                (1..=3).map(move |instance| (u128::from(handle) << 32) | instance)
            })
            .collect();
        assert_eq!(census, expected);
        for plan in &plans.plans.rows[..ORDERS] {
            assert_eq!(plan.front_valid, 1);
            if plan.instance == 1 {
                let child = plan.child_fronts.expect("line child area");
                assert_eq!(child.area_province_count, 4);
                assert_eq!(child.count, Some(2));
                assert_eq!(child.group_child_count, Some(2));
                let (start, len) = child
                    .sections
                    .expect("accepted Naval source and child payload");
                assert_eq!(len, 2);
                let sections = &plans.child_sections.first[start..start + len];
                assert_eq!(
                    (sections[0].segment_start, sections[0].segment_end),
                    (-1, 0)
                );
                assert_eq!((sections[1].segment_start, sections[1].segment_end), (0, 1));
                assert!(sections.iter().all(|section| section.group != 0));
            } else {
                assert_eq!(plan.assigned_len, 1);
            }
        }
        for (index, invasion) in plans.invasions.rows[..GROUPS].iter().enumerate() {
            assert_eq!(
                (invasion.instance, invasion.target, invasion.unit_owner),
                (3, 2, 1)
            );
            assert_eq!(invasion.progress, 123_456 + index as i64);
        }
    }

    #[cfg(test)]
    fn parts_capture_what_the_single_job_captures(
        world: &collector_extended_support::bench_world::World,
        counts: &[usize],
        nonempty: bool,
    ) {
        use collector_divisions::orders::Orders;
        use collector_divisions::orders_parts::Split;
        let memory = world.slice();
        let observed = |orders: &Orders, plans: &mut Captured| {
            let mut naval = collector_naval_history::extended::naval_history::NavalJob::new();
            if nonempty {
                naval.capture_roots(orders.roots(), orders.root_outcome);
                let report = collector_naval_history::extended::naval_history::copy_naval_refresh(
                    &memory,
                    collector_naval_history::extended::naval_history::NavalCopyContext {
                        base: world.base,
                        game_state: world.game_state,
                        hour: 1,
                        force_full: false,
                    },
                    &mut naval,
                );
                assert!(report.raw_ok, "same-pass Naval OOB copy must complete");
            }
            plans.visit_child_fronts(
                &memory,
                world.base,
                orders.roots(),
                nonempty.then_some(&naval),
            );
            plans.capture_fronts(&memory, world.base);
            if nonempty {
                assert_nonempty_order_parts_fixture(orders, plans);
            }
            let census: Vec<_> = plans
                .order
                .rows()
                .iter()
                .map(|&row| plans.plans.rows[row as usize].key())
                .collect();
            format!(
                "{:?}",
                (
                    orders.rows.outcome,
                    orders.root_outcome,
                    &orders.rows.rows[..orders.rows.outcome.len()],
                    &orders.geometry[..orders.geometry_len],
                    census,
                    (
                        &plans.assigned.first[..plans.assigned.len],
                        plans.assigned_hosts.pointers(),
                        &plans.links.first[..plans.links.len],
                        &plans.names.first[..plans.names.len],
                        &plans.group_names.first[..plans.group_names.len],
                        &plans.root_orders.first[..plans.root_orders.len],
                        &plans.child_sections.first[..plans.child_sections.len],
                        plans.child_front_failures,
                        &plans.fronts.sections.first[..plans.fronts.sections.len],
                        &plans.fronts.pairs.first[..plans.fronts.pairs.len],
                    ),
                    plans.tables(),
                )
            )
        };
        let mut single = Orders::vacant();
        single.walk_roots(&memory, world.base);
        single.walk(&memory, world.base);
        assert_eq!(
            single.rows.outcome,
            Outcome::Rows {
                len: world.orders as usize
            }
        );
        let mut plans = Captured::with_capacities(&Capacities::default());
        plans.visit(
            &memory,
            world.base,
            world.game_state,
            single.roots(),
            single.visits(),
            single.successor_lists(),
            single.root_outcome,
            single.rows.outcome,
            &single.rows.rows[..single.rows.outcome.len()],
            &single.geometry[..single.geometry_len],
        );
        assert_eq!(
            plans.plans.outcome,
            Outcome::Rows {
                len: world.orders as usize
            }
        );
        let expected = observed(&single, &mut plans);
        for &parts in counts {
            let mut orders = Orders::vacant();
            let mut plans = Captured::with_capacities(&Capacities::default());
            for repeat in 0..if nonempty { 2 } else { 1 } {
                orders.walk_roots(&memory, world.base);
                orders.begin_parts(Split::even(parts, orders.roots()), 10_000_000);
                orders.prepare_parts();
                assert_eq!(orders.part_count(), parts);
                if repeat == 0 {
                    orders.walk_first_part(&memory, world.base);
                }
                for step in 1..parts {
                    let index = if repeat == 0 { step } else { parts - step };
                    let mut part = orders.take_part(index).unwrap();
                    part.run(&memory, world.base);
                    orders.return_part(index, part);
                }
                if repeat != 0 {
                    orders.walk_first_part(&memory, world.base);
                }
                let part_ticks = orders.stitch();
                assert!(!orders.failed());
                let plan_parts = orders.plan_parts().unwrap();
                plans.visit_parts(
                    &memory,
                    world.base,
                    world.game_state,
                    orders.roots(),
                    orders.visits(),
                    orders.successor_lists(),
                    orders.root_outcome,
                    orders.rows.outcome,
                    &orders.rows.rows[..orders.rows.outcome.len()],
                    &orders.geometry[..orders.geometry_len],
                    &plan_parts[..parts],
                    part_ticks,
                );
                assert_eq!(
                    observed(&orders, &mut plans),
                    expected,
                    "{} groups, {parts} parts, pass {repeat}",
                    world.groups
                );
            }
        }
    }
    pub fn execution_image() -> Image {
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

    pub fn execution_capture() -> Captured {
        let mut capacities = Capacities::default();
        capacities.raise_for_countries(5);
        capacities.execution.units = 1;
        capacities.execution.scalar_observations = 32;
        Captured::with_capacities(&capacities)
    }

    #[test]
    fn attach_execution_hint_follows_native_graph() {
        let mut world = crate::unit_fixture::build(crate::unit_fixture::Shape {
            countries: 24,
            divisions: 0,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        world.bytes[engine_native::cgame_state::SINGLETON_RVA.0
            ..engine_native::cgame_state::SINGLETON_RVA.0 + 8]
            .copy_from_slice(&(world.state as u64).to_le_bytes());
        let mut image = execution_image();
        image.u64(GROUP, (world.base + corders_group::VTABLE_RVA.0) as u64);
        image.u64(ORDER, (world.base + corder_instance::VTABLE_RVA.0) as u64);
        image.u64(world.base + registry::LOW_ROOTS_RVA.0 + 53 * 8, 0x1000);
        image.u64(world.base + registry::INVALID_REFERENCE_RVA.0, 0);
        struct Combined<'a> {
            image: &'a Image,
            world: &'a crate::unit_fixture::World,
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
        use collector_states::wars_test_kit as kit;
        use std::cell::Cell;
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
                    .copy_from_slice(&(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
                bytes[RESERVED + ccountry::ID..RESERVED + ccountry::ID + 4]
                    .copy_from_slice(&0_u32.to_le_bytes());
            }
            let memory = Counted {
                bytes: &bytes,
                calls: Cell::new(0),
            };
            let mut storage = engine_allocation::shared(
                engine_native_intelligence::country_memory::Cache::try_with_capacity(64).unwrap(),
            )
            .unwrap();
            let countries = engine_native_intelligence::country_memory::Cache::prepare_shared(
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
            let TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(rows)) =
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
                TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(mut rows)),
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
        assert_eq!(prep.terms, None);
        for (offset, expected) in [
            (
                corder_instance::CONDITION_CACHE,
                replay_data::rows::OrderExecutionInputs::default(),
            ),
            (
                corder_instance::ACCUMULATED_PREPARATION,
                replay_data::rows::OrderExecutionInputs {
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
    fn child_fronts_wait_for_their_group_phase_whose_absence_refuses_only_them() {
        let m = image();
        let mut c = Captured::with_capacities(&Capacities::default());
        pass(&m, &mut c);
        c.capture();
        assert_eq!(c.child_front_failures, [0; 3]);
        let complete = c.tables();
        assert!(matches!(
            &complete[0],
            TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(rows)) if rows.len() == 1
        ));
        assert!(!c.child_fronts_allocate());
        assert_eq!(c.visit_child_fronts(&m, 0, &[], None), 0);
        assert_eq!(c.child_front_failures, [0; 3]);

        c.reset();
        orders_pass(&m, &mut c);
        assert!(!c.child_fronts_allocate());
        c.capture_fronts(&m, 0);
        c.capture();
        assert_eq!(c.child_front_failures, [1, 0, 0]);
        assert_eq!(c.tables(), complete);

        c.reset();
        orders_pass(&m, &mut c);
        c.fail_child_fronts();
        assert_eq!(c.visit_child_fronts(&m, 0, &[], None), 0);
        c.capture_fronts(&m, 0);
        c.capture();
        assert_eq!(c.child_front_failures, [1, 0, 0]);
        assert_eq!(c.tables(), complete);
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
            matches!(&c.tables()[0], TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(rows)) if rows[0].blitz == 255)
        );
        assert!(
            matches!(&c.tables()[1], TableView::Complete(replay_data::rows::TableRows::OrdersGroupPlans(rows)) if rows[0].red == 0x7fc01234)
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
        c.fronts.pairs.required = capacity.pairs + 1;
        c.fronts.required_handles = capacity.handles + 1;
        c.grow(&mut capacity);
        assert!(capacity.assigned > 16384 && capacity.pairs > 65536 && capacity.handles > 2048);
        assert_eq!(c.assigned.first.len(), capacity.assigned);
        assert_eq!(c.fronts.pairs.first.len(), capacity.pairs);
        assert_eq!(c.fronts.handles.len(), capacity.handles);
        c.reset();
        c.capture();
        assert!(
            c.outcomes()
                .iter()
                .all(|v| matches!(v, Outcome::Unavailable { .. }))
        );
    }

    #[test]
    fn an_unreadable_invasion_withholds_only_its_own_table() {
        let mut m = image();
        m.u32(
            ORDER + corder_instance::TYPE,
            corder_instance::TYPE_INVASION as u32,
        );
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

    #[cfg(test)]
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

    #[test]
    fn arena_spans_are_exact_across_rows_growth_and_reuse() {
        const SECOND: usize = 0x4800;
        let build = |group_name: &[u8], unique: &[u8]| {
            let mut m = image();
            let root = GROUP + corders_group::PRIMARY_ORDER_DATA;
            m.u64(root, 0x3800);
            m.u32(root + 8, 2);
            m.u32(root + 12, 2);
            m.u64(0x3808, SECOND as u64);
            m.put(SECOND, &[0; 0x400]);
            m.u64(SECOND, corder_instance::VTABLE_RVA.0 as u64);
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
                TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay_data::rows::TableRows::OrdersGroupPlans(groups)),
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

    #[test]
    fn reused_captures_record_every_rename_as_a_fresh_capture_does() {
        const UNIQUE: usize = 0x5000;
        const GROUP_NAME: usize = 0x5100;
        let unique_at = ORDER + corder_instance::NAME_STRINGS[4];
        let group_at = GROUP + corders_group::NAME;
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
                TableView::Complete(replay_data::rows::TableRows::ArmyOrderPlans(plans)),
                TableView::Complete(replay_data::rows::TableRows::OrdersGroupPlans(groups)),
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
            (b"Operation Barbarossa", true, b"Heeresgruppe Mitte", true),
            (b"Operation Zitadelle!", true, b"Heeresgruppe Sued!", true),
            (
                b"Operation Blau",
                true,
                b"Heeresgruppe Nord, 16. Armee",
                true,
            ),
            (b"Op Uranus", false, b"Army 1", false),
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
            execution: order_execution::Capacity::default(),
            country_references: 0,
            country_links: 0,
        };
        for (hour, (unique, unique_heap, group, group_heap)) in hours.into_iter().enumerate() {
            string(&mut m, unique_at, unique, unique_heap.then_some(UNIQUE));
            string(&mut m, group_at, group, group_heap.then_some(GROUP_NAME));
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

