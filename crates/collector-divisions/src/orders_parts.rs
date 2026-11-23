use collector_primitives::clock::qpc;
use engine_native::{
    Memory, corder_instance, corder_instance::AssignedSources, corder_instance::CensusPart,
    corder_instance::PartWalk, corder_instance::PlanOrder, corder_instance::PlanPart,
    corder_instance::PlanRow, corder_instance::PlanWalk, corder_instance::Row,
    corder_instance::SuccessorLists, corder_instance::Visit, corders_group::Root, walk::Outcome,
};
use replay_data::rows::TableId;
use std::{mem::size_of, ops::Range};

pub const MAX_PARTS: usize = 8;
pub const PART_NANOS: u64 = 250_000;
const GROUP_WEIGHT: u64 = 10;
const ORDER_WEIGHT: u64 = 17;

pub struct Part {
    roots: Vec<Root>,
    root_outcome: Outcome,
    range: Range<usize>,
    rows: Vec<Row>,
    geometry: Vec<u32>,
    visits: Vec<Visit>,
    lists: SuccessorLists,
    census: Option<PartWalk>,
    plans: Vec<PlanRow>,
    assigned: Vec<u64>,
    links: Vec<u32>,
    names: Vec<u8>,
    order: PlanOrder,
    hosts: AssignedSources,
    plan: Option<PlanWalk>,
    pub census_ticks: u64,
    pub plan_ticks: u64,
}

impl Part {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            root_outcome: Outcome::Unavailable {
                reason: "army order part not walked",
            },
            range: 0..0,
            rows: Vec::new(),
            geometry: Vec::new(),
            visits: Vec::new(),
            lists: SuccessorLists::default(),
            census: None,
            plans: Vec::new(),
            assigned: Vec::new(),
            links: Vec::new(),
            names: Vec::new(),
            order: PlanOrder::default(),
            hosts: AssignedSources::default(),
            plan: None,
            census_ticks: 0,
            plan_ticks: 0,
        }
    }

    pub fn take_roots(
        &mut self,
        roots: &[Root],
        outcome: Outcome,
        range: Range<usize>,
    ) -> Result<(), &'static str> {
        self.roots.clear();
        engine_allocation::reserve(&mut self.roots, roots.len())
            .map_err(|_| "army order part root allocation failed")?;
        self.roots.extend_from_slice(roots);
        self.root_outcome = outcome;
        self.range = range;
        self.census = None;
        self.plan = None;
        self.census_ticks = 0;
        self.plan_ticks = 0;
        Ok(())
    }

    pub fn run(&mut self, memory: &impl Memory, base: usize) {
        let mut reads = collector_memory::read_accounting::table(TableId::ArmyOrders);
        let begin = qpc();
        let census = corder_instance::walk_listing_successors_part(
            memory,
            base,
            &mut self.rows,
            &mut self.geometry,
            &self.roots,
            self.root_outcome,
            self.range.clone(),
            &mut self.visits,
            &mut self.lists,
        );
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        let (plan_reads, gather_ticks) = self.lists.plan_gathers();
        self.census_ticks = ticks.saturating_sub(gather_ticks);
        reads.select(TableId::ArmyOrderPlans);
        memory.add_deferred_reads(plan_reads);
        self.census = Some(census);
        let begin = qpc();
        let plan = corder_instance::walk_plans_ordered_with_sources(
            memory,
            base,
            &self.roots[..self.root_outcome.len().min(self.roots.len())],
            &self.visits,
            &self.lists,
            census.walk.outcome,
            &mut self.plans,
            &mut self.assigned,
            &mut self.links,
            &mut self.names,
            &mut self.order,
            Some(&mut self.hosts),
        );
        self.plan_ticks = (qpc().saturating_sub(begin).max(0) as u64).saturating_add(gather_ticks);
        self.plan = Some(plan);
    }

    pub fn census_part(&self) -> Option<CensusPart<'_>> {
        Some(CensusPart {
            walk: self.census?,
            rows: &self.rows,
            geometry: &self.geometry,
            visits: &self.visits,
        })
    }

    pub fn plan_part(&self) -> Option<PlanPart<'_>> {
        let entered = self.census?.walk.visited;
        Some(PlanPart {
            orders: &self.rows[..entered.min(self.rows.len())],
            walk: self.plan?,
            rows: &self.plans,
            assigned: &self.assigned,
            links: &self.links,
            names: &self.names,
            order: &self.order,
            hosts: Some(&self.hosts),
        })
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.roots.capacity() * size_of::<Root>()
            + self.rows.capacity() * size_of::<Row>()
            + self.geometry.capacity() * size_of::<u32>()
            + self.visits.capacity() * size_of::<Visit>()
            + self.lists.allocated_bytes()
            + self.plans.capacity() * size_of::<PlanRow>()
            + self.assigned.capacity() * size_of::<u64>()
            + self.links.capacity() * size_of::<u32>()
            + self.names.capacity()
            + self.order.allocated_bytes()
            + self.hosts.allocated_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    parts: usize,
    starts: [u64; MAX_PARTS - 1],
}

impl Default for Split {
    fn default() -> Self {
        Self {
            parts: 1,
            starts: [u64::MAX; MAX_PARTS - 1],
        }
    }
}

impl Split {
    pub fn parts(&self) -> usize {
        self.parts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn even(parts: usize, roots: &[Root]) -> Self {
        let mut split = Self {
            parts,
            ..Self::default()
        };
        for (part, start) in split.starts.iter_mut().enumerate().take(parts - 1) {
            *start = roots
                .get(roots.len() * (part + 1) / parts)
                .map_or(u64::MAX, |root| root.handle);
        }
        split
    }

    pub fn ranges(&self, roots: &[Root]) -> [Range<usize>; MAX_PARTS] {
        let mut ranges = std::array::from_fn(|_| roots.len()..roots.len());
        let mut start = 0;
        for (part, range) in ranges.iter_mut().enumerate().take(self.parts) {
            let end = if part + 1 == self.parts {
                roots.len()
            } else {
                roots
                    .partition_point(|root| root.handle < self.starts[part])
                    .max(start)
            };
            *range = start..end;
            start = end;
        }
        ranges
    }

    pub fn plan(ticks: u64, frequency: u64, roots: &[Root], rows: &[Row]) -> Self {
        let target = (u128::from(frequency) * u128::from(PART_NANOS) / 1_000_000_000).max(1);
        let parts = u128::from(ticks)
            .div_ceil(target)
            .clamp(1, MAX_PARTS as u128) as usize;
        let mut split = Self {
            parts,
            ..Self::default()
        };
        if parts == 1 || roots.is_empty() {
            split.parts = 1;
            return split;
        }
        let mut orders = rows.iter().map(|row| row.owner_group).peekable();
        let weights = roots.iter().map(|root| {
            let mut count = 0;
            while orders.next_if(|&group| group < root.handle).is_some() {}
            while orders.next_if(|&group| group == root.handle).is_some() {
                count += 1;
            }
            GROUP_WEIGHT + ORDER_WEIGHT * count
        });
        let total = roots.len() as u64 * GROUP_WEIGHT + rows.len() as u64 * ORDER_WEIGHT;
        let mut part = 0;
        let mut sum = 0;
        for (root, weight) in roots.iter().zip(weights) {
            while part + 1 < parts && sum >= total * (part as u64 + 1) / parts as u64 {
                split.starts[part] = root.handle;
                part += 1;
            }
            sum += weight;
        }
        split
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(count: u64) -> Vec<Root> {
        (1..=count)
            .map(|id| {
                let mut root = Root::default();
                root.handle = (id << 32) | 53;
                root
            })
            .collect()
    }

    #[test]
    fn the_split_follows_measured_time_and_divides_per_group_work() {
        let groups = roots(8);
        let rows: Vec<Row> = groups[..4]
            .iter()
            .flat_map(|root| {
                (1..=2).map(|instance| Row {
                    owner_group: root.handle,
                    instance,
                    ..Row::default()
                })
            })
            .collect();
        let target = PART_NANOS / 1_000;
        for frequency in [10_000_000, 1_000_000] {
            let ticks = |micros: u64| micros * frequency / 1_000_000;
            let parts = |micros| Split::plan(ticks(micros), frequency, &groups, &rows).parts();
            assert_eq!(parts(target / 2), 1, "{frequency}");
            assert_eq!(parts(target), 1, "{frequency}");
            assert_eq!(parts(target + 1), 2, "{frequency}");
            assert_eq!(parts(2 * target), 2, "{frequency}");
            assert_eq!(parts(100 * target), MAX_PARTS, "{frequency}");
        }
        let frequency = 10_000_000;
        let split = Split::plan(2 * target * 10, frequency, &groups, &rows);
        let ranges = split.ranges(&groups);
        assert_eq!(ranges[0], 0..3);
        assert_eq!(ranges[1], 3..8);
        assert!(ranges[2].is_empty() && ranges[3].is_empty());
        let mut next = roots(9);
        next.remove(1);
        let ranges = split.ranges(&next);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, ranges[1].start);
        assert_eq!(ranges[1].end, next.len());
        assert_eq!(Split::default().ranges(&next)[0], 0..next.len());
    }
}
