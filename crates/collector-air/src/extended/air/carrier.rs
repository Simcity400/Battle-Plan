use super::*;
use engine_native_air::cair_base::{
    CarrierAceSlot,
    CarrierBase,
    CarrierCountryTerms,
    CarrierDemand,
    CarrierForecastSlot,
    CarrierIndexedCertificate,
    CarrierLengths,
    CarrierPool,
    CarrierRegistration,
    CountryOccurrence,
};
use std::cell::RefCell;
use std::collections::HashMap;

struct CachedStateOrder {
    pointer: usize,
    segment: u64,
    countries: Result<Vec<CountryOccurrence>, &'static str>,
    pass: u64,
}

pub struct Bases {
    pub rows: Vec<CarrierBase>,
    pub countries: Vec<CountryOccurrence>,
    pub names: Vec<u8>,
    pub len: usize,
    pub countries_len: usize,
    pub names_len: usize,
    indexed: bool,
    demand: CarrierDemand,
    state_segments: Vec<(u64, usize, u64, usize)>,
    state_cache: HashMap<u64, CachedStateOrder>,
    state_cache_pass: u64,
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
            state_segments: Vec::new(),
            state_cache: HashMap::new(),
            state_cache_pass: 0,
        }
    }
    pub fn prepare_states(&mut self, job: &super::AirBasesJob) -> Result<(), &'static str> {
        self.state_segments.clear();
        allocation::reserve(&mut self.state_segments, job.segments.len())
            .map_err(|_| ALLOCATION_FAILED)?;
        self.state_segments.extend(
            job.segments
                .iter()
                .enumerate()
                .map(|(idx, segment)| (segment.handle, segment.pointer, segment.id, idx)),
        );
        self.state_segments
            .sort_unstable_by_key(|(handle, _, _, _)| *handle);
        let needs_log = job
            .segments
            .iter()
            .any(|segment| segment.carried_raw.is_none());
        let recorded = if needs_log {
            Some(collector_raw::raw::Recorded::build(&job.log)?)
        } else {
            None
        };
        let count = job
            .segments
            .iter()
            .try_fold(0_usize, |sum, segment| {
                let demand = match segment.carried_raw.as_deref() {
                    Some(raw) => raw.dense_count,
                    None => engine_native_air::cair_base::state_country_order_demand(
                        recorded.as_ref().unwrap(),
                        segment.pointer,
                    )
                    .unwrap_or(0),
                };
                sum.checked_add(demand)
            })
            .ok_or(DEMAND_OVERFLOW)?;
        let demand = job.carrier.demand;
        prepare(
            &mut self.countries,
            demand.countries.checked_add(count).ok_or(DEMAND_OVERFLOW)?,
        )?;
        prepare(
            &mut self.names,
            count
                .checked_mul(8)
                .and_then(|bytes| demand.name_bytes.checked_add(bytes))
                .ok_or(DEMAND_OVERFLOW)?,
        )?;
        prepare(
            &mut self.rows,
            demand
                .bases
                .checked_add(job.segments.len())
                .ok_or(DEMAND_OVERFLOW)?,
        )
    }

    #[cfg(test)]
    pub fn decode_states(
        &mut self,
        job: &super::AirBasesJob,
        image: usize,
        source: &[engine_native_air::cair_base::Row],
        resolved: &mut [engine_native_air::cair_base::ResolvedLink],
    ) -> Result<(), &'static str> {
        let log = collector_raw::raw::Recorded::build(&job.log)?;
        let recorded = crate::extended::roster_replay::RosterReplay::new(
            &log,
            &job.roster,
            image,
            job.game_state,
        );
        self.decode_states_with_recorded(job, image, source, resolved, &recorded)
    }

    pub(crate) fn decode_states_with_recorded<M: Memory>(
        &mut self,
        job: &super::AirBasesJob,
        image: usize,
        source: &[engine_native_air::cair_base::Row],
        resolved: &mut [engine_native_air::cair_base::ResolvedLink],
        recorded: &M,
    ) -> Result<(), &'static str> {
        let countries = engine_native::ccountry::Reader::new(recorded, image, job.game_state);
        self.state_cache_pass = self.state_cache_pass.wrapping_add(1);
        let pass = self.state_cache_pass;
        for base in source.iter().filter(|base| base.state.is_some()) {
            if self.len >= self.rows.len() {
                break;
            }
            let mut row = CarrierBase {
                base: base.id,
                countries: Err("state air-base country-order certificate unavailable"),
                host_name: Ok(None),
            };
            let matched = self
                .state_segments
                .binary_search_by_key(&base.id, |(handle, _, _, _)| *handle)
                .ok()
                .map(|at| self.state_segments[at]);
            let (Some((_, pointer, segment_id, job_idx)), Ok(countries)) = (matched, &countries)
            else {
                continue;
            };
            let start = self.countries_len;
            let cached_hit = if let Some(cached) = self.state_cache.get_mut(&base.id)
                && cached.pointer == pointer
                && cached.segment == segment_id
            {
                match &cached.countries {
                    Ok(items)
                        if self.countries[start..].len() >= items.len()
                            && engine_native_air::cair_base::verify_country_occurrences(
                                countries, resolved, items,
                            ) =>
                    {
                        self.countries[start..start + items.len()].copy_from_slice(items);
                        self.countries_len += items.len();
                        cached.pass = pass;
                        Some(Ok((start, items.len())))
                    }
                    Err(reason) => {
                        cached.pass = pass;
                        Some(Err(*reason))
                    }
                    _ => None,
                }
            } else {
                None
            };
            row.countries = match cached_hit {
                Some(hit) => hit,
                None => {
                    let result = match job.segments[job_idx].carried_raw.as_deref() {
                        Some(raw) => {
                            let overlay = raw.memory(recorded);
                            engine_native::ccountry::Reader::new(&overlay, image, job.game_state)
                                .and_then(|overlay_countries| {
                                    engine_native_air::cair_base::state_country_order(
                                        &overlay,
                                        image,
                                        pointer,
                                        base,
                                        &overlay_countries,
                                        resolved,
                                        &mut self.countries[start..],
                                        &mut self.names[self.names_len..],
                                    )
                                })
                        }
                        None => engine_native_air::cair_base::state_country_order(
                            recorded,
                            image,
                            pointer,
                            base,
                            countries,
                            resolved,
                            &mut self.countries[start..],
                            &mut self.names[self.names_len..],
                        ),
                    };
                    let cacheable = match result {
                        Ok(len) => {
                            self.countries_len += len;
                            let mut items = Vec::new();
                            if allocation::advisory(|| allocation::reserve(&mut items, len))
                                .is_ok()
                            {
                                items.extend_from_slice(&self.countries[start..start + len]);
                                Some(Ok(items))
                            } else {
                                None
                            }
                        }
                        Err(reason)
                            if reason != "state air-base country-order storage unavailable"
                                && reason
                                    != "state air-base country-order byte storage unavailable" =>
                        {
                            Some(Err(reason))
                        }
                        Err(_) => None,
                    };
                    if let Some(cached_countries) = cacheable
                        && allocation::advisory(|| allocation::reserve_map(&mut self.state_cache, 1))
                            .is_ok()
                    {
                        self.state_cache.insert(
                            base.id,
                            CachedStateOrder {
                                pointer,
                                segment: segment_id,
                                countries: cached_countries,
                                pass,
                            },
                        );
                    }
                    result.map(|len| (start, len))
                }
            };
            self.rows[self.len] = row;
            self.len += 1;
        }
        self.state_cache.retain(|_, cached| cached.pass == pass);
        self.index_rows();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.countries_len = 0;
        self.names_len = 0;
        self.demand = CarrierDemand::default();
        self.indexed = true;
    }
    pub fn install(&mut self, lengths: CarrierLengths, demand: CarrierDemand) {
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
    pub fn prepare(&mut self, demand: CarrierDemand) -> Result<(), &'static str> {
        prepare(&mut self.rows, demand.bases)?;
        prepare(&mut self.countries, demand.countries)?;
        prepare(&mut self.names, demand.name_bytes)
    }
    pub fn grow(&mut self) {
        let _ = self.prepare(self.demand);
    }
    pub fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<CarrierBase>()
            + self.countries.capacity() * size_of::<CountryOccurrence>()
            + self.names.capacity()
            + self.state_segments.capacity() * size_of::<(u64, usize, u64, usize)>()
            + self.state_cache.capacity() * size_of::<(u64, CachedStateOrder)>()
            + self
                .state_cache
                .values()
                .map(|cached| {
                    cached
                        .countries
                        .as_ref()
                        .map_or(0, |items| items.capacity() * size_of::<CountryOccurrence>())
                })
                .sum::<usize>()
    }
    pub fn capacity(&self) -> [usize; 3] {
        [self.rows.len(), self.countries.len(), self.names.len()]
    }
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
    pub fn equal(
        &self,
        base: u64,
        recorded: &rows::Recorded<Option<rows::AirBaseCarrierInputs>>,
    ) -> bool {
        let rows::Recorded::Value(recorded) = recorded else {
            return false;
        };
        let Some(row) = self.row(base) else {
            return recorded.is_none();
        };
        let Some(recorded) = recorded else {
            return false;
        };
        let countries = row
            .countries
            .ok()
            .and_then(|range| span(&self.countries, self.countries_len, range));
        let equal_countries = match (countries, recorded.registered_countries.as_deref()) {
            (None, None) => true,
            (Some(left), Some(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right).all(|(left, right)| {
                        left.country == right.country && left.slot == right.slot
                    })
            }
            _ => false,
        };
        let name = row
            .host_name
            .ok()
            .flatten()
            .and_then(|range| span(&self.names, self.names_len, range));
        equal_countries && name == recorded.host_name.as_deref()
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

pub struct Registrations {
    state: super::state_fuel_inputs::Packet,
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
            state: Default::default(),
            rows: allocation::owned_vec(allocation::default_vec(rows)),
            pools: Vec::new(),
            len: 0,
            pools_len: 0,
            indexed: true,
            demand: CarrierDemand::default(),
        }
    }
    pub fn reset(&mut self) {
        self.state.clear();
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
    pub fn prepare(&mut self, demand: CarrierDemand) -> Result<(), &'static str> {
        prepare(&mut self.rows, demand.registrations)?;
        prepare(&mut self.pools, demand.pools)
    }
    pub fn grow(&mut self) {
        let _ = self.prepare(self.demand);
    }
    pub fn bytes(&self) -> usize {
        self.state.bytes() as usize
            + self.rows.capacity() * size_of::<CarrierRegistration>()
            + self.pools.capacity() * size_of::<CarrierPool>()
    }
    pub fn capacity(&self) -> [usize; 2] {
        [self.rows.len(), self.pools.len()]
    }
    pub fn index_rows(&mut self) {
        self.rows[..self.len].sort_unstable_by_key(|row| (row.base, row.slot));
        self.indexed = true;
    }
    pub fn install_states(
        &mut self,
        packet: &super::state_fuel_inputs::Packet,
    ) -> Result<(), &'static str> {
        self.state.clear();
        allocation::reserve(&mut self.state.rows, packet.rows.len())
            .and_then(|()| allocation::reserve(&mut self.state.pools, packet.pools.len()))
            .map_err(|_| ALLOCATION_FAILED)?;
        self.state.rows.extend_from_slice(&packet.rows);
        self.state.pools.extend_from_slice(&packet.pools);
        self.state
            .rows
            .sort_unstable_by_key(|row| (row.base, row.slot));
        Ok(())
    }
    pub fn cursor(&self) -> RegistrationCursor<'_> {
        RegistrationCursor {
            registrations: self,
            state_at: std::cell::Cell::new(0),
            row_at: std::cell::Cell::new(0),
        }
    }
    pub fn registration(&self, source: &cair_base::RegistrationRow) -> Registration<'_> {
        self.cursor().registration(source)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn state_fuel(&self, source: &cair_base::RegistrationRow) -> [Option<i64>; 2] {
        self.registration(source).state_fuel()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn packet(
        &self,
        source: &cair_base::RegistrationRow,
    ) -> rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>> {
        self.registration(source).packet()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn equal(
        &self,
        source: &cair_base::RegistrationRow,
        old: &rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>>,
    ) -> bool {
        self.registration(source).equal(old)
    }
}

pub struct RegistrationCursor<'a> {
    registrations: &'a Registrations,
    state_at: std::cell::Cell<usize>,
    row_at: std::cell::Cell<usize>,
}

fn find_sorted<T>(
    rows: &[T],
    cursor: &std::cell::Cell<usize>,
    key: (u64, u32),
    key_of: impl Fn(&T) -> (u64, u32),
) -> Option<usize> {
    let pos = cursor.get();
    if pos > 0 && rows.get(pos - 1).is_some_and(|row| key_of(row) >= key) {
        return match rows.binary_search_by_key(&key, &key_of) {
            Ok(index) => {
                cursor.set(index + 1);
                Some(index)
            }
            Err(index) => {
                cursor.set(index);
                None
            }
        };
    }
    if let Some(row) = rows.get(pos) {
        match key_of(row).cmp(&key) {
            std::cmp::Ordering::Equal => {
                cursor.set(pos + 1);
                return Some(pos);
            }
            std::cmp::Ordering::Greater => return None,
            std::cmp::Ordering::Less => {}
        }
    } else {
        return None;
    }
    let mut low = pos + 1;
    let mut step = 1;
    while low + step < rows.len() && key_of(&rows[low + step]) < key {
        low += step;
        step <<= 1;
    }
    let end = (low + step + 1).min(rows.len());
    let at = low + rows[low..end].partition_point(|row| key_of(row) < key);
    if rows.get(at).is_some_and(|row| key_of(row) == key) {
        cursor.set(at + 1);
        Some(at)
    } else {
        cursor.set(at);
        None
    }
}

impl<'a> RegistrationCursor<'a> {
    pub fn registration(&self, source: &cair_base::RegistrationRow) -> Registration<'a> {
        let key = (source.base, source.slot);
        let registrations = self.registrations;
        if let Some(index) = find_sorted(&registrations.state.rows, &self.state_at, key, |row| {
            (row.base, row.slot)
        }) {
            return Registration {
                row: registrations.state.rows.get(index),
                pools: &registrations.state.pools,
                pools_len: registrations.state.pools.len(),
                wing_len: source.wings_len,
                state: true,
            };
        }
        let row = registrations
            .indexed
            .then(|| registrations.rows.get(..registrations.len))
            .flatten()
            .and_then(|rows| {
                let index = find_sorted(rows, &self.row_at, key, |row| (row.base, row.slot))?;
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
            });
        Registration {
            row,
            pools: &registrations.pools,
            pools_len: registrations.pools_len,
            wing_len: source.wings_len,
            state: false,
        }
    }
}

pub struct Registration<'a> {
    row: Option<&'a CarrierRegistration>,
    pools: &'a [CarrierPool],
    pools_len: usize,
    wing_len: usize,
    state: bool,
}
impl Registration<'_> {
    fn pool_slice(&self, row: &CarrierRegistration) -> Option<&[CarrierPool]> {
        let pools = span(self.pools, self.pools_len, row.pools.ok()?)?;
        let mut consumed = 0;
        for pool in pools {
            if pool.wings_start != consumed {
                return None;
            }
            consumed = consumed.checked_add(pool.wings_len as usize)?;
            if consumed > self.wing_len || u32::try_from(pool.wings_start).is_err() {
                return None;
            }
        }
        (consumed == self.wing_len).then_some(pools)
    }
    pub fn state_fuel(&self) -> [Option<i64>; 2] {
        self.row
            .filter(|_| self.state)
            .map(|row| [row.fuel.demand.ok(), row.fuel.allocated.ok()])
            .unwrap_or([None, None])
    }
    pub fn packet(&self) -> rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>> {
        rows::Recorded::Value(self.row.map(|row| {
            let pools = self.pool_slice(row).map(|source| {
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
                fuel_demand_raw: (!self.state).then(|| row.fuel.demand.ok()).flatten(),
                fuel_allocated_raw: (!self.state).then(|| row.fuel.allocated.ok()).flatten(),
                pools,
            }
        }))
    }
    pub fn equal(
        &self,
        old: &rows::Recorded<Option<rows::AirBaseRegistrationCarrierInputs>>,
    ) -> bool {
        let rows::Recorded::Value(old) = old else {
            return false;
        };
        match (self.row, old) {
            (None, None) => true,
            (Some(row), Some(old)) => {
                if row.country != old.country
                    || (!self.state).then(|| row.fuel.demand.ok()).flatten() != old.fuel_demand_raw
                    || (!self.state).then(|| row.fuel.allocated.ok()).flatten()
                        != old.fuel_allocated_raw
                {
                    return false;
                }
                match (self.pool_slice(row), &old.pools) {
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

pub struct Scratch {
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
    pub fn prepare(&mut self, demand: CarrierDemand) -> Result<(), &'static str> {
        prepare(&mut self.indexed, demand.indexed)?;
        prepare(&mut self.resolved, demand.resolved)?;
        prepare(&mut self.country_terms, demand.country_terms)?;
        prepare(&mut self.aces, demand.aces)?;
        prepare(&mut self.forecasts, demand.forecasts)
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

pub fn buffers<'a>(
    bases: &'a mut Bases,
    registrations: &'a mut Registrations,
    scratch: &'a mut Scratch,
) -> cair_base::CarrierBuffers<'a> {
    cair_base::CarrierBuffers {
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

const ALLOCATION_FAILED: &str = "capture buffer allocation failed";
const DEMAND_OVERFLOW: &str = "carrier state demand overflow";

pub fn prepare<T: Default>(rows: &mut Vec<T>, required: usize) -> Result<(), &'static str> {
    let mut capacity = rows.len();
    collector_primitives::table_capture::grow_capacity(&mut capacity, required);
    collector_primitives::table_capture::grow_vec(rows, capacity)
        .then_some(())
        .ok_or(ALLOCATION_FAILED)
}
fn span<T>(rows: &[T], len: usize, (start, count): (usize, usize)) -> Option<&[T]> {
    let end = start.checked_add(count)?;
    if end > len {
        return None;
    }
    rows.get(start..end)
}

pub struct Job {
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
    pub fn presize(&mut self, demand: CarrierDemand) {
        let _ = self.bases.prepare(demand);
        let _ = self.registrations.prepare(demand);
        let _ = self.scratch.prepare(demand);
        self.log
            .get_mut()
            .presize(demand.raw_spans, demand.raw_bytes);
        if let Ok(required) = allocation::mul(demand.raw_spans, 2) {
            let _ = prepare(&mut self.index, required);
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
        let _ = prepare(&mut self.index, self.required_index);
        self.index.fill(usize::MAX);
        let _ = self.bases.prepare(self.demand);
        let _ = self.registrations.prepare(self.demand);
        let _ = self.scratch.prepare(self.demand);
    }
    pub fn bytes(&self) -> u64 {
        (self.index.capacity() * size_of::<usize>()
            + self.bases.bytes()
            + self.registrations.bytes()
            + self.scratch.bytes()) as u64
    }
}

#[derive(Debug, Default)]
pub struct Memo {
    pub spans: Vec<collector_raw::raw::Span>,
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
        use collector_primitives::raw_memory::vector_bytes;
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
        bases.indexed = true;
        registrations.indexed = true;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
