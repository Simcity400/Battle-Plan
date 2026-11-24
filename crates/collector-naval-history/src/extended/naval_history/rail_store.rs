use collector_observers::rail_changes::{COUNTDOWN, Changes, Counts};
use collector_projection::projection::{Witness, generation};
use engine_allocation as allocation;
use engine_railways::{
    cprovince_railway_info as info, crailway_manager::HeldBatch, crailway_manager::InfoOutputs,
    crailway_manager::PROVINCE_BATCH, crailway_manager::RailCarry, crailway_manager::RailHeader,
    crailway_manager::RailLocation, crailway_manager::fresh_inputs::RailwayBuildingRow,
    rail_neighbor,
};
use std::sync::Arc;

pub struct CleanRails {
    pub(crate) witness: Witness,
    pub(crate) cooldowns: Vec<info::Row>,
    pub(crate) edges: Vec<rail_neighbor::Row>,
    pub(crate) buildings: Vec<RailwayBuildingRow>,
    pub(crate) levels: Vec<i32>,
    pub(crate) buildings_required: usize,
    pub(crate) levels_required: usize,
}

impl CleanRails {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.cooldowns.capacity() * size_of::<info::Row>()
            + self.edges.capacity() * size_of::<rail_neighbor::Row>()
            + self.buildings.capacity() * size_of::<RailwayBuildingRow>()
            + self.levels.capacity() * size_of::<i32>()
    }
}

const BATCH: u32 = PROVINCE_BATCH as u32;

fn rebased(mut row: RailwayBuildingRow, first: usize) -> RailwayBuildingRow {
    if let Some(selection) = row.selection.as_mut()
        && let Ok(span) = selection.levels.as_mut()
    {
        span.first = first;
    }
    row
}

#[derive(Default)]
struct Batch {
    whole: bool,
    unplanned: bool,
    locations: Vec<RailLocation>,
    cooldowns: Vec<info::Row>,
    edge_ends: Vec<usize>,
    edges: Vec<rail_neighbor::Row>,
    buildings: Vec<RailwayBuildingRow>,
    levels: Vec<i32>,
}

impl Batch {
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

#[derive(Clone, Copy)]
struct Plan {
    location: RailLocation,
    held: Option<usize>,
}

#[derive(Default)]
pub struct RailStore {
    epoch: Option<u64>,
    header: Option<RailHeader>,
    list: Vec<u32>,
    listed: bool,
    batches: Vec<Batch>,
    clean: Option<Arc<CleanRails>>,

    take: Option<u64>,
    tables: Option<RailHeader>,
    list_now: Vec<u32>,
    listed_now: bool,
    marked: Vec<u32>,
    observing: bool,
    lost: bool,
    batch: usize,
    scratch: Batch,
    plans: Vec<Plan>,
    counts: Counts,

    bytes: usize,
    forgotten: bool,
}

impl RailStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    fn forget_held(&mut self) {
        self.epoch = None;
        self.header = None;
        self.list.clear();
        self.listed = false;
        self.clean = None;
        for batch in &mut self.batches {
            batch.forget();
        }
    }

    fn measure(&self) -> usize {
        self.batches.iter().map(Batch::measure).sum::<usize>()
            + self.batches.capacity() * size_of::<Batch>()
            + self.clean.as_ref().map_or(0, |clean| clean.allocated_bytes())
            + self.scratch.measure()
            + (self.list.capacity() + self.list_now.capacity() + self.marked.capacity())
                * size_of::<u32>()
            + self.plans.capacity() * size_of::<Plan>()
    }

    fn marked(&self, province: u32) -> bool {
        self.marked.binary_search(&province).is_ok()
    }

    fn unmarked(&mut self) {
        self.observing = false;
    }
}

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

pub struct RailReuse<'a> {
    store: &'a mut RailStore,
}

impl<'a> RailReuse<'a> {
    pub fn begin(
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

    pub fn clean_hit(&mut self, header: RailHeader) -> Option<Arc<CleanRails>> {
        let store = &mut *self.store;
        if !store.observing || !store.marked.is_empty() || store.header != Some(header) {
            store.clean = None;
            return None;
        }
        let batches = (header.rail.count as usize).div_ceil(PROVINCE_BATCH);
        if store.batches.len() < batches || !store.batches[..batches].iter().all(|b| b.whole) {
            store.clean = None;
            return None;
        }
        let clean = store.clean.as_ref()?.clone();
        store.tables = Some(header);
        store.counts.carried_batches = batches as u64;
        store.counts.reused = clean.cooldowns.len() as u64;
        store.counts.reused_edges = clean.edges.len() as u64;
        Some(clean)
    }

    pub fn finish(self, complete: bool) {
        self.finish_with_clean(complete, |_| None);
    }

    pub fn finish_with_clean(
        self,
        complete: bool,
        build: impl FnOnce(Witness) -> Option<CleanRails>,
    ) -> Option<Witness> {
        let store = self.store;
        let slots = store.tables.map(|header| header.rail.count as usize);
        let walked = complete && slots.is_some();
        let held = walked && !store.lost && store.take.is_some();
        let mut witness = None;
        if held {
            store.epoch = store.take;
            store.header = store.tables;
            std::mem::swap(&mut store.list, &mut store.list_now);
            store.listed = store.listed_now;
            let batches = slots.unwrap_or(0).div_ceil(PROVINCE_BATCH);
            store.batches.truncate(batches);
            if let Some(clean) = &store.clean {
                witness = Some(clean.witness);
            } else if store.batches.len() == batches && store.batches.iter().all(|b| b.whole) {
                let candidate = Witness::generation(generation());
                if let Some(clean) = build(candidate) {
                    witness = Some(clean.witness);
                    store.clean = Some(Arc::new(clean));
                }
            }
        } else {
            store.forget_held();
        }
        store.counts.full = store.counts.carried_batches == 0 && store.counts.reused == 0;
        store.plans.clear();
        store.scratch.reset();
        store.bytes = store.measure();
        collector_observers::rail_changes::record(&store.counts, slots.filter(|_| walked));
        witness
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

    fn plan(&mut self, location: &RailLocation) -> (usize, bool) {
        let store = &mut *self.store;
        let token = store.plans.len();
        if token == store.plans.capacity() {
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
