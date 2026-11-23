
use super::{ResolvedLink, Resolver, Row, carrier, fuel};
use engine_native_memory::{Memory, cpdx_array::Array, walk::Buffer};
use engine_native::{ccountry, cname_group_member};
use engine_native_parts::creference_registry::ScopedResolver;

pub use carrier::{
    ContainerFuel, CountryOccurrence, IndexedContainer as CarrierIndexedCertificate,
};
pub use fuel::{
    AceSlot as CarrierAceSlot, CountryTerms as CarrierCountryTerms,
    ForecastSlot as CarrierForecastSlot, Packet as CarrierPoolFuel,
};

type Span = (usize, usize);
type Available<T> = Result<T, &'static str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarrierBase {
    pub base: u64,
    pub countries: Available<Span>,
    pub host_name: Available<Option<Span>>,
}

impl Default for CarrierBase {
    fn default() -> Self {
        Self {
            base: 0,
            countries: Err("carrier base not observed"),
            host_name: Err("carrier base row not admitted"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarrierRegistration {
    pub base: u64,
    pub country: u32,
    pub slot: u32,
    pub fuel: ContainerFuel,
    pub pools: Available<Span>,
}

impl Default for CarrierRegistration {
    fn default() -> Self {
        Self {
            base: 0,
            country: 0,
            slot: 0,
            fuel: ContainerFuel {
                demand: Err("carrier container not observed"),
                allocated: Err("carrier container not observed"),
            },
            pools: Err("carrier registration membership incomplete"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CarrierPool {
    pub type_index: i32,
    pub subunit_index: u32,
    pub wings_start: usize,
    pub wings_len: u32,
    pub fuel: CarrierPoolFuel,
    pub planes_raw: Option<i32>,
}

pub struct CarrierBuffers<'a> {
    pub bases: &'a mut Vec<CarrierBase>,
    pub registrations: &'a mut Vec<CarrierRegistration>,
    pub pools: &'a mut Vec<CarrierPool>,
    pub countries: &'a mut Vec<CountryOccurrence>,
    pub names: &'a mut Vec<u8>,
    pub indexed: &'a mut Vec<Option<CarrierIndexedCertificate>>,
    pub resolved: &'a mut Vec<ResolvedLink>,
    pub country_terms: &'a mut Vec<CarrierCountryTerms>,
    pub aces: &'a mut Vec<CarrierAceSlot>,
    pub forecasts: &'a mut Vec<CarrierForecastSlot>,
}

struct CaptureBuffers<'a> {
    bases: Buffer<'a, CarrierBase>,
    registrations: Buffer<'a, CarrierRegistration>,
    pools: Buffer<'a, CarrierPool>,
    countries: Buffer<'a, CountryOccurrence>,
    names: Buffer<'a, u8>,
    indexed: Buffer<'a, Option<CarrierIndexedCertificate>>,
    resolved: &'a mut [ResolvedLink],
    country_terms: Buffer<'a, CarrierCountryTerms>,
    aces: Buffer<'a, CarrierAceSlot>,
    forecasts: Buffer<'a, CarrierForecastSlot>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CarrierDemand {
    pub bases: usize,
    pub registrations: usize,
    pub pools: usize,
    pub countries: usize,
    pub name_bytes: usize,
    pub indexed: usize,
    pub resolved: usize,
    pub country_terms: usize,
    pub aces: usize,
    pub forecasts: usize,
    pub raw_spans: usize,
    pub raw_bytes: usize,
    pub failure: Option<&'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CarrierLengths {
    pub bases: usize,
    pub registrations: usize,
    pub pools: usize,
    pub countries: usize,
    pub names: usize,
}

struct BaseRead {
    pointer: usize,
    handle: u64,
    output: Option<usize>,
    indexed_len: usize,
    indexed_complete: bool,
    registration: Option<CarrierIndexedCertificate>,
    registration_output: Option<usize>,
    pools_start: usize,
    pools_failure: Option<&'static str>,
    expected_pools: Option<u32>,
    seen_pools: u32,
    consumed_wings: usize,
    pool: Option<PoolRead>,
}

struct PoolRead {
    metadata: Available<carrier::PoolMetadata>,
    fuel: Option<fuel::PoolRead>,
    start: usize,
    members: Array,
    seen: u32,
    planes: Available<i32>,
}

pub struct CarrierCapture<'a, M: Memory> {
    memory: &'a M,
    image: usize,
    countries: &'a ccountry::Reader<'a, M>,
    resolved: Resolver<'a>,
    database: Option<Available<engine_native_parts::csub_unit_definition::Database>>,
    references: ScopedResolver,
    fuel: fuel::Reader<'a, M>,
    bases: Buffer<'a, CarrierBase>,
    registrations: Buffer<'a, CarrierRegistration>,
    pools: Buffer<'a, CarrierPool>,
    dense: Buffer<'a, CountryOccurrence>,
    names: cname_group_member::Names<'a>,
    indexed: Buffer<'a, Option<CarrierIndexedCertificate>>,
    lengths: CarrierLengths,
    demand: CarrierDemand,
    current: Option<BaseRead>,
    transport_planes: Available<()>,
}

impl<'a, M: Memory> CarrierCapture<'a, M> {

    pub fn new(
        memory: &'a M,
        image: usize,
        game_state: usize,
        countries: &'a ccountry::Reader<'a, M>,
        buffers: CarrierBuffers<'a>,
    ) -> Self {
        let [canonical, links] = countries.arrays();
        let refused = Buffer::Growing(&mut *buffers.resolved)
            .ensure(links.count as usize)
            .and_then(|_| {
                Buffer::Growing(&mut *buffers.country_terms).ensure(canonical.count as usize)
            })
            .err();
        let mut capture = Self::new_buffers(
            memory,
            image,
            game_state,
            countries,
            CaptureBuffers {
                bases: Buffer::Growing(buffers.bases),
                registrations: Buffer::Growing(buffers.registrations),
                pools: Buffer::Growing(buffers.pools),
                countries: Buffer::Growing(buffers.countries),
                names: Buffer::Growing(buffers.names),
                indexed: Buffer::Growing(buffers.indexed),
                resolved: buffers.resolved.as_mut_slice(),
                country_terms: Buffer::Growing(buffers.country_terms),
                aces: Buffer::Growing(buffers.aces),
                forecasts: Buffer::Growing(buffers.forecasts),
            },
        );
        if let Some(reason) = refused {
            capture.demand.failure.get_or_insert(reason);
        }
        capture
    }

    fn new_buffers(
        memory: &'a M,
        image: usize,
        game_state: usize,
        countries: &'a ccountry::Reader<'a, M>,
        buffers: CaptureBuffers<'a>,
    ) -> Self {
        let CaptureBuffers {
            bases,
            registrations,
            pools,
            countries: dense,
            names,
            mut indexed,
            resolved,
            country_terms,
            aces,
            forecasts,
        } = buffers;
        resolved.fill(None);
        indexed.fill(None);
        let [canonical, links] = countries.arrays();
        Self {
            memory,
            image,
            countries,
            resolved: Resolver { cells: resolved },
            database: None,
            references: ScopedResolver::new(image),
            fuel: fuel::Reader::new_buffers(
                memory,
                image,
                game_state,
                countries,
                country_terms,
                aces,
                forecasts,
            ),
            bases,
            registrations,
            pools,
            dense,
            names: cname_group_member::Names {
                bytes: names,
                len: 0,
                required: 0,
            },
            indexed,
            lengths: CarrierLengths::default(),
            demand: CarrierDemand {
                resolved: links.count as usize,
                country_terms: canonical.count as usize,
                ..CarrierDemand::default()
            },
            current: None,
            transport_planes: Err("transport carrier plane demand not admitted"),
        }
    }

    pub fn with_transport_planes(mut self, admission: Available<()>) -> Self {
        self.transport_planes = admission;
        self
    }

    pub fn lengths(&self) -> CarrierLengths {
        self.lengths
    }
    pub fn demand(&self) -> CarrierDemand {
        self.demand
    }

    pub fn begin_base(&mut self, pointer: usize, handle: u64, host_base: bool) {
        self.finish_base(false);
        if !host_base {
            return;
        }
        add(&mut self.demand.bases, 1, &mut self.demand.failure);
        if let Err(reason) = self.bases.ensure(self.demand.bases) {
            self.demand.failure.get_or_insert(reason);
        }
        let output = (self.lengths.bases < self.bases.len()).then_some(self.lengths.bases);
        if let Some(index) = output {
            self.bases[index] = CarrierBase {
                base: handle,
                ..CarrierBase::default()
            };
            self.lengths.bases += 1;
        }
        self.current = Some(BaseRead {
            pointer,
            handle,
            output,
            indexed_len: 0,
            indexed_complete: false,
            registration: None,
            registration_output: None,
            pools_start: self.lengths.pools,
            pools_failure: None,
            expected_pools: None,
            seen_pools: 0,
            consumed_wings: 0,
            pool: None,
        });
    }

    pub fn indexed_header(&mut self, indexed_count: u32) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        let count = indexed_count as usize;
        self.demand.indexed = self.demand.indexed.max(count);
        if let Err(reason) = self.indexed.ensure(count) {
            self.demand.failure.get_or_insert(reason);
        }
        let used = count.min(self.indexed.len());
        self.indexed[..used].fill(None);
        base.indexed_len = count;
        base.indexed_complete = count <= self.indexed.len();
    }

    pub fn authenticated_base(&mut self, source: Row) {
        let Some(base) = self.current.as_ref() else {
            return;
        };
        if source.id != base.handle || source.state.is_some() {
            return;
        }
        let name = carrier::host_name(
            self.memory,
            self.image,
            &source,
            &mut self.references,
            &mut self.names,
        );
        if let Some(output) = base.output {
            if let Ok(Some((start, len))) = name {
                if let Some(end) = start
                    .checked_add(len)
                    .filter(|end| *end <= self.names.bytes.len())
                {
                    self.names.len = end;
                    self.lengths.names = end;
                } else {
                    self.bases[output].host_name = Err("carrier host-name boundary invalid");
                    self.demand.name_bytes = self.names.required;
                    return;
                }
            }
            self.bases[output].host_name = name;
        }
        self.demand.name_bytes = self.names.required;
    }

    pub fn indexed_container(&mut self, country: u32, slot: u32, pointer: usize) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        if slot as usize >= base.indexed_len || pointer == 0 {
            base.indexed_complete = false;
            return;
        }
        let certificate = CarrierIndexedCertificate {
            base: base.handle,
            pointer,
            country,
            slot,
        };
        if let Some(cell) = self.indexed.get_mut(slot as usize) {
            *cell = Some(certificate);
        } else {
            base.indexed_complete = false;
        }
        base.registration = Some(certificate);
    }

    pub fn begin_registration(&mut self, fuel: ContainerFuel) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        let Some(certificate) = base.registration else {
            return;
        };
        add(&mut self.demand.registrations, 1, &mut self.demand.failure);
        base.pools_start = self.lengths.pools;
        base.pools_failure = None;
        base.expected_pools = None;
        base.seen_pools = 0;
        base.consumed_wings = 0;
        base.pool = None;
        if let Err(reason) = self.registrations.ensure(self.demand.registrations) {
            self.demand.failure.get_or_insert(reason);
        }
        base.registration_output = (self.lengths.registrations < self.registrations.len())
            .then_some(self.lengths.registrations);
        if let Some(index) = base.registration_output {
            self.registrations[index] = CarrierRegistration {
                base: certificate.base,
                country: certificate.country,
                slot: certificate.slot,
                fuel,
                ..CarrierRegistration::default()
            };
            self.lengths.registrations += 1;
        }
    }

    pub fn registration_header(&mut self, pool_count: u32) {
        let Some(base) = self
            .current
            .as_mut()
            .filter(|base| base.registration.is_some())
        else {
            return;
        };
        base.expected_pools = Some(pool_count);
        add(
            &mut self.demand.pools,
            pool_count as usize,
            &mut self.demand.failure,
        );
        if let Err(reason) = self.pools.ensure(self.demand.pools) {
            self.demand.failure.get_or_insert(reason);
        }
    }

    pub fn begin_pool(&mut self, pointer: usize, members: Array) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        let Some(registration) = base.registration else {
            return;
        };
        if base.pool.take().is_some() {
            base.pools_failure
                .get_or_insert("carrier previous pool membership incomplete");
        }
        match base.seen_pools.checked_add(1) {
            Some(count) => base.seen_pools = count,
            None => {
                base.pools_failure
                    .get_or_insert("carrier pool count overflow");
            }
        }
        match (members.count as usize).checked_mul(2) {
            Some(count) => {
                add(&mut self.demand.aces, count, &mut self.demand.failure);
                add(&mut self.demand.forecasts, count, &mut self.demand.failure);
            }
            None => {
                self.demand
                    .failure
                    .get_or_insert("carrier fuel memo demand overflow");
            }
        }
        let metadata = carrier::pool_metadata(
            self.memory,
            self.image,
            carrier::PoolObservation {
                pointer,
                registration,
                members,
                wings_start: base.consumed_wings,
            },
            self.countries,
            &mut self.resolved,
            &mut self.database,
        );
        let fuel = metadata.as_ref().ok().map(|metadata| {
            self.fuel.begin_pool(fuel::PoolInput {
                pointer: metadata.pool,
                owner: metadata.owner,
                base_reference: metadata.base_reference,
                definition: metadata.definition.pointer,
                wing_count: metadata.wings_len,
            })
        });
        base.pool = Some(PoolRead {
            metadata,
            fuel,
            start: base.consumed_wings,
            members,
            seen: 0,
            planes: self.transport_planes.map(|()| 0),
        });
    }

    pub fn wing(&mut self, pointer: usize) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        let Some(pool) = base.pool.as_mut() else {
            return;
        };
        let Some(seen) = pool.seen.checked_add(1) else {
            base.pools_failure
                .get_or_insert("carrier pool occurrence overflow");
            return;
        };
        pool.seen = seen;
        match base.consumed_wings.checked_add(1) {
            Some(count) => base.consumed_wings = count,
            None => {
                base.pools_failure
                    .get_or_insert("carrier registration wing boundary overflow");
            }
        }
        if self.transport_planes.is_err() {
            if let Some(read) = pool.fuel.as_mut() {
                self.fuel.wing(read, pointer);
            }
            return;
        }
        let planes = if let Some(read) = pool.fuel.as_mut() {
            self.fuel.wing_with_planes(read, pointer)
        } else {
            pointer
                .checked_add(crate::cair_wing::PLANE_COUNT)
                .and_then(|at| engine_native_memory::read::<i32>(self.memory, at))
                .ok_or("carrier original wing planes unreadable or overflow")
        };
        pool.planes = pool
            .planes
            .and_then(|sum| planes.map(|planes| sum.wrapping_add(planes)));
    }

    pub fn finish_pool(&mut self) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        let Some(pool) = base.pool.take() else {
            return;
        };
        let packet = pool.fuel.map(fuel::PoolRead::finish);
        let row = (|| {
            let metadata = pool.metadata?;
            if metadata.members != pool.members || metadata.wings_start != pool.start {
                return Err("carrier copied pool observation changed");
            }
            if pool.seen != pool.members.count || metadata.wings_len != pool.members.count {
                return Err("carrier pool membership incomplete");
            }
            if pool.start.checked_add(pool.seen as usize) != Some(base.consumed_wings) {
                return Err("carrier pool relative wing boundary invalid");
            }
            let fuel = packet.ok_or("carrier pool fuel input unavailable")?;
            Ok(CarrierPool {
                type_index: metadata.definition.type_index,
                subunit_index: metadata.definition.subunit_index,
                wings_start: pool.start,
                wings_len: pool.seen,
                fuel,
                planes_raw: pool.planes.ok(),
            })
        })();
        match row {
            Ok(row) => {
                if let Some(destination) = self.pools.get_mut(self.lengths.pools) {
                    *destination = row;
                    self.lengths.pools += 1;
                } else {
                    base.pools_failure
                        .get_or_insert("carrier pool arena too short");
                }
            }
            Err(reason) => {
                base.pools_failure.get_or_insert(reason);
            }
        }
    }

    pub fn finish_registration(&mut self, accepted_wings: Option<usize>) {
        let Some(base) = self.current.as_mut() else {
            return;
        };
        if base.registration.take().is_none() {
            return;
        }
        if base.pool.take().is_some() {
            base.pools_failure
                .get_or_insert("carrier pool membership incomplete");
        }
        if base.expected_pools != Some(base.seen_pools) {
            base.pools_failure
                .get_or_insert("carrier registration pool census incomplete");
        }
        if accepted_wings != Some(base.consumed_wings) {
            base.pools_failure
                .get_or_insert("carrier registration membership not admitted");
        }
        if base.registration_output.is_none() {
            base.pools_failure
                .get_or_insert("carrier registration arena too short");
        }
        let pools = match base.pools_failure {
            Some(reason) => {
                self.lengths.pools = base.pools_start;
                Err(reason)
            }
            None => Ok((base.pools_start, self.lengths.pools - base.pools_start)),
        };
        if let Some(output) = base.registration_output.take() {
            self.registrations[output].pools = pools;
        }
    }

    pub fn finish_base(&mut self, original_complete: bool) {
        self.finish_registration(None);
        let Some(base) = self.current.take() else {
            return;
        };
        let result = if !original_complete {
            Err("carrier indexed registrations incomplete")
        } else if !base.indexed_complete {
            match carrier::dense_demand(self.memory, base.pointer) {
                Ok(required) => add(
                    &mut self.demand.countries,
                    required,
                    &mut self.demand.failure,
                ),
                Err(reason) => {
                    self.demand.failure.get_or_insert(reason);
                }
            }
            Err("carrier indexed registrations incomplete")
        } else {
            let start = self.lengths.countries;
            let mut output = carrier::DenseCountries {
                rows: self.dense.suffix(start),
                len: 0,
                required: 0,
            };
            let result = carrier::dense_countries(
                self.memory,
                base.pointer,
                base.handle,
                self.countries,
                &mut self.resolved,
                &self.indexed[..base.indexed_len],
                &mut output,
            );
            add(
                &mut self.demand.countries,
                output.required,
                &mut self.demand.failure,
            );
            if result.is_ok() && base.output.is_some() {
                self.lengths.countries += output.len;
            }
            result.map(|()| (start, output.len))
        };
        if let Some(index) = base.output {
            self.bases[index].countries = result;
        }
    }

    pub fn finish_walk(&mut self) {
        self.finish_base(false);
    }
}

fn add(value: &mut usize, amount: usize, failure: &mut Option<&'static str>) {
    match value.checked_add(amount) {
        Some(next) => *value = next,
        None => {
            failure.get_or_insert("carrier observation demand overflow");
        }
    }
}
