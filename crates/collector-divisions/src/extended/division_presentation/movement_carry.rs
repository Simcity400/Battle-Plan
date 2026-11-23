use engine_native_memory::Memory;
use engine_native::{carmy::DivisionSeed, cprovince};
use engine_naval_history::cnaval_unit_transfer::{
    SpeedCache, SpeedInputs, TransportRegion, movement_speeds_inputs,
};
use std::collections::HashMap;

const ALLOCATION: &str = "division transport observation allocation failed";
const QUERY_MISSING: &str = "division transport query missing";

#[derive(Default)]
pub(super) struct Transport {
    regions: HashMap<usize, TransportRegion>,
    pending: Vec<(usize, Result<(u32, TransportRegion), &'static str>)>,
    wanted_regions: Vec<TransportRegion>,
    queries: Vec<(u32, usize)>,
    results: Vec<Result<SpeedInputs, &'static str>>,
    wanted: Vec<usize>,
    cache: SpeedCache,
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

    pub(super) fn clear(&mut self) {
        self.regions.clear();
        self.pending.clear();
        self.wanted_regions.clear();
        self.queries.clear();
        self.results.clear();
        self.wanted.clear();
    }

    pub(super) fn begin(&mut self, divisions: usize) -> Result<(), &'static str> {
        self.clear();
        reserve(&mut self.pending, divisions)
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
                .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                .ok_or("division transport region pointer unavailable")?
                as usize;
            if let Some(&region) = self.regions.get(&pointer) {
                return Ok((owner, region));
            }
            let id = engine_native_parts::cstrategic_region::id(memory, base, pointer)?
                .ok_or("division transport source region absent")?;
            let region = TransportRegion { id, pointer };
            if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.regions, 1))
                .is_ok()
            {
                self.regions.insert(pointer, region);
            }
            Ok((owner, region))
        })();
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((index, region));
        }
    }

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
            query(
                owner,
                &self.wanted_regions,
                &self.wanted,
                &mut self.cache,
                &mut |_, inputs| results.push(inputs),
            );
            start = end;
        }
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
