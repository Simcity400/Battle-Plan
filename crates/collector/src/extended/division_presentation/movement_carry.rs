//! Transfer transport speeds, one batched query per owner; each region is authenticated once an hour.
use engine_native::{Memory, carmy::DivisionSeed, cprovince};
use engine_naval_history::cnaval_unit_transfer::{
    SpeedCache, SpeedInputs, TransportRegion, movement_speeds_inputs,
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

const ALLOCATION: &str = "division transport observation allocation failed";
const QUERY_MISSING: &str = "division transport query missing";

const COUNTS: usize = 11;
const COUNT_NAMES: [&str; COUNTS] = [
    "hours",
    "divisions",
    "stationary_divisions",
    "province_ids_carried",
    "province_ids_read",
    "transfer_divisions",
    "transfer_regions_carried",
    "transfer_regions_read",
    "transport_owners",
    "transport_queries",
    "land_divisions_read",
];
static TOTALS: [AtomicU64; COUNTS] = [const { AtomicU64::new(0) }; COUNTS];

/// One pass's counts, in [`COUNT_NAMES`] order.
pub(super) type Counts = [u64; COUNTS];

pub(super) fn record(counts: &Counts) {
    for (total, count) in TOTALS.iter().zip(counts) {
        total.fetch_add(*count, Ordering::Relaxed);
    }
}

pub(super) fn status_data() -> impl serde::Serialize {
    let [carried, read, uncertified] = collector_division_parts::dig_in::carry_counts();
    crate::status::status!({
        "reads": crate::status::map(
            COUNT_NAMES
                .into_iter()
                .zip(TOTALS.each_ref().map(|total| total.load(Ordering::Relaxed)))
        ),
        "dig_in_blocks": {
            "carried": carried,
            "read": read,
            "uncertified": uncertified,
        },
        "scope": "cumulative since attach, per hourly movement pass: its census divisions and the stationary ones (no route, combat or transfer), whose movement rows read nothing once their source and target province ids are held; province ids answered from the ids held for the current world or read and authenticated afresh; transfer divisions, whose location's strategic region is read every hour (the map editor rebinds provinces) and authenticated once an hour per region (transfer_regions_carried: answered by this hour's earlier authentication), and the batched transport speed queries (owners, and owner and region pairs); land inputs read for every division; dig-in commander blocks carried under the commander's retreat-ban certificate, read under one, or read without one",
    })
}

#[derive(Default)]
pub(super) struct Transport {
    /// This hour's authenticated regions, by pointer.
    regions: HashMap<usize, TransportRegion>,
    /// This hour's transfer divisions by row index: owner and region, or the refusal.
    pending: Vec<(usize, Result<(u32, TransportRegion), &'static str>)>,
    /// Distinct by id.
    wanted_regions: Vec<TransportRegion>,
    /// Distinct (owner, region index) pairs, ascending; `results` follows them.
    queries: Vec<(u32, usize)>,
    results: Vec<Result<SpeedInputs, &'static str>>,
    wanted: Vec<usize>,
    cache: SpeedCache,
    /// Regions carried and read, owners and queries this hour.
    counts: [u64; 4],
}

fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), &'static str> {
    engine_allocation::reserve(values, additional).map_err(|_| ALLOCATION)
}

impl Transport {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.regions.capacity() * (size_of::<(usize, TransportRegion)>() + 1)
            + self.pending.capacity()
                * size_of::<(usize, Result<(u32, TransportRegion), &'static str>)>()
            + self.wanted_regions.capacity() * size_of::<TransportRegion>()
            + self.queries.capacity() * size_of::<(u32, usize)>()
            + self.results.capacity() * size_of::<Result<SpeedInputs, &'static str>>()
            + self.wanted.capacity() * size_of::<usize>()
            + self.cache.allocated_bytes()
    }

    /// Forget the hour.
    pub(super) fn clear(&mut self) {
        self.regions.clear();
        self.pending.clear();
        self.wanted_regions.clear();
        self.queries.clear();
        self.results.clear();
        self.wanted.clear();
        self.counts = [0; 4];
    }

    pub(super) fn begin(&mut self, divisions: usize) -> Result<(), &'static str> {
        self.clear();
        reserve(&mut self.pending, divisions)
    }

    pub(super) fn counts(&self) -> [u64; 4] {
        self.counts
    }

    pub(super) fn push(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
        seed: &DivisionSeed,
    ) {
        let region = (|| -> Result<(u32, TransportRegion), &'static str> {
            let owner = seed
                .logical_owner
                .ok_or("division transport owner unavailable")?;
            let location = seed
                .location
                .ok_or("division transport location unavailable")?;
            let pointer = location
                .checked_add(cprovince::STRATEGIC_REGION)
                .and_then(|at| engine_native::read::<u64>(memory, at))
                .ok_or("division transport region pointer unavailable")?
                as usize;
            if let Some(&region) = self.regions.get(&pointer) {
                self.counts[0] += 1;
                return Ok((owner, region));
            }
            let id = engine_native::cstrategic_region::id(memory, base, pointer)?
                .ok_or("division transport source region absent")?;
            let region = TransportRegion { id, pointer };
            self.counts[1] += 1;
            if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.regions, 1))
                .is_ok()
            {
                self.regions.insert(pointer, region);
            }
            Ok((owner, region))
        })();
        // Room for every census row was reserved in begin.
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((index, region));
        }
    }

    /// Emits every pushed division's inputs or refusal, in push order.
    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        emit: impl FnMut(usize, Result<SpeedInputs, &'static str>),
    ) {
        self.acquire_with(
            |owner, regions, wanted, cache, emit| {
                movement_speeds_inputs(memory, base, state, owner, regions, wanted, cache, emit)
            },
            emit,
        );
    }

    /// [`Self::acquire`] with `query` answering one owner's wanted regions.
    fn acquire_with(
        &mut self,
        mut query: impl FnMut(
            u32,
            &[TransportRegion],
            &[usize],
            &mut SpeedCache,
            &mut dyn FnMut(TransportRegion, Result<SpeedInputs, &'static str>),
        ),
        mut emit: impl FnMut(usize, Result<SpeedInputs, &'static str>),
    ) {
        self.wanted_regions.clear();
        self.queries.clear();
        self.results.clear();
        let prepared = self.prepare();
        if let Err(reason) = prepared {
            for &(index, entry) in &self.pending {
                emit(index, entry.and(Err(reason)));
            }
            return;
        }
        let mut start = 0;
        while start < self.queries.len() {
            let owner = self.queries[start].0;
            let end = start
                + self.queries[start..]
                    .iter()
                    .take_while(|query| query.0 == owner)
                    .count();
            self.wanted.clear();
            self.wanted
                .extend(self.queries[start..end].iter().map(|&(_, at)| at));
            let results = &mut self.results;
            // Each wanted region is emitted once, in order.
            query(
                owner,
                &self.wanted_regions,
                &self.wanted,
                &mut self.cache,
                &mut |_, inputs| results.push(inputs),
            );
            self.counts[2] += 1;
            start = end;
        }
        self.counts[3] = self.queries.len() as u64;
        self.cache.finish();
        for &(index, entry) in &self.pending {
            emit(
                index,
                entry.and_then(|(owner, region)| self.result(owner, region)),
            );
        }
    }

    fn prepare(&mut self) -> Result<(), &'static str> {
        reserve(&mut self.wanted_regions, self.pending.len())?;
        reserve(&mut self.queries, self.pending.len())?;
        for (_, entry) in &self.pending {
            if let Ok((_, region)) = entry {
                self.wanted_regions.push(*region);
            }
        }
        self.wanted_regions.sort_unstable_by_key(|region| region.id);
        self.wanted_regions.dedup_by_key(|region| region.id);
        for (_, entry) in &self.pending {
            if let Ok((owner, region)) = entry {
                let at = self
                    .wanted_regions
                    .binary_search_by_key(&region.id, |wanted| wanted.id)
                    .map_err(|_| QUERY_MISSING)?;
                self.queries.push((*owner, at));
            }
        }
        self.queries.sort_unstable();
        self.queries.dedup();
        reserve(&mut self.results, self.queries.len())?;
        reserve(&mut self.wanted, self.queries.len())?;
        self.cache.begin(self.wanted_regions.len())
    }

    fn result(&self, owner: u32, region: TransportRegion) -> Result<SpeedInputs, &'static str> {
        let at = self
            .wanted_regions
            .binary_search_by_key(&region.id, |wanted| wanted.id)
            .map_err(|_| QUERY_MISSING)?;
        let query = self
            .queries
            .binary_search(&(owner, at))
            .map_err(|_| QUERY_MISSING)?;
        *self.results.get(query).ok_or(QUERY_MISSING)?
    }
}

#[cfg(test)]
mod tests;
