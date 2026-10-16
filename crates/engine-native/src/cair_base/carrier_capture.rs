//! Optional carrier observation inside the existing registration walk.
//!
//! The native mirror and owned decoder feed the same authenticated occurrences.
//! The optional Memory acquires only missing operands; it may consult an exact
//! owned-key view of the main log first. Source addresses are transient keys,
//! never replay values and never a deferred native-memory permission.

use super::{ResolvedLink, Resolver, Row, carrier, fuel};
use crate::{
    Memory, ccountry, cname_group_member, cpdx_array::Array, creference_registry::ScopedResolver,
    walk::Buffer,
};

pub use carrier::{
    ContainerFuel, CountryOccurrence, IndexedContainer as CarrierIndexedCertificate,
};
pub use fuel::{
    AceSlot as CarrierAceSlot, CountryTerms as CarrierCountryTerms,
    ForecastSlot as CarrierForecastSlot, Packet as CarrierPoolFuel,
};

type Span = (usize, usize);
type Available<T> = Result<T, &'static str>;

/// Independent base suffixes. A failed dense census never erases a host name.
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

/// The two container scalars remain independent of pool membership success.
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

/// One complete registered pool. Wing boundaries are relative to its current
/// registration, including every repeated occurrence and every empty pool.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CarrierPool {
    pub type_index: i32,
    pub subunit_index: u32,
    pub wings_start: usize,
    pub wings_len: u32,
    pub fuel: CarrierPoolFuel,
    /// Signed-i32 wrapping sum of each wing's plane count, independent of fuel.
    pub planes_raw: Option<i32>,
}

/// Caller-owned admitted capacities; acquisition never grows these slices.
pub struct CarrierBuffers<'a> {
    pub bases: &'a mut [CarrierBase],
    pub registrations: &'a mut [CarrierRegistration],
    pub pools: &'a mut [CarrierPool],
    pub countries: &'a mut [CountryOccurrence],
    pub names: &'a mut [u8],
    pub indexed: &'a mut [Option<CarrierIndexedCertificate>],
    pub resolved: &'a mut [ResolvedLink],
    pub country_terms: &'a mut [CarrierCountryTerms],
    pub aces: &'a mut [CarrierAceSlot],
    pub forecasts: &'a mut [CarrierForecastSlot],
}

/// Worker-owned carrier arenas grow from original headers before admission.
pub struct GrowingCarrierBuffers<'a> {
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
    resolved: Buffer<'a, ResolvedLink>,
    country_terms: Buffer<'a, CarrierCountryTerms>,
    aces: Buffer<'a, CarrierAceSlot>,
    forecasts: Buffer<'a, CarrierForecastSlot>,
}

/// Off-thread growth demand. Checked overflow is retained in `failure`; it
/// never influences the predecessor's walk or becomes a smaller known demand.
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

/// Exactly one admitted observation, with no self-referential storage. The
/// caller's country reader has the same lifetime/Memory as these optional
/// operands. Its headers and every memo belong to this pass only.
pub struct CarrierCapture<'a, M: Memory> {
    memory: &'a M,
    image: usize,
    countries: &'a ccountry::Reader<'a, M>,
    resolved: Resolver<'a>,
    database: Option<Available<crate::csub_unit_definition::Database>>,
    references: ScopedResolver,
    fuel: fuel::Reader<'a, M>,
    bases: Buffer<'a, CarrierBase>,
    registrations: Buffer<'a, CarrierRegistration>,
    pools: Buffer<'a, CarrierPool>,
    dense: Buffer<'a, CountryOccurrence>,
    names: cname_group_member::NameArena<'a>,
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
        Self::new_buffers(
            memory,
            image,
            game_state,
            countries,
            CaptureBuffers {
                bases: Buffer::Fixed(buffers.bases),
                registrations: Buffer::Fixed(buffers.registrations),
                pools: Buffer::Fixed(buffers.pools),
                countries: Buffer::Fixed(buffers.countries),
                names: Buffer::Fixed(buffers.names),
                indexed: Buffer::Fixed(buffers.indexed),
                resolved: Buffer::Fixed(buffers.resolved),
                country_terms: Buffer::Fixed(buffers.country_terms),
                aces: Buffer::Fixed(buffers.aces),
                forecasts: Buffer::Fixed(buffers.forecasts),
            },
        )
    }

    pub fn new_growing(
        memory: &'a M,
        image: usize,
        game_state: usize,
        countries: &'a ccountry::Reader<'a, M>,
        buffers: GrowingCarrierBuffers<'a>,
    ) -> Result<Self, &'static str> {
        let [canonical, links] = countries.arrays();
        Buffer::Growing(&mut *buffers.resolved).ensure(links.count as usize)?;
        Buffer::Growing(&mut *buffers.country_terms).ensure(canonical.count as usize)?;
        Ok(Self::new_buffers(
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
                resolved: Buffer::Growing(buffers.resolved),
                country_terms: Buffer::Growing(buffers.country_terms),
                aces: Buffer::Growing(buffers.aces),
                forecasts: Buffer::Growing(buffers.forecasts),
            },
        ))
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
            mut resolved,
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
            names: cname_group_member::NameArena {
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

    /// The collector supplies the copied callback's numerical admission. A refusal leaves every
    /// legacy carrier/fuel attempt unchanged and forbids added wing plane-count reads, including
    /// when both fuel branches never reached it.
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

    /// Call after the original base identity, BEFORE its `COUNTRY_CONTAINERS` header read. An
    /// accepted base-row/name therefore survives a failed registration header. State-indexed bases
    /// pass `host_base = false`.
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

    /// The original registration reader supplies its validated live `COUNTRY_CONTAINERS` count. No
    /// optional header read or guessed country-count bound is substituted.
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

    /// Only an accepted current AirBases row may demand host bytes. The
    /// decoder supplies its O(1) traversal-order join; this method never reads
    /// base fields absent from the original registration log.
    pub fn authenticated_base(&mut self, source: Row) {
        let Some(base) = self.current.as_ref() else {
            return;
        };
        if source.id != base.handle || source.state.is_some() {
            return;
        }
        let name = carrier::host_name_buffer(
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

    /// The main pass already authenticated the exact container vtable, raw
    /// country reference and indexed ordinal. No E0/F0 success is needed.
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

    /// Begins one accepted indexed container before its original pool-header
    /// read, with the fuel scalars the original pass read in that container's
    /// span ([`super::container_span_words`]).
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

    /// Full pool demand comes from the original validated container header,
    /// even if a later pointer/identity fault stops its observed prefix.
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

    /// Called after the ORIGINAL pool vtable/header checks and before its
    /// original wing loop. Missing pool fields are acquired exactly once.
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
        // Twice the copied header's full member count bounds distinct ace and
        // forecast keys, including demand after a later original wing fault.
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

    /// Called once per already authenticated primary-wing occurrence, before
    /// the predecessor decides whether its own destination slice has space.
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
        // Each original occurrence enters the fuel accumulator exactly once; admitted transport
        // borrows that attempt's plane-count result.
        let planes = if let Some(read) = pool.fuel.as_mut() {
            self.fuel.wing_with_planes(read, pointer)
        } else {
            pointer
                .checked_add(crate::cair_wing::PLANE_COUNT)
                .and_then(|at| crate::read::<i32>(self.memory, at))
                .ok_or("carrier original wing planes unreadable or overflow")
        };
        pool.planes = pool
            .planes
            .and_then(|sum| planes.map(|planes| sum.wrapping_add(planes)));
    }

    /// Only the normal return after every original member calls this event.
    /// Empty pools reach it too; a failed member is never a complete empty sum.
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

    /// `accepted_wings` is the original whole registration's admitted local
    /// length, or None on truncation/fault. Scalars survive either result.
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

    /// Dense `REGISTERED_CONTAINERS` reading runs only after complete original indexed membership.
    /// A prefix/short certificate arena is never a complete indexed proof.
    pub fn finish_base(&mut self, original_complete: bool) {
        self.finish_registration(None);
        let Some(base) = self.current.take() else {
            return;
        };
        let result = if !original_complete {
            Err("carrier indexed registrations incomplete")
        } else if !base.indexed_complete {
            // The complete original pass established this base, but admitted
            // scratch did not hold all indexed certificates. Discover only
            // its validated dense-header capacity; never walk uncertified
            // pointers or turn this sizing read into an available suffix.
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

    /// The owner calls this on every early original return; no optional pool
    /// or dense prefix can then be mistaken for complete membership.
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
