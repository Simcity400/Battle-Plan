//! The Orders job's census and plan walk in parts over consecutive ranges of
//! the handle-sorted order groups, joined by the OrdersStitch job into the
//! census and plans one walk over all groups takes
//! (`engine_native::corder_instance::stitch_census`, `stitch_plans`). Each part
//! reads its own groups' orders only; the stitch reads nothing.
//!
//! The split is sized from live time: the next hour's part count is last
//! hour's measured census and plan-walk time over [`PART_NANOS`], bounded
//! by the [`MAX_PARTS`] job slots reserved for it, and its ranges cut the
//! groups where last hour's per-group work divides evenly. The callback
//! carries the split from hour to hour, whichever pooled frame each hour
//! uses.
use crate::sampling::qpc;
use engine_native::{
    Memory, corder_instance, corder_instance::AssignedSources, corder_instance::CensusPart,
    corder_instance::PartWalk, corder_instance::PlanOrder, corder_instance::PlanPart,
    corder_instance::PlanRow, corder_instance::PlanWalk, corder_instance::Row,
    corder_instance::SuccessorLists, corder_instance::Visit, corders_group::Root, walk::Outcome,
};
use replay_data::rows::TableId;
use std::{mem::size_of, ops::Range};

/// The part jobs an hour can hold: the Orders job's own part and the
/// OrdersPart jobs' (`Role::OrdersPart` reserves `MAX_PARTS - 1` slots).
pub(crate) const MAX_PARTS: usize = 8;
/// The census and plan-walk worker time one part targets, in nanoseconds:
/// 0.25 ms, about a part's share of the root census that precedes it. Live,
/// 1943's census and plan walk (828 groups, 1,185 orders) take about 2.0 ms,
/// so eight parts, and 1936's (475 groups, 415 orders) about 0.7 ms, so
/// three. A part costs its dispatch and its share of the stitch, tens of
/// microseconds.
pub(crate) const PART_NANOS: u64 = 250_000;
/// A group's relative cost against one order's, for the cuts: the census
/// reads each group's rosters (about 1 us) and each order's census and plan
/// (about 1.7 us).
const GROUP_WEIGHT: u64 = 10;
const ORDER_WEIGHT: u64 = 17;

/// One part's census and plan walk over its groups, with its own storage,
/// reused across hours.
pub(crate) struct Part {
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
    /// The census's own ticks, without the plan records it gathered.
    pub(crate) census_ticks: u64,
    /// The plan walk's ticks with the census's plan-record gathers.
    pub(crate) plan_ticks: u64,
}

impl Part {
    pub(crate) fn new() -> Self {
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

    /// This pass's roots and the part's range of them, from the root census
    /// job; grows on that worker.
    pub(crate) fn take_roots(
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

    /// The part's census and plan walk, as the single Orders job walks them
    /// for its groups: the census charged to ArmyOrders, the plan walk and
    /// the plan records the census gathered to ArmyOrderPlans.
    pub(crate) fn run(&mut self, memory: &impl Memory, base: usize) {
        let mut reads = crate::read_accounting::table(TableId::ArmyOrders);
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
        plan_reads.add_current();
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

    /// The part's census as the stitch reads it; `None` for a part that did
    /// not run.
    pub(crate) fn census_part(&self) -> Option<CensusPart<'_>> {
        Some(CensusPart {
            walk: self.census?,
            rows: &self.rows,
            geometry: &self.geometry,
            visits: &self.visits,
        })
    }

    /// The part's plan walk as the stitch reads it; `None` for a part that
    /// did not run.
    pub(crate) fn plan_part(&self) -> Option<PlanPart<'_>> {
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

    /// The part's storage, its own allocation included.
    pub(crate) fn allocated_bytes(&self) -> usize {
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

/// How the next hour splits its groups: the part count and the handle each
/// later part starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Split {
    parts: usize,
    starts: [u64; MAX_PARTS - 1],
}

impl Default for Split {
    /// One part: the first hour, or after an hour without a complete census.
    fn default() -> Self {
        Self {
            parts: 1,
            starts: [u64::MAX; MAX_PARTS - 1],
        }
    }
}

impl Split {
    pub(crate) fn parts(&self) -> usize {
        self.parts
    }

    /// `parts` parts with as many of `roots` each as can be.
    #[cfg(test)]
    pub(crate) fn even(parts: usize, roots: &[Root]) -> Self {
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

    /// The part ranges of this pass's handle-sorted `roots`: part `k` takes
    /// the groups whose handles reach its start handle and not the next
    /// part's. Ranges are consecutive and cover every root; some may be empty.
    pub(crate) fn ranges(&self, roots: &[Root]) -> [Range<usize>; MAX_PARTS] {
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

    /// The next hour's split from this hour's: `ticks` of census and plan
    /// walk over all parts at the session's QPC `frequency` (ticks per
    /// second), and the stitched census's handle-sorted `roots` and
    /// key-sorted `rows`.
    pub(crate) fn plan(ticks: u64, frequency: u64, roots: &[Root], rows: &[Row]) -> Self {
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
        // Each group's cost: its rosters and its orders.
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
            // The next part starts at the first group past its share. With
            // fewer groups than parts the last parts start past every handle
            // and stay empty.
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
                // The pointer is the engine's own; a part needs only the handle.
                let mut root = Root::default();
                root.handle = (id << 32) | 53;
                root
            })
            .collect()
    }

    /// The part count follows last hour's measured time, whatever the QPC
    /// frequency, bounded by the reserved slots, and the cuts divide last
    /// hour's per-group work.
    #[test]
    fn the_split_follows_measured_time_and_divides_per_group_work() {
        let groups = roots(8);
        // Two orders in each of the first four groups, none after.
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
        // The same elapsed times at 10 MHz and at 1 MHz: half the target,
        // the target and a microsecond more, twice it, and a hundred times.
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
        // Weights 44 for each of the first four groups and 10 for the rest:
        // half of 216 is reached before the fourth group.
        let split = Split::plan(2 * target * 10, frequency, &groups, &rows);
        let ranges = split.ranges(&groups);
        assert_eq!(ranges[0], 0..3);
        assert_eq!(ranges[1], 3..8);
        assert!(ranges[2].is_empty() && ranges[3].is_empty());
        // Next hour's groups: one removed, one added; the ranges stay
        // consecutive and cover them.
        let mut next = roots(9);
        next.remove(1);
        let ranges = split.ranges(&next);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, ranges[1].start);
        assert_eq!(ranges[1].end, next.len());
        // One part covers everything.
        assert_eq!(Split::default().ranges(&next)[0], 0..next.len());
    }
}
