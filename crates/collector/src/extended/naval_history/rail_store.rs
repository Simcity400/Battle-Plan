//! RailCooldowns rows with their building and selection inputs, indexed
//! levels and RailEdges rows held between hours per batch of
//! [`PROVINCE_BATCH`] rail slots ([`crate::rail_changes`]).
//!
//! Each hour first takes the rail domain's marked provinces
//! ([`crate::dirty_set::DirtySet::take`]) and reads the manager's cooldown
//! list. A batch is published from the held outputs of its last read,
//! without reading any of its slots, provinces, infos or buildings, only
//! when all of these hold: the take returned the epoch of the held complete
//! walk; no marked province lies in the batch; the manager, rail table and
//! province table headers are those of the held walk; and the batch's last
//! read held every info it found. The daily countdown marks [`COUNTDOWN`]:
//! every province on the held list and on this hour's list (duplicates
//! included) is then marked, since the countdown writes the info of each
//! occurrence. Outside
//! a countdown the two lists may differ only in marked provinces; any other
//! difference is an unobserved write.
//!
//! Every other batch is read: each info's identity, cooldown and pair
//! header are gathered, and an unmarked info whose gathered location is
//! that of its held read is carried without reading its pairs, levels or
//! building. An unmarked info that changed, or that a whole held batch had
//! and this read did not find, is an unobserved write: the rest of the walk
//! reads afresh. A read batch replaces its held outputs.
//!
//! The store follows the Late job's loan. A walk that does not complete, a
//! refused take or a failed hold keeps nothing for the next hour, which
//! then reads every batch. Allocation is advisory: an info whose outputs
//! cannot be held leaves its batch to be read next hour.
use crate::rail_changes::{COUNTDOWN, Changes, Counts};
use engine_allocation as allocation;
use engine_railways::{
    cprovince_railway_info as info, crailway_manager::HeldBatch, crailway_manager::InfoOutputs,
    crailway_manager::PROVINCE_BATCH, crailway_manager::RailCarry, crailway_manager::RailHeader,
    crailway_manager::RailLocation, crailway_manager::fresh_inputs::RailwayBuildingRow,
    rail_neighbor,
};

const BATCH: u32 = PROVINCE_BATCH as u32;

/// `row` with its selected level span starting at `first`.
fn rebased(mut row: RailwayBuildingRow, first: usize) -> RailwayBuildingRow {
    if let Some(selection) = row.selection.as_mut()
        && let Ok(span) = selection.levels.as_mut()
    {
        span.first = first;
    }
    row
}

/// One batch's held outputs, in slot order.
#[derive(Default)]
struct Batch {
    /// The batch's last read held every info it found.
    whole: bool,
    /// Some info of the batch being read was not planned.
    unplanned: bool,
    locations: Vec<RailLocation>,
    cooldowns: Vec<info::Row>,
    /// Each info's end in `edges`.
    edge_ends: Vec<usize>,
    edges: Vec<rail_neighbor::Row>,
    /// Each selected level span starts within `levels`.
    buildings: Vec<RailwayBuildingRow>,
    levels: Vec<i32>,
}

impl Batch {
    /// Empty and whole, ready to hold a read.
    fn reset(&mut self) {
        self.whole = true;
        self.unplanned = false;
        self.locations.clear();
        self.cooldowns.clear();
        self.edge_ends.clear();
        self.edges.clear();
        self.buildings.clear();
        self.levels.clear();
    }

    /// Holds nothing that certifies anything.
    fn forget(&mut self) {
        self.reset();
        self.whole = false;
    }

    fn find(&self, province: u32) -> Option<usize> {
        self.locations
            .binary_search_by_key(&province, |location| location.province)
            .ok()
    }

    fn outputs(&self, index: usize) -> InfoOutputs<'_> {
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.edge_ends[previous]);
        let building = self.buildings[index];
        let levels = building
            .selection
            .and_then(|selection| selection.levels.ok())
            .and_then(|span| {
                self.levels
                    .get(span.first..span.first.checked_add(span.len)?)
            })
            .unwrap_or(&[]);
        InfoOutputs {
            cooldown: self.cooldowns[index].cooldown_ticks,
            edges: &self.edges[start..self.edge_ends[index]],
            building: rebased(building, 0),
            levels,
        }
    }

    /// Holds one info's outputs after those already held; `false` when the
    /// storage could not grow or the info does not follow them in slot
    /// order, which only withholds the batch's certificate.
    fn hold(&mut self, location: RailLocation, outputs: &InfoOutputs<'_>) -> bool {
        if self
            .locations
            .last()
            .is_some_and(|last| last.province >= location.province)
        {
            return false;
        }
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.locations, 1)?;
            allocation::reserve(&mut self.cooldowns, 1)?;
            allocation::reserve(&mut self.edge_ends, 1)?;
            allocation::reserve(&mut self.buildings, 1)?;
            allocation::reserve(&mut self.edges, outputs.edges.len())?;
            allocation::reserve(&mut self.levels, outputs.levels.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let first = self.levels.len();
        self.edges.extend_from_slice(outputs.edges);
        self.levels.extend_from_slice(outputs.levels);
        self.locations.push(location);
        self.cooldowns.push(info::Row {
            province: location.province,
            cooldown_ticks: outputs.cooldown,
            building_bound: info::building_bound(&location.identity),
        });
        self.edge_ends.push(self.edges.len());
        self.buildings.push(rebased(outputs.building, first));
        true
    }

    fn view(&self) -> HeldBatch<'_> {
        HeldBatch {
            cooldowns: &self.cooldowns,
            edges: &self.edges,
            buildings: &self.buildings,
            levels: &self.levels,
        }
    }

    fn measure(&self) -> usize {
        self.locations.capacity() * size_of::<RailLocation>()
            + self.cooldowns.capacity() * size_of::<info::Row>()
            + self.edge_ends.capacity() * size_of::<usize>()
            + self.edges.capacity() * size_of::<rail_neighbor::Row>()
            + self.buildings.capacity() * size_of::<RailwayBuildingRow>()
            + self.levels.capacity() * size_of::<i32>()
    }
}

/// One planned info of the batch being read: its location and, when it may
/// be carried, its index in the held batch.
#[derive(Clone, Copy)]
struct Plan {
    location: RailLocation,
    held: Option<usize>,
}

/// Outputs of the latest complete walk per batch, and the walk being made.
#[derive(Default)]
pub(crate) struct RailStore {
    /// The take epoch of the held walk; `None` holds nothing.
    epoch: Option<u64>,
    header: Option<RailHeader>,
    /// The cooldown list read with the held walk, sorted.
    list: Vec<u32>,
    listed: bool,
    /// Indexed by batch.
    batches: Vec<Batch>,

    // The walk being made.
    take: Option<u64>,
    tables: Option<RailHeader>,
    /// This hour's cooldown list, sorted.
    list_now: Vec<u32>,
    listed_now: bool,
    /// Provinces marked this hour, sorted.
    marked: Vec<u32>,
    /// Held batches may be carried.
    observing: bool,
    /// A read batch could not be held: keep nothing.
    lost: bool,
    /// The batch being read and its planned infos.
    batch: usize,
    scratch: Batch,
    plans: Vec<Plan>,
    counts: Counts,

    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held output.
    forgotten: bool,
}

impl RailStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next walk reads afresh.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }

    fn forget_held(&mut self) {
        self.epoch = None;
        self.header = None;
        self.list.clear();
        self.listed = false;
        for batch in &mut self.batches {
            batch.forget();
        }
    }

    fn measure(&self) -> usize {
        self.batches.iter().map(Batch::measure).sum::<usize>()
            + self.batches.capacity() * size_of::<Batch>()
            + self.scratch.measure()
            + (self.list.capacity() + self.list_now.capacity() + self.marked.capacity())
                * size_of::<u32>()
            + self.plans.capacity() * size_of::<Plan>()
    }

    fn marked(&self, province: u32) -> bool {
        self.marked.binary_search(&province).is_ok()
    }

    /// An unmarked info or list changed: a write no hook saw.
    fn unmarked(&mut self) {
        self.observing = false;
    }
}

/// Whether `held` and `now` (sorted, duplicates kept) list each province
/// equally often, but for marked ones.
fn lists_agree(held: &[u32], now: &[u32], marked: &[u32]) -> bool {
    let (mut held, mut now) = (held, now);
    loop {
        let province = match (held.first(), now.first()) {
            (None, None) => return true,
            (Some(&a), Some(&b)) => a.min(b),
            (Some(&a), None) => a,
            (None, Some(&b)) => b,
        };
        let split = |list: &[u32]| list.partition_point(|&id| id <= province);
        let (a, b) = (split(held), split(now));
        if a != b && marked.binary_search(&province).is_err() {
            return false;
        }
        held = &held[a..];
        now = &now[b..];
    }
}

/// One walk's view of the store and the observation that certifies reuse.
pub(crate) struct RailReuse<'a> {
    store: &'a mut RailStore,
}

impl<'a> RailReuse<'a> {
    /// Takes the hour's marks (the domain's single consumer) and reads the
    /// manager's cooldown list with `read_list`, before the walk reads any
    /// slot.
    pub(crate) fn begin(
        store: &'a mut RailStore,
        changes: &dyn Changes,
        read_list: impl FnOnce(&mut Vec<u32>) -> Result<(), &'static str>,
    ) -> Self {
        if std::mem::take(&mut store.forgotten) {
            store.forget_held();
        }
        store.counts = Counts::default();
        store.tables = None;
        store.lost = false;
        store.batch = 0;
        store.scratch.reset();
        store.plans.clear();
        // Room for one batch's plans, so planning never allocates.
        let _ = allocation::advisory(|| allocation::reserve(&mut store.plans, PROVINCE_BATCH));
        store.marked.clear();
        let (mut countdown, mut lost) = (false, false);
        let marked = &mut store.marked;
        let take = changes.take(&mut |id| {
            if id == COUNTDOWN {
                countdown = true;
                return;
            }
            let Ok(id) = u32::try_from(id) else {
                lost = true;
                return;
            };
            if allocation::advisory(|| allocation::reserve(marked, 1)).is_ok() {
                marked.push(id);
            } else {
                lost = true;
            }
        });
        store.take = take;
        store.listed_now = read_list(&mut store.list_now).is_ok();
        store.list_now.sort_unstable();
        store.marked.sort_unstable();
        store.marked.dedup();
        store.counts.countdown = countdown;
        store.observing = store.take.is_some()
            && store.take == store.epoch
            && store.listed
            && store.listed_now
            && !lost;
        if store.observing && countdown {
            // The countdown wrote the info of every occurrence on the list
            // it walked, which is the held list with this hour's marked
            // pushes; those it kept are on this hour's list. A negative id
            // names no slot the walk can bound.
            let RailStore {
                list,
                list_now,
                marked,
                ..
            } = &mut *store;
            let named = list
                .iter()
                .chain(list_now.iter())
                .all(|&id| id <= i32::MAX as u32);
            let reserved =
                allocation::advisory(|| allocation::reserve(marked, list.len() + list_now.len()));
            if named && reserved.is_ok() {
                marked.extend(list.iter().chain(list_now.iter()));
                marked.sort_unstable();
                marked.dedup();
            } else {
                store.observing = false;
            }
        } else if store.observing && !lists_agree(&store.list, &store.list_now, &store.marked) {
            store.unmarked();
        }
        store.counts.marked = store.marked.len() as u64;
        Self { store }
    }

    /// Ends the walk: `complete` when the cooldown, edge and building tables
    /// were each read or carried whole.
    pub(crate) fn finish(self, complete: bool) {
        let store = self.store;
        let slots = store.tables.map(|header| header.rail.count as usize);
        // A completed walk sizes the bitmap even when nothing certifies it:
        // the first bitmap is offered only once a walk has counted the slots.
        let walked = complete && slots.is_some();
        let held = walked && !store.lost && store.take.is_some();
        if held {
            store.epoch = store.take;
            store.header = store.tables;
            std::mem::swap(&mut store.list, &mut store.list_now);
            store.listed = store.listed_now;
            let batches = slots.unwrap_or(0).div_ceil(PROVINCE_BATCH);
            store.batches.truncate(batches);
        } else {
            store.forget_held();
        }
        store.counts.full = store.counts.carried_batches == 0 && store.counts.reused == 0;
        store.plans.clear();
        store.scratch.reset();
        store.bytes = store.measure();
        crate::rail_changes::record(&store.counts, slots.filter(|_| walked));
    }
}

impl RailCarry for RailReuse<'_> {
    fn tables(&mut self, header: &RailHeader) {
        let store = &mut *self.store;
        store.tables = Some(*header);
        if store.header != Some(*header) {
            store.observing = false;
        }
        let batches = (header.rail.count as usize).div_ceil(PROVINCE_BATCH);
        if store.batches.len() < batches {
            let missing = batches - store.batches.len();
            let grown = allocation::advisory(|| allocation::reserve(&mut store.batches, missing));
            match grown {
                Ok(()) => store.batches.resize_with(batches, Batch::default),
                Err(_) => store.lost = true,
            }
        }
    }

    fn quiet_batch(&mut self, first: u32, count: u32) -> Option<HeldBatch<'_>> {
        let store = &mut *self.store;
        store.batch = (first / BATCH) as usize;
        store.plans.clear();
        store.scratch.reset();
        if !store.observing {
            return None;
        }
        let end = first.saturating_add(count);
        let next = store.marked.partition_point(|&province| province < first);
        if store
            .marked
            .get(next)
            .is_some_and(|&province| province < end)
        {
            return None;
        }
        let batch = store.batches.get(store.batch).filter(|batch| batch.whole)?;
        Some(batch.view())
    }

    fn carried_batch(&mut self) {
        let store = &mut *self.store;
        if let Some(batch) = store.batches.get(store.batch) {
            store.counts.reused += batch.cooldowns.len() as u64;
            store.counts.reused_edges += batch.edges.len() as u64;
        }
        store.counts.carried_batches += 1;
    }

    /// Plans an info of a batch being read, after its identity, cooldown and
    /// pair header were gathered and before any of its outputs are read.
    fn plan(&mut self, location: &RailLocation) -> (usize, bool) {
        let store = &mut *self.store;
        let token = store.plans.len();
        if token == store.plans.capacity() {
            // Unplanned: read afresh and held for nothing.
            store.scratch.whole = false;
            store.scratch.unplanned = true;
            return (usize::MAX, false);
        }
        let mut held = None;
        if store.observing && !store.marked(location.province) {
            let found = store
                .batches
                .get(store.batch)
                .map(|batch| (batch.find(location.province), batch.whole));
            match found {
                Some((Some(index), _))
                    if store.batches[store.batch].locations[index] == *location =>
                {
                    held = Some(index)
                }
                // Changed, or new where the held read found every info.
                Some((Some(_), _)) | Some((None, true)) => store.unmarked(),
                _ => {}
            }
        }
        store.plans.push(Plan {
            location: *location,
            held,
        });
        (token, held.is_some())
    }

    fn held(&self, token: usize) -> Option<InfoOutputs<'_>> {
        let index = self.store.plans.get(token)?.held?;
        Some(self.store.batches.get(self.store.batch)?.outputs(index))
    }

    fn carried(&mut self, token: usize) {
        let store = &mut *self.store;
        let (
            Some(Plan {
                location,
                held: Some(index),
            }),
            Some(batch),
        ) = (
            store.plans.get(token).copied(),
            store.batches.get(store.batch),
        )
        else {
            store.scratch.whole = false;
            return;
        };
        let outputs = batch.outputs(index);
        store.counts.reused += 1;
        store.counts.reused_edges += outputs.edges.len() as u64;
        if !store.scratch.hold(location, &outputs) {
            store.scratch.whole = false;
        }
    }

    fn read(&mut self, token: usize, outputs: Option<InfoOutputs<'_>>) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_edges += outputs.map_or(0, |outputs| outputs.edges.len() as u64);
        let (Some(plan), Some(outputs)) = (store.plans.get(token).copied(), outputs) else {
            store.scratch.whole = false;
            return;
        };
        if !store.scratch.hold(plan.location, &outputs) {
            store.scratch.whole = false;
        }
    }

    fn read_batch(&mut self) {
        let store = &mut *self.store;
        store.counts.read_batches += 1;
        // An unmarked info a whole held batch had and this read did not
        // find: a destruction no hook saw.
        let mut vanished = false;
        if store.observing
            && !store.scratch.unplanned
            && let Some(batch) = store.batches.get(store.batch).filter(|batch| batch.whole)
        {
            let mut planned = store
                .plans
                .iter()
                .map(|plan| plan.location.province)
                .peekable();
            for location in &batch.locations {
                while planned
                    .next_if(|&province| province < location.province)
                    .is_some()
                {}
                if planned.peek() != Some(&location.province)
                    && store.marked.binary_search(&location.province).is_err()
                {
                    vanished = true;
                    break;
                }
            }
        }
        if vanished {
            store.unmarked();
        }
        match store.batches.get_mut(store.batch) {
            Some(batch) => std::mem::swap(batch, &mut store.scratch),
            None => store.lost = true,
        }
        store.scratch.reset();
        store.plans.clear();
    }
}
