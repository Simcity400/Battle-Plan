use collector_extended_support::outcomes::elapsed;
use collector_memory::read_accounting;
use collector_primitives::{
    clock::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{corder_instance, corders_group};
use replay_data::rows::TableId;
use std::mem::size_of;

const NOT_WALKED: Outcome = Outcome::Unavailable {
    reason: "army order census not acquired",
};

pub struct Orders {
    pub rows: CaptureRows<corder_instance::Row>,
    pub geometry: Vec<u32>,
    pub geometry_len: usize,
    pub geometry_required: usize,
    pub groups: Vec<corders_group::Root>,
    pub groups_required: usize,
    pub visits: Vec<corder_instance::Visit>,
    pub visits_required: usize,
    pub successors: corder_instance::SuccessorLists,
    pub root_outcome: Outcome,
    pub roots_walked: bool,
    pub ticks: u64,
    parts: [Option<Box<crate::orders_parts::Part>>; crate::orders_parts::MAX_PARTS],
    part_count: usize,
    split: crate::orders_parts::Split,
    frequency: u64,
    next_split: crate::orders_parts::Split,
    failed: bool,
}

impl Orders {
    /// # Safety
    /// Supply capacities measured for the active session's sizing pass, or call from an isolated
    /// fixture without a live attachment. Allocation failure invokes the process-wide collector
    /// allocation-failure handler.
    pub unsafe fn new(rows: usize, geometry: usize, groups: usize, visits: usize) -> Self {
        Self {
            rows: CaptureRows::new(rows),
            geometry: engine_allocation::owned_vec(engine_allocation::default_vec(geometry)),
            geometry_len: 0,
            geometry_required: 0,
            groups: engine_allocation::owned_vec(engine_allocation::default_vec::<
                corders_group::Root,
            >(groups)),
            groups_required: 0,
            visits: engine_allocation::owned_vec(engine_allocation::default_vec::<
                corder_instance::Visit,
            >(visits)),
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
    pub fn vacant() -> Self {
        // SAFETY: Zero capacities perform no heap allocation and cannot invoke the failure handler.
        unsafe { Self::new(0, 0, 0, 0) }
    }
    pub fn reset(&mut self) {
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
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.geometry.capacity() * size_of::<u32>()
            + self.groups.capacity() * size_of::<corders_group::Root>()
            + self.visits.capacity() * size_of::<corder_instance::Visit>()
            + self.successors.allocated_bytes()
            + self.parts_allocated_bytes()
    }
    pub fn parts_allocated_bytes(&self) -> usize {
        self.parts
            .iter()
            .flatten()
            .map(|part| part.allocated_bytes())
            .sum()
    }
    /// # Safety
    /// Run only at the map sizing or frame-growth boundary with exclusive `&mut` access to this
    /// buffer (or in an isolated fixture). Nonzero growth invokes the process-wide collector
    /// allocation-failure handler on OOM.
    pub unsafe fn grow(
        &mut self,
        rows: &mut usize,
        geometry: &mut usize,
        groups: &mut usize,
        visits: &mut usize,
    ) {
        *rows = (*rows).max(self.rows.outcome.len());
        *geometry = (*geometry).max(self.geometry_required);
        *visits = (*visits).max(self.visits_required);
        self.rows.grow(rows);
        grow_capacity(geometry, self.geometry_required);
        grow_capacity(groups, self.groups_required);
        grow_capacity(visits, self.visits_required);
        grow_vec(&mut self.geometry, *geometry);
        grow_vec(&mut self.groups, *groups);
        grow_vec(&mut self.visits, *visits);
    }
    pub fn fail_roots(&mut self, reason: &'static str) {
        self.root_outcome = Outcome::Unavailable { reason };
        self.roots_walked = true;
    }
    pub fn walk_roots(&mut self, memory: &impl Memory, base: usize) {
        let _reads = read_accounting::table(TableId::ArmyOrders);
        let begin = qpc();
        self.root_outcome = corders_group::walk_roots(memory, base, &mut self.groups);
        self.roots_walked = true;
        self.ticks = elapsed(begin);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn walk(&mut self, memory: &impl Memory, base: usize) {
        let mut reads = read_accounting::table(TableId::ArmyOrders);
        self.rows.outcome = NOT_WALKED;
        let begin = qpc();
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
        let (plan_reads, plan_ticks) = self.successors.plan_gathers();
        self.ticks = self.ticks.saturating_sub(plan_ticks);
        reads.select(TableId::ArmyOrderPlans);
        memory.add_deferred_reads(plan_reads);
    }
    pub fn fail(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        self.rows.outcome = outcome;
        self.root_outcome = outcome;
        self.roots_walked = false;
        self.geometry_len = 0;
        self.ticks = 0;
        self.failed = true;
    }
    pub fn begin_parts(&mut self, split: crate::orders_parts::Split, frequency: u64) {
        self.split = split;
        self.frequency = frequency;
        self.next_split = crate::orders_parts::Split::default();
    }
    pub fn prepare_parts(&mut self) {
        let count = self.split.parts();
        let ranges = self.split.ranges(self.roots());
        let mut result = Ok(());
        for (index, range) in ranges.into_iter().enumerate().take(count) {
            if self.parts[index].is_none() {
                match engine_allocation::boxed(crate::orders_parts::Part::new()) {
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
    pub fn next_split(&self) -> crate::orders_parts::Split {
        self.next_split
    }
    pub fn part_count(&self) -> usize {
        self.part_count
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
    pub fn take_part(&mut self, index: usize) -> Option<Box<crate::orders_parts::Part>> {
        self.parts.get_mut(index)?.take()
    }
    pub fn return_part(&mut self, index: usize, part: Box<crate::orders_parts::Part>) {
        self.parts[index] = Some(part);
    }
    pub fn walk_first_part(&mut self, memory: &impl Memory, base: usize) {
        if self.part_count == 0 {
            return;
        }
        if let Some(part) = self.parts[0].as_deref_mut() {
            part.run(memory, base);
        }
    }
    pub fn stitch(&mut self) -> u64 {
        if self.failed {
            return 0;
        }
        let count = self.part_count;
        if count == 0 {
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
    fn install_census(&mut self, walk: corder_instance::Walk) {
        self.roots_walked = false;
        self.rows.outcome = walk.outcome;
        self.root_outcome = walk.root_outcome;
        self.geometry_len = walk.geometry_len;
        self.geometry_required = walk.required_geometry;
        self.groups_required = walk.required_groups;
        self.visits_required = walk.minimum_visits.max(walk.visited.saturating_mul(2));
    }
    pub fn plan_parts(
        &self,
    ) -> Option<[corder_instance::PlanPart<'_>; crate::orders_parts::MAX_PARTS]> {
        let views = self
            .parts
            .each_ref()
            .map(|part| part.as_deref()?.plan_part());
        let first = views[0].filter(|_| views[..self.part_count].iter().all(Option::is_some))?;
        Some(views.map(|view| view.unwrap_or(first)))
    }
    pub fn plan_split(&mut self) {
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
    pub fn roots(&self) -> &[corders_group::Root] {
        &self.groups[..self.root_outcome.len().min(self.groups.len())]
    }
    pub fn visits(&self) -> &[corder_instance::Visit] {
        &self.visits
    }
    pub fn successor_lists(&self) -> &corder_instance::SuccessorLists {
        &self.successors
    }
}
