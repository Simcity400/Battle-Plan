//! State base scalar observations and metadata-only pool projections.
//! Membership follows the existing registration segment; fuel words never
//! come from its deliberately zeroed carried spans. Only native writer marks
//! authorize another scalar read after the initial frozen observation.
use super::{AirBasesJob, bases::HeldSegment};
use crate::raw::RawLog;
use engine_native::{
    Memory, cair_base, cair_wing_pool, ccountry, ccountry_air_container as container,
    cpdx_array::Array, read,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
static REFUSALS: Mutex<std::collections::BTreeMap<&'static str, u64>> =
    Mutex::new(std::collections::BTreeMap::new());
fn refused(reason: &'static str) {
    let mut counts = REFUSALS.lock().unwrap_or_else(|error| error.into_inner());
    *counts.entry(reason).or_default() += 1;
}
pub(crate) fn status() -> impl serde::Serialize {
    REFUSALS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}
use cair_base::{CarrierIndexedCertificate, CarrierPool, CarrierRegistration, ContainerFuel};

#[derive(Debug, Default)]
pub(crate) struct Packet {
    pub rows: Vec<CarrierRegistration>,
    pub pools: Vec<CarrierPool>,
    pub native_bytes: u64,
}
impl Packet {
    pub fn clear(&mut self) {
        self.rows.clear();
        self.pools.clear();
        self.native_bytes = 0;
    }
    pub fn bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.rows) + crate::raw_memory::vector_bytes(&self.pools)
    }
}
#[derive(Debug)]
struct Container {
    pointer: usize,
    country: u32,
    slot: u32,
    pools: Result<Vec<CarrierPool>, &'static str>,
    certificate: Option<(u64, u64)>,
    observed: bool,
    fuel: ContainerFuel,
}
#[derive(Debug)]
struct Base {
    segment: u64,
    containers: Result<Vec<Container>, &'static str>,
}
#[derive(Debug, Default)]
pub(crate) struct Store {
    bases: HashMap<u64, Base>,
    resolved: Vec<cair_base::ResolvedLink>,
    visited: HashSet<u64>,
    index: Vec<SnapshotSpan>,
}
impl Store {
    pub(crate) fn clear(&mut self) {
        self.bases.clear();
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bases.capacity() * std::mem::size_of::<(u64, Base)>()
            + self.resolved.capacity() * std::mem::size_of::<cair_base::ResolvedLink>()
            + self.visited.capacity() * std::mem::size_of::<u64>()
            + self.index.capacity() * std::mem::size_of::<SnapshotSpan>()
            + self
                .bases
                .values()
                .map(|base| {
                    base.containers
                        .as_ref()
                        .map(|rows| {
                            rows.capacity() * std::mem::size_of::<Container>()
                                + rows
                                    .iter()
                                    .filter_map(|row| row.pools.as_ref().ok())
                                    .map(|pools| {
                                        pools.capacity() * std::mem::size_of::<CarrierPool>()
                                    })
                                    .sum::<usize>()
                        })
                        .unwrap_or(0)
                })
                .sum::<usize>()
    }
    fn capture<M: Memory>(
        &mut self,
        live: &M,
        image: usize,
        game_state: usize,
        log: &RawLog<3>,
        segments: &[HeldSegment],
        output: &mut Packet,
    ) -> Result<(), &'static str> {
        output.clear();
        let changed = segments.iter().any(|segment| {
            self.bases
                .get(&segment.handle)
                .is_none_or(|base| base.segment != segment.id)
        });
        if changed {
            prepare_snapshot(log, &mut self.index)?;
        }
        let recorded = SnapshotMemory {
            log,
            index: &self.index,
        };
        let metadata = MetadataMemory {
            recorded: &recorded,
            live,
        };
        let countries = changed
            .then(|| ccountry::Reader::new(&metadata, image, game_state))
            .transpose()?;
        if let Some(countries) = countries.as_ref() {
            let links = countries.arrays()[1].count as usize;
            let additional_links = links.saturating_sub(self.resolved.len());
            engine_allocation::reserve(&mut self.resolved, additional_links)
                .map_err(|_| "state air-base metadata storage unavailable")?;
            self.resolved.resize(links, None);
            self.resolved.fill(None);
        }
        self.visited.clear();
        engine_allocation::reserve_set(&mut self.visited, segments.len())
            .map_err(|_| "state air-base metadata storage unavailable")?;
        let mut count = 0;
        for segment in segments {
            self.visited.insert(segment.handle);
            let changed = self
                .bases
                .get(&segment.handle)
                .is_none_or(|base| base.segment != segment.id);
            if changed {
                let containers = read_membership(
                    &recorded,
                    &metadata,
                    image,
                    segment,
                    countries.as_ref().expect("changed base demands countries"),
                    &mut self.resolved,
                );
                if let Err(reason) = containers.as_ref() {
                    refused(reason);
                }
                engine_allocation::reserve_map(&mut self.bases, 1)
                    .map_err(|_| "state air-base metadata storage unavailable")?;
                self.bases.insert(
                    segment.handle,
                    Base {
                        segment: segment.id,
                        containers,
                    },
                );
            }
            let base = self
                .bases
                .get_mut(&segment.handle)
                .expect("inserted state base");
            let Ok(containers) = base.containers.as_mut() else {
                continue;
            };
            engine_allocation::reserve(&mut output.rows, containers.len())
                .map_err(|_| "state air-base input row storage unavailable")?;
            for row in containers {
                count += 1;
                let certificate = crate::air_base_carry_changes::fuel::sequence(row.pointer);
                let pointer = row.pointer;
                update_fuel(live, row, certificate, || {
                    crate::air_base_carry_changes::fuel::sequence(pointer)
                });
                let start = output.pools.len();
                let pools = match &row.pools {
                    Ok(pools) => {
                        engine_allocation::reserve(&mut output.pools, pools.len())
                            .map_err(|_| "state air-base pool storage unavailable")?;
                        output.pools.extend_from_slice(pools);
                        Ok((start, pools.len()))
                    }
                    Err(reason) => Err(*reason),
                };
                output.rows.push(CarrierRegistration {
                    base: segment.handle,
                    country: row.country,
                    slot: row.slot,
                    fuel: row.fuel,
                    pools,
                });
            }
        }
        self.bases.retain(|handle, _| self.visited.contains(handle));
        crate::air_base_carry_changes::fuel::record_demand(count);
        Ok(())
    }
}

/// A complete receiver certificate brackets every scalar observation. Missing
/// certificates never become polling; an overlapping writer refuses the pair
/// and discards the previous token instead of certifying mixed scalar words.
fn update_fuel(
    memory: &impl Memory,
    row: &mut Container,
    certificate: Option<(u64, u64)>,
    after: impl FnOnce() -> Option<(u64, u64)>,
) {
    let Some(certificate) = certificate else {
        refuse_fuel(row, "state air-base fuel writer certificate unavailable");
        return;
    };
    let should_read = !row.observed || row.certificate != Some(certificate);
    let fuel = if should_read {
        ContainerFuel {
            demand: scalar(memory, row.pointer, container::FUEL_DEMAND_RAW),
            allocated: scalar(memory, row.pointer, container::FUEL_ALLOCATED_RAW),
        }
    } else {
        row.fuel
    };
    let after = after();
    if after != Some(certificate) {
        refuse_fuel(
            row,
            "state air-base fuel writer changed during scalar observation",
        );
        // A completed intervening writer certifies this refusal, rather than
        // retrying the same quiet receiver until a read happens to succeed.
        // An in-flight/unavailable observer has no token to retain.
        row.certificate = after;
        return;
    }
    if should_read {
        for value in [fuel.demand, fuel.allocated] {
            if let Err(reason) = value {
                refused(reason);
            }
        }
        row.fuel = fuel;
        row.observed = true;
        row.certificate = Some(certificate);
    }
}
fn refuse_fuel(row: &mut Container, reason: &'static str) {
    refused(reason);
    row.fuel = ContainerFuel {
        demand: Err(reason),
        allocated: Err(reason),
    };
    row.certificate = None;
    row.observed = true;
}

fn scalar(memory: &impl Memory, pointer: usize, offset: usize) -> Result<i64, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("state air-base fuel scalar unreadable")
}
/// Extra subtype/owner leaves may read live only during a fresh membership
/// projection. Existing observations always win; held fuel words are unused.
// Metadata projects subfields of complete original spans. This indexed view
// uses owned bytes only, preserving a failed covering observation instead of
// rereading it. It is independent of the typed replay's exact-key cursor.
#[derive(Debug, Clone, Copy)]
struct SnapshotSpan {
    start: usize,
    end: usize,
    index: usize,
    maximum_end: usize,
}
fn prepare_snapshot(log: &RawLog<3>, index: &mut Vec<SnapshotSpan>) -> Result<(), &'static str> {
    index.clear();
    engine_allocation::reserve(index, log.spans.len())
        .map_err(|_| "state air-base snapshot index storage unavailable")?;
    for (ordinal, span) in log
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.len != 0)
    {
        let end = span
            .addr
            .checked_add(span.len as usize)
            .ok_or("state air-base snapshot address overflow")?;
        index.push(SnapshotSpan {
            start: span.addr,
            end,
            index: ordinal,
            maximum_end: 0,
        });
    }
    index.sort_unstable_by_key(|span| (span.start, span.index));
    let mut maximum = 0;
    for span in index {
        maximum = maximum.max(span.end);
        span.maximum_end = maximum;
    }
    Ok(())
}
struct SnapshotMemory<'a> {
    log: &'a RawLog<3>,
    index: &'a [SnapshotSpan],
}
impl SnapshotMemory<'_> {
    fn covering(&self, address: usize, len: usize) -> Option<usize> {
        let end = address.checked_add(len)?;
        let last = self.index.partition_point(|span| span.start <= address);
        let mut winner = None;
        for span in self.index[..last].iter().rev() {
            if span.maximum_end < end {
                break;
            }
            if span.end >= end && winner.is_none_or(|ordinal| ordinal < span.index) {
                winner = Some(span.index);
            }
        }
        winner
    }
}
impl Memory for SnapshotMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        let Some(index) = self.covering(address, destination.len()) else {
            return false;
        };
        let span = &self.log.spans[index];
        if !span.ok {
            return false;
        }
        let Some(start) = (span.start as usize).checked_add(address - span.addr) else {
            return false;
        };
        let Some(end) = start.checked_add(destination.len()) else {
            return false;
        };
        let Some(source) = self.log.bytes.get(start..end) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}
struct MetadataMemory<'a, 'b, M> {
    recorded: &'a SnapshotMemory<'b>,
    live: &'a M,
}
impl<M: Memory> Memory for MetadataMemory<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.recorded.covering(address, destination.len()).is_some() {
            self.recorded.copy(address, destination)
        } else {
            self.live.copy(address, destination)
        }
    }
}
fn field<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("state air-base metadata field unreadable")
}
fn array(memory: &impl Memory, object: usize, offset: usize) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("state air-base metadata vector unreadable")
}
fn read_membership<M: Memory>(
    recorded: &impl Memory,
    metadata: &M,
    image: usize,
    segment: &HeldSegment,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut [cair_base::ResolvedLink],
) -> Result<Vec<Container>, &'static str> {
    let dense = array(recorded, segment.pointer, cair_base::REGISTERED_CONTAINERS)?;
    let indexed = array(recorded, segment.pointer, cair_base::COUNTRY_CONTAINERS)?;
    let mut result = engine_allocation::vec(dense.count as usize)
        .map_err(|_| "state air-base metadata storage unavailable")?;
    let mut seen = HashSet::new();
    engine_allocation::reserve_set(&mut seen, dense.count as usize)
        .map_err(|_| "state air-base metadata storage unavailable")?;
    for ordinal in 0..dense.count {
        let at = dense
            .element(ordinal, 8)
            .ok_or("state air-base dense occurrence outside snapshot")?;
        let pointer = read::<u64>(recorded, at)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("state air-base dense occurrence unavailable")?;
        if !seen.insert(pointer) {
            continue;
        }
        if read::<u64>(recorded, pointer)
            != image.checked_add(container::VTABLE_RVA).map(|v| v as u64)
        {
            return Err("state air-base container identity mismatch");
        }
        let country = field::<u32>(recorded, pointer, container::COUNTRY)?;
        let slot = if country == 0 || countries.link(country)?.is_none() {
            0
        } else {
            countries.resolve(country)?.canonical_slot
        };
        let at = indexed
            .element(slot, 8)
            .ok_or("state air-base container slot outside snapshot")?;
        if read::<u64>(recorded, at) != Some(pointer as u64) {
            return Err("state air-base indexed container mismatch");
        }
        let pools = read_pools(
            recorded,
            metadata,
            image,
            CarrierIndexedCertificate {
                base: segment.handle,
                pointer,
                country,
                slot,
            },
            countries,
            resolved,
        );
        if let Err(reason) = pools.as_ref() {
            refused(reason);
        }
        result.push(Container {
            pointer,
            country,
            slot,
            pools,
            certificate: None,
            observed: false,
            fuel: ContainerFuel {
                demand: Err("state air-base fuel not observed"),
                allocated: Err("state air-base fuel not observed"),
            },
        });
    }
    result.sort_unstable_by_key(|row| row.slot);
    if result.windows(2).any(|rows| rows[0].slot == rows[1].slot) {
        return Err("state air-base containers share one indexed slot");
    }
    Ok(result)
}
fn read_pools<M: Memory>(
    recorded: &impl Memory,
    metadata: &M,
    image: usize,
    registration: CarrierIndexedCertificate,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut [cair_base::ResolvedLink],
) -> Result<Vec<CarrierPool>, &'static str> {
    let pools = array(recorded, registration.pointer, container::POOLS)?;
    let mut result = engine_allocation::vec(pools.count as usize)
        .map_err(|_| "state air-base pool storage unavailable")?;
    let mut consumed = 0;
    for ordinal in 0..pools.count {
        let pointer = pools
            .element(ordinal, 8)
            .and_then(|at| read::<u64>(recorded, at))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("state air-base pool occurrence unavailable")?;
        if read::<u64>(recorded, pointer)
            != image
                .checked_add(cair_wing_pool::VTABLE_RVA)
                .map(|v| v as u64)
        {
            return Err("state air-base pool identity mismatch");
        }
        let members = array(recorded, pointer, cair_wing_pool::WINGS)?;
        let pool = cair_base::state_pool_inputs(
            metadata,
            image,
            registration,
            pointer,
            members,
            consumed,
            countries,
            resolved,
        )?;
        consumed = consumed
            .checked_add(pool.wings_len as usize)
            .ok_or("state air-base pool wing boundary overflow")?;
        result.push(pool);
    }
    Ok(result)
}

/// Called only from the existing allocating AirBases worker, after its
/// registration snapshot completed. Failure preserves every predecessor row.
pub(crate) fn capture(job: &mut AirBasesJob, memory: &impl Memory, image: usize) {
    job.state_fuel.clear();
    if job.log.overflowed || job.registration_demand.is_none() {
        refused("state air-base original registration snapshot unavailable");
        return;
    }
    let Some(store) = job.registration_store.as_deref_mut() else {
        refused("state air-base registration writer certificate unavailable");
        return;
    };
    if store.bases() > job.segments.len() {
        refused("state air-base registration segment certificate unavailable");
    }
    let counted = Counted {
        memory,
        bytes: Cell::new(0),
    };
    let _reads = crate::read_accounting::table(replay_data::rows::TableId::AirBaseRegistrations);
    if let Err(reason) = store.state_inputs.capture(
        &counted,
        image,
        job.game_state,
        &job.log,
        &job.segments,
        &mut job.state_fuel,
    ) {
        refused(reason);
        job.state_fuel.clear();
    }
    job.state_fuel.native_bytes = counted.bytes.get();
    store.refresh_allocated_bytes();
}

struct Counted<'a, M> {
    memory: &'a M,
    bytes: Cell<u64>,
}
impl<M: Memory> Memory for Counted<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ok = self.memory.copy(address, destination);
        if ok {
            self.bytes
                .set(self.bytes.get().saturating_add(destination.len() as u64));
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn update_fuel(memory: &impl Memory, row: &mut Container, certificate: Option<(u64, u64)>) {
        super::update_fuel(memory, row, certificate, || certificate);
    }
    struct ScalarMemory {
        demand: Cell<i64>,
        allocation: Cell<i64>,
        reads: Cell<usize>,
    }
    impl Memory for ScalarMemory {
        fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            let value = match at {
                container::FUEL_DEMAND_RAW => self.demand.get(),
                container::FUEL_ALLOCATED_RAW => self.allocation.get(),
                _ => return false,
            };
            if bytes.len() != 8 {
                return false;
            }
            bytes.copy_from_slice(&value.to_le_bytes());
            true
        }
    }
    fn row() -> Container {
        Container {
            pointer: 0,
            country: 1,
            slot: 1,
            pools: Ok(vec![]),
            certificate: None,
            observed: false,
            fuel: ContainerFuel {
                demand: Err("unset"),
                allocated: Err("unset"),
            },
        }
    }
    #[test]
    fn quiet_scalar_words_carry_and_only_writer_changes_reread() {
        let memory = ScalarMemory {
            demand: Cell::new(-21),
            allocation: Cell::new(7),
            reads: Cell::new(0),
        };
        let mut row = row();
        update_fuel(&memory, &mut row, Some((1, 0)));
        assert_eq!(row.fuel.demand, Ok(-21));
        memory.demand.set(25);
        memory.allocation.set(-11);
        update_fuel(&memory, &mut row, Some((1, 0)));
        assert_eq!(memory.reads.get(), 2);
        assert_eq!(row.fuel.allocated, Ok(7));
        update_fuel(&memory, &mut row, Some((1, 1)));
        assert_eq!(memory.reads.get(), 4);
        assert_eq!(row.fuel.demand, Ok(25));
        assert_eq!(row.fuel.allocated, Ok(-11));
    }
    #[test]
    fn missing_certificate_never_becomes_hourly_polling_or_zero_fuel() {
        let memory = ScalarMemory {
            demand: Cell::new(33),
            allocation: Cell::new(2),
            reads: Cell::new(0),
        };
        let mut row = row();
        update_fuel(&memory, &mut row, None);
        assert!(row.fuel.demand.is_err());
        update_fuel(&memory, &mut row, None);
        assert_eq!(memory.reads.get(), 0);
        update_fuel(&memory, &mut row, Some((2, 0)));
        assert_eq!(row.fuel.demand, Ok(33));
        assert_eq!(memory.reads.get(), 2);
        update_fuel(&memory, &mut row, None);
        assert!(row.fuel.allocated.is_err());
        assert_eq!(row.certificate, None);
        assert_eq!(memory.reads.get(), 2);
    }
    #[test]
    fn a_writer_between_scalar_words_refuses_both_and_never_retries_a_quiet_token() {
        struct Racing {
            token: Cell<Option<(u64, u64)>>,
            reads: Cell<usize>,
        }
        impl Memory for Racing {
            fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                let value: i64 = match at {
                    container::FUEL_DEMAND_RAW => {
                        self.token.set(Some((1, 2)));
                        10
                    }
                    container::FUEL_ALLOCATED_RAW => 20,
                    _ => return false,
                };
                bytes.copy_from_slice(&value.to_le_bytes());
                true
            }
        }
        let memory = Racing {
            token: Cell::new(Some((1, 1))),
            reads: Cell::new(0),
        };
        let mut row = row();
        row.observed = true;
        row.certificate = Some((1, 0));
        row.fuel = ContainerFuel {
            demand: Ok(1),
            allocated: Ok(2),
        };
        super::update_fuel(&memory, &mut row, Some((1, 1)), || memory.token.get());
        assert!(row.fuel.demand.is_err() && row.fuel.allocated.is_err());
        assert_eq!(row.certificate, Some((1, 2)));
        assert_eq!(memory.reads.get(), 2);
        super::update_fuel(&memory, &mut row, Some((1, 2)), || Some((1, 2)));
        assert!(row.fuel.demand.is_err() && row.fuel.allocated.is_err());
        assert_eq!(memory.reads.get(), 2);
        let settled = ScalarMemory {
            demand: Cell::new(30),
            allocation: Cell::new(40),
            reads: Cell::new(0),
        };
        super::update_fuel(&settled, &mut row, Some((1, 3)), || Some((1, 3)));
        assert_eq!(row.fuel.demand, Ok(30));
        assert_eq!(row.fuel.allocated, Ok(40));
        assert_eq!(settled.reads.get(), 2);
        // A now-active writer cannot trigger uncertified fallback reads.
        memory.token.set(None);
        super::update_fuel(&memory, &mut row, None, || memory.token.get());
        assert_eq!(memory.reads.get(), 2);
    }
    #[test]
    fn a_carried_pair_is_refused_if_its_writer_starts_during_the_observation() {
        let memory = ScalarMemory {
            demand: Cell::new(33),
            allocation: Cell::new(2),
            reads: Cell::new(0),
        };
        let mut row = row();
        update_fuel(&memory, &mut row, Some((1, 0)));
        super::update_fuel(&memory, &mut row, Some((1, 0)), || None);
        assert!(row.fuel.demand.is_err() && row.fuel.allocated.is_err());
        assert_eq!(row.certificate, None);
        assert_eq!(memory.reads.get(), 2);
    }

    #[test]
    fn scalar_read_faults_are_carried_under_the_unchanged_complete_certificate() {
        let memory = ScalarMemory {
            demand: Cell::new(33),
            allocation: Cell::new(2),
            reads: Cell::new(0),
        };
        let mut row = row();
        row.pointer = 1;
        update_fuel(&memory, &mut row, Some((1, 0)));
        assert!(row.fuel.demand.is_err() && row.fuel.allocated.is_err());
        assert_eq!(row.certificate, Some((1, 0)));
        assert_eq!(memory.reads.get(), 2);
        update_fuel(&memory, &mut row, Some((1, 0)));
        assert_eq!(memory.reads.get(), 2);
        update_fuel(&memory, &mut row, Some((1, 1)));
        assert_eq!(memory.reads.get(), 4);
    }

    #[test]
    fn metadata_reads_owned_subfields_and_does_not_mask_a_failed_observation() {
        let source = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &[7; 16],
        };
        let mut log = RawLog::<3>::new(3, 32);
        assert!(log.take(&source, 0x1000, 16, 0).is_some());
        let mut index = Vec::new();
        prepare_snapshot(&log, &mut index).unwrap();
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        assert_eq!(
            read::<u64>(&snapshot, 0x1008),
            Some(u64::from_le_bytes([7; 8]))
        );
        log.spans.push(crate::raw::Span {
            addr: 0x1008,
            len: 8,
            start: 0,
            mask: 0,
            ok: false,
            bulk: false,
        });
        prepare_snapshot(&log, &mut index).unwrap();
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        let live = ScalarMemory {
            demand: Cell::new(1),
            allocation: Cell::new(2),
            reads: Cell::new(0),
        };
        let metadata = MetadataMemory {
            recorded: &snapshot,
            live: &live,
        };
        assert_eq!(read::<u64>(&metadata, 0x1008), None);
        assert_eq!(live.reads.get(), 0);
    }
}
