//! Prepared storage for the existing hourly air-base carrier suffixes.
//! Source addresses are used only against the owned raw log on decode.

use super::*;
use engine::cair_base::{
    CarrierAceSlot, CarrierBase, CarrierBuffers, CarrierCountryTerms, CarrierDemand,
    CarrierForecastSlot, CarrierIndexedCertificate, CarrierLengths, CarrierPool,
    CarrierRegistration, CountryOccurrence,
};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

const SEED_REFUSALS: [&str; 9] = [
    "carrier exact index capacity",
    "carrier exact index invalid",
    "carrier original census overflowed",
    "carrier census byte range invalid",
    "carrier census repeated failed observation",
    "carrier census repeated observation changed",
    "carrier census seed capacity",
    "carrier census source span invalid",
    "carrier census unclassified seed refusal",
];
static SEED_REFUSAL_COUNTS: [AtomicU64; SEED_REFUSALS.len()] =
    [const { AtomicU64::new(0) }; SEED_REFUSALS.len()];

/// Acquisition counts once; owned decode retains the verdict without counting
/// again. A future seed refusal still has a counter until given its own slot.
pub(super) fn note_seed_refusal(reason: &'static str) {
    let index = SEED_REFUSALS
        .iter()
        .position(|candidate| *candidate == reason)
        .unwrap_or(SEED_REFUSALS.len() - 1);
    SEED_REFUSAL_COUNTS[index].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn capture_status() -> impl serde::Serialize {
    std::array::from_fn::<_, { SEED_REFUSALS.len() }, _>(|index| {
        (
            SEED_REFUSALS[index],
            SEED_REFUSAL_COUNTS[index].load(Ordering::Relaxed),
        )
    })
}

pub(crate) struct Bases {
    pub rows: Vec<CarrierBase>,
    pub countries: Vec<CountryOccurrence>,
    pub names: Vec<u8>,
    pub len: usize,
    pub countries_len: usize,
    pub names_len: usize,
    indexed: bool,
    demand: CarrierDemand,
}

impl Bases {
    pub fn new(rows: usize) -> Self {
        Self {
            rows: allocation::owned_vec(allocation::default_vec(rows)),
            countries: Vec::new(),
            names: Vec::new(),
            len: 0,
            countries_len: 0,
            names_len: 0,
            indexed: true,
            demand: CarrierDemand::default(),
        }
    }
    pub fn reset(&mut self) {
        self.len = 0;
        self.countries_len = 0;
        self.names_len = 0;
        self.demand = CarrierDemand::default();
        self.indexed = true;
    }
    pub fn install(&mut self, lengths: CarrierLengths, demand: CarrierDemand) {
        // An inconsistent observer length cannot certify an available prefix.
        // This rejects only optional packets; the predecessor rows stay intact.
        if lengths.bases > self.rows.len()
            || lengths.countries > self.countries.len()
            || lengths.names > self.names.len()
        {
            self.reset();
            self.demand = demand;
            return;
        }
        self.len = lengths.bases;
        self.countries_len = lengths.countries;
        self.names_len = lengths.names;
        self.demand = demand;
        self.indexed = false;
    }
    pub fn prepare(&mut self, demand: CarrierDemand) {
        prepare(&mut self.rows, demand.bases);
        prepare(&mut self.countries, demand.countries);
        prepare(&mut self.names, demand.name_bytes);
    }
    pub fn grow(&mut self) {
        self.prepare(self.demand);
    }
    pub fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<CarrierBase>()
            + self.countries.capacity() * size_of::<CountryOccurrence>()
            + self.names.capacity()
    }
    pub fn capacity(&self) -> [usize; 3] {
        [self.rows.len(), self.countries.len(), self.names.len()]
    }
    /// Worker-only outer-key lookup preparation. Ordered payload arenas and
    /// the mandatory AirBases traversal are never reordered.
    pub fn index_rows(&mut self) {
        self.rows[..self.len].sort_unstable_by_key(|row| row.base);
        self.indexed = true;
    }
    fn row(&self, base: u64) -> Option<&CarrierBase> {
        if !self.indexed {
            return None;
        }
        let rows = self.rows.get(..self.len)?;
        let index = rows.binary_search_by_key(&base, |row| row.base).ok()?;
        if index
            .checked_sub(1)
            .and_then(|index| rows.get(index))
            .is_some_and(|row| row.base == base)
            || rows.get(index + 1).is_some_and(|row| row.base == base)
        {
            return None;
        }
        rows.get(index)
    }
    pub fn packet(&self, base: u64) -> rows::Recorded<Option<rows::AirBaseCarrierInputs>> {
        rows::Recorded::Value(self.row(base).map(|row| rows::AirBaseCarrierInputs {
            registered_countries: row.countries.ok().and_then(|range| {
                let source = span(&self.countries, self.countries_len, range)?;
                Some(allocation::owned_vec(allocation::collect_vec(
                    source.iter().map(|item| rows::AirBaseCountryOccurrence {
                        country: item.country,
                        slot: item.slot,
                    }),
                )))
            }),
            host_name: row.host_name.ok().flatten().and_then(|range| {
                Some(allocation::owned_vec(allocation::copy_vec(span(
                    &self.names,
                    self.names_len,
                    range,
                )?)))
            }),
        }))
    }
}

pub(crate) struct Registrations {
    pub rows: Vec<CarrierRegistration>,
    pub pools: Vec<CarrierPool>,
    pub len: usize,
    pub pools_len: usize,
    indexed: bool,
    demand: CarrierDemand,
}

impl Registrations {
    pub fn new(rows: usize) -> Self {
        Self {
            rows: allocation::owned_vec(allocation::default_vec(rows)),
            pools: Vec::new(),
            len: 0,
            pools_len: 0,
            indexed: true,
            demand: CarrierDemand::default(),
        }
    }
    pub fn reset(&mut self) {
        self.len = 0;
        self.pools_len = 0;
        self.indexed = true;
        self.demand = CarrierDemand::default();
    }
    pub fn install(&mut self, lengths: CarrierLengths, demand: CarrierDemand) {
        if lengths.registrations > self.rows.len() || lengths.pools > self.pools.len() {
            self.reset();
            self.demand = demand;
            return;
        }
        self.len = lengths.registrations;
        self.pools_len = lengths.pools;
        self.demand = demand;
        self.indexed = false;
    }
    pub fn prepare(&mut self, demand: CarrierDemand) {
        prepare(&mut self.rows, demand.registrations);
        prepare(&mut self.pools, demand.pools);
    }
    pub fn grow(&mut self) {
        self.prepare(self.demand);
    }
    pub fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<CarrierRegistration>()
            + self.pools.capacity() * size_of::<CarrierPool>()
    }
    pub fn capacity(&self) -> [usize; 2] {
        [self.rows.len(), self.pools.len()]
    }
    /// Worker-only sorting of the outer lookup; source-order pools stay exact.
    pub fn index_rows(&mut self) {
        self.rows[..self.len].sort_unstable_by_key(|row| (row.base, row.slot));
        self.indexed = true;
    }
    fn row(&self, base: u64, slot: u32) -> Option<&CarrierRegistration> {
        if !self.indexed {
            return None;
        }
        let rows = self.rows.get(..self.len)?;
        let key = (base, slot);
        let index = rows
            .binary_search_by_key(&key, |row| (row.base, row.slot))
            .ok()?;
        if index
            .checked_sub(1)
            .and_then(|index| rows.get(index))
            .is_some_and(|row| (row.base, row.slot) == key)
            || rows
                .get(index + 1)
                .is_some_and(|row| (row.base, row.slot) == key)
        {
            return None;
        }
        rows.get(index)
    }
    fn pool_slice(&self, row: &CarrierRegistration, wing_len: usize) -> Option<&[CarrierPool]> {
        let pools = span(&self.pools, self.pools_len, row.pools.ok()?)?;
        let mut consumed = 0;
        for pool in pools {
            if pool.wings_start != consumed {
                return None;
            }
            consumed = consumed.checked_add(pool.wings_len as usize)?;
            if consumed > wing_len || u32::try_from(pool.wings_start).is_err() {
                return None;
            }
        }
        (consumed == wing_len).then_some(pools)
    }
    pub fn packet(
        &self,
        source: &cair_base::RegistrationRow,
    ) -> rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>> {
        rows::Recorded::Value(self.row(source.base, source.slot).map(|row| {
            let pools = self.pool_slice(row, source.wings_len).map(|source| {
                allocation::owned_vec(allocation::collect_vec(source.iter().map(|pool| {
                    rows::CarrierAirPool {
                        type_index: pool.type_index,
                        subunit_index: pool.subunit_index,
                        wing_start: pool.wings_start as u32,
                        wing_count: pool.wings_len,
                        current_fuel_raw: pool.fuel.current,
                        maximum_fuel_raw: pool.fuel.maximum,
                    }
                })))
            });
            rows::AirBaseRegistrationCarrierInputs {
                country: row.country,
                fuel_demand_raw: row.fuel.demand.ok(),
                fuel_allocated_raw: row.fuel.allocated.ok(),
                pools,
            }
        }))
    }
    /// Compare borrowed values before allocating a changed replay row.
    pub fn equal(
        &self,
        source: &cair_base::RegistrationRow,
        old: &rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>>,
    ) -> bool {
        let rows::Recorded::Value(old) = old else {
            return false;
        };
        match (self.row(source.base, source.slot), old) {
            (None, None) => true,
            (Some(row), Some(old)) => {
                if row.country != old.country
                    || row.fuel.demand.ok() != old.fuel_demand_raw
                    || row.fuel.allocated.ok() != old.fuel_allocated_raw
                {
                    return false;
                }
                match (self.pool_slice(row, source.wings_len), &old.pools) {
                    (None, None) => true,
                    (Some(current), Some(old)) => {
                        current.len() == old.len()
                            && current.iter().zip(old).all(|(current, old)| {
                                current.type_index == old.type_index
                                    && current.subunit_index == old.subunit_index
                                    && current.wings_start == old.wing_start as usize
                                    && current.wings_len == old.wing_count
                                    && current.fuel.current == old.current_fuel_raw
                                    && current.fuel.maximum == old.maximum_fuel_raw
                            })
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

pub(crate) struct Scratch {
    pub indexed: Vec<Option<CarrierIndexedCertificate>>,
    pub resolved: Vec<cair_base::ResolvedLink>,
    pub country_terms: Vec<CarrierCountryTerms>,
    pub aces: Vec<CarrierAceSlot>,
    pub forecasts: Vec<CarrierForecastSlot>,
}
impl Scratch {
    pub fn new() -> Self {
        Self {
            indexed: Vec::new(),
            resolved: Vec::new(),
            country_terms: Vec::new(),
            aces: Vec::new(),
            forecasts: Vec::new(),
        }
    }
    pub fn prepare(&mut self, demand: CarrierDemand) {
        prepare(&mut self.indexed, demand.indexed);
        prepare(&mut self.resolved, demand.resolved);
        prepare(&mut self.country_terms, demand.country_terms);
        prepare(&mut self.aces, demand.aces);
        prepare(&mut self.forecasts, demand.forecasts);
    }
    pub fn capacity(&self) -> [usize; 5] {
        [
            self.indexed.len(),
            self.resolved.len(),
            self.country_terms.len(),
            self.aces.len(),
            self.forecasts.len(),
        ]
    }
    pub fn bytes(&self) -> usize {
        self.indexed.capacity() * size_of::<Option<CarrierIndexedCertificate>>()
            + self.resolved.capacity() * size_of::<cair_base::ResolvedLink>()
            + self.country_terms.capacity() * size_of::<CarrierCountryTerms>()
            + self.aces.capacity() * size_of::<CarrierAceSlot>()
            + self.forecasts.capacity() * size_of::<CarrierForecastSlot>()
    }
}

pub(crate) fn buffers<'a>(
    bases: &'a mut Bases,
    registrations: &'a mut Registrations,
    scratch: &'a mut Scratch,
) -> CarrierBuffers<'a> {
    CarrierBuffers {
        bases: &mut bases.rows,
        registrations: &mut registrations.rows,
        pools: &mut registrations.pools,
        countries: &mut bases.countries,
        names: &mut bases.names,
        indexed: &mut scratch.indexed,
        resolved: &mut scratch.resolved,
        country_terms: &mut scratch.country_terms,
        aces: &mut scratch.aces,
        forecasts: &mut scratch.forecasts,
    }
}

pub(crate) fn buffers_growing<'a>(
    bases: &'a mut Bases,
    registrations: &'a mut Registrations,
    scratch: &'a mut Scratch,
) -> cair_base::GrowingCarrierBuffers<'a> {
    cair_base::GrowingCarrierBuffers {
        bases: &mut bases.rows,
        registrations: &mut registrations.rows,
        pools: &mut registrations.pools,
        countries: &mut bases.countries,
        names: &mut bases.names,
        indexed: &mut scratch.indexed,
        resolved: &mut scratch.resolved,
        country_terms: &mut scratch.country_terms,
        aces: &mut scratch.aces,
        forecasts: &mut scratch.forecasts,
    }
}

fn prepare<T: Default>(rows: &mut Vec<T>, required: usize) {
    let mut capacity = rows.len();
    crate::table_capture::grow_capacity(&mut capacity, required);
    crate::table_capture::grow_vec(rows, capacity);
}
fn span<T>(rows: &[T], len: usize, (start, count): (usize, usize)) -> Option<&[T]> {
    let end = start.checked_add(count)?;
    if end > len {
        return None;
    }
    rows.get(start..end)
}

/// Native optional staging is independent of the predecessor's RawLog. The
/// allocating AirBases worker grows from original observations; fixed callback
/// storage and owned decode use the fully prepared demand.
pub(crate) struct Job {
    pub log: RefCell<RawLog<2>>,
    pub index: Vec<usize>,
    pub required_index: usize,
    pub bases: Bases,
    pub registrations: Registrations,
    pub scratch: Scratch,
    pub demand: CarrierDemand,
    pub seed: Result<(), &'static str>,
}
impl std::fmt::Debug for Job {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CarrierJob")
            .field("log", &self.log)
            .field("index_capacity", &self.index.len())
            .field("demand", &self.demand)
            .field("seed", &self.seed)
            .finish()
    }
}
impl Default for Job {
    fn default() -> Self {
        Self {
            log: RefCell::new(RawLog::default()),
            index: Vec::new(),
            required_index: 0,
            bases: Bases::new(0),
            registrations: Registrations::new(0),
            scratch: Scratch::new(),
            demand: CarrierDemand::default(),
            seed: Err("carrier census seed not acquired"),
        }
    }
}
impl Job {
    pub(crate) fn presize(&mut self, demand: CarrierDemand) {
        self.bases.prepare(demand);
        self.registrations.prepare(demand);
        self.scratch.prepare(demand);
        self.log
            .get_mut()
            .presize(demand.raw_spans, demand.raw_bytes);
        if let Ok(required) = allocation::mul(demand.raw_spans, 2) {
            prepare(&mut self.index, required);
            self.index.fill(usize::MAX);
        }
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.log.get_mut().clear();
        self.index.fill(usize::MAX);
        self.required_index = 0;
        self.bases.reset();
        self.registrations.reset();
        self.demand = CarrierDemand::default();
        self.seed = Err("carrier census seed not acquired");
    }
    pub fn grow_for_next(&self) -> bool {
        let demand = self.demand;
        // A failure in one optional field does not suppress growth needed by
        // another independent field that exhausted the raw arena later.
        self.log.borrow().overflowed
            || self.required_index > self.index.len()
            || demand.bases > self.bases.rows.len()
            || demand.countries > self.bases.countries.len()
            || demand.name_bytes > self.bases.names.len()
            || demand.registrations > self.registrations.rows.len()
            || demand.pools > self.registrations.pools.len()
            || demand.indexed > self.scratch.indexed.len()
            || demand.resolved > self.scratch.resolved.len()
            || demand.country_terms > self.scratch.country_terms.len()
            || demand.aces > self.scratch.aces.len()
            || demand.forecasts > self.scratch.forecasts.len()
    }
    pub fn grow(&mut self) {
        self.log.get_mut().grow();
        prepare(&mut self.index, self.required_index);
        self.index.fill(usize::MAX);
        self.bases.prepare(self.demand);
        self.registrations.prepare(self.demand);
        self.scratch.prepare(self.demand);
    }
    pub fn bytes(&self) -> u64 {
        (self.index.capacity() * size_of::<usize>()
            + self.bases.bytes()
            + self.registrations.bytes()
            + self.scratch.bytes()) as u64
    }
}

/// Sole worker memo copy; capacities are retained on invalidation.
#[derive(Debug, Default)]
pub(crate) struct Memo {
    pub spans: Vec<crate::raw::Span>,
    pub bytes: Vec<u8>,
    pub has_bulk: bool,
    pub faults: [bool; 2],
    pub bases: Vec<CarrierBase>,
    pub countries: Vec<CountryOccurrence>,
    pub names: Vec<u8>,
    pub registrations: Vec<CarrierRegistration>,
    pub pools: Vec<CarrierPool>,
    pub demand: CarrierDemand,
}
impl Memo {
    pub fn bytes(&self) -> (u64, u64) {
        use crate::raw_memory::vector_bytes;
        (
            vector_bytes(&self.spans) + vector_bytes(&self.bytes),
            vector_bytes(&self.bases)
                + vector_bytes(&self.countries)
                + vector_bytes(&self.names)
                + vector_bytes(&self.registrations)
                + vector_bytes(&self.pools),
        )
    }
    pub fn input_equal(&self, log: &RawLog<2>) -> bool {
        !log.overflowed
            && self.has_bulk == log.has_bulk
            && self.spans == log.spans
            && self.bytes.as_slice() == log.bytes.as_ref()
            && self.faults == log.fault
    }
    pub fn save(
        &mut self,
        bases: &Bases,
        registrations: &Registrations,
        log: &RawLog<2>,
        demand: CarrierDemand,
    ) -> bool {
        fn save<T: Copy>(destination: &mut Vec<T>, source: &[T]) -> bool {
            if allocation::reserve(destination, source.len().saturating_sub(destination.len()))
                .is_err()
            {
                return false;
            }
            destination.clear();
            destination.extend_from_slice(source);
            true
        }
        if !save(&mut self.spans, &log.spans)
            || !save(&mut self.bytes, log.bytes.as_ref())
            || !save(&mut self.bases, &bases.rows[..bases.len])
            || !save(&mut self.countries, &bases.countries[..bases.countries_len])
            || !save(&mut self.names, &bases.names[..bases.names_len])
            || !save(
                &mut self.registrations,
                &registrations.rows[..registrations.len],
            )
            || !save(
                &mut self.pools,
                &registrations.pools[..registrations.pools_len],
            )
        {
            return false;
        }
        self.has_bulk = log.has_bulk;
        self.faults = log.fault;
        self.demand = demand;
        true
    }
    pub fn restore(&self, bases: &mut Bases, registrations: &mut Registrations) {
        bases.rows[..self.bases.len()].copy_from_slice(&self.bases);
        bases.countries[..self.countries.len()].copy_from_slice(&self.countries);
        bases.names[..self.names.len()].copy_from_slice(&self.names);
        registrations.rows[..self.registrations.len()].copy_from_slice(&self.registrations);
        registrations.pools[..self.pools.len()].copy_from_slice(&self.pools);
        let lengths = CarrierLengths {
            bases: self.bases.len(),
            registrations: self.registrations.len(),
            countries: self.countries.len(),
            names: self.names.len(),
            pools: self.pools.len(),
        };
        bases.install(lengths, self.demand);
        registrations.install(lengths, self.demand);
        // Memo outputs were saved after worker key preparation.
        bases.indexed = true;
        registrations.indexed = true;
    }
}

#[cfg(test)]
mod tests;
