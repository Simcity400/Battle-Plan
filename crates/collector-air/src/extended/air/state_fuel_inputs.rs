use super::{AirBasesJob, bases::HeldSegment};
use cair_base::{CarrierIndexedCertificate, CarrierPool, CarrierRegistration, ContainerFuel};
use collector_raw::raw::RawLog;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};
use engine_native_parts::cair_wing_pool;
use engine_native::ccountry;
use engine_native_air::{cair_base, ccountry_air_container as container};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct Packet {
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
        collector_primitives::raw_memory::vector_bytes(&self.rows)
            + collector_primitives::raw_memory::vector_bytes(&self.pools)
    }
}
#[derive(Debug)]
struct Container {
    pointer: usize,
    country: u32,
    slot: u32,
    pools: Result<Vec<CarrierPool>, &'static str>,
}
#[derive(Debug)]
struct Base {
    segment: u64,
    containers: Result<Vec<Container>, &'static str>,
}
#[derive(Debug, Default)]
pub struct Store {
    bases: HashMap<u64, Base>,
    resolved: Vec<cair_base::ResolvedLink>,
    visited: HashSet<u64>,
    index: Vec<SnapshotSpan>,
    index_scratch: Vec<SnapshotSpan>,
    fuel_pointers: Vec<usize>,
    fuel_words: Vec<u8>,
}
impl Store {
    pub fn clear(&mut self) {
        self.bases.clear();
    }
    pub fn bytes(&self) -> usize {
        self.bases.capacity() * std::mem::size_of::<(u64, Base)>()
            + self.resolved.capacity() * std::mem::size_of::<cair_base::ResolvedLink>()
            + self.visited.capacity() * std::mem::size_of::<u64>()
            + self.index.capacity() * std::mem::size_of::<SnapshotSpan>()
            + self.index_scratch.capacity() * std::mem::size_of::<SnapshotSpan>()
            + self.fuel_pointers.capacity() * std::mem::size_of::<usize>()
            + self.fuel_words.capacity()
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
            prepare_snapshot(log, &mut self.index, &mut self.index_scratch)?;
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
        self.fuel_pointers.clear();
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
            engine_allocation::reserve(&mut self.fuel_pointers, containers.len())
                .map_err(|_| "state air-base input row storage unavailable")?;
            for row in containers {
                self.fuel_pointers.push(row.pointer);
                let fuel = ContainerFuel {
                    demand: Err(UNREADABLE),
                    allocated: Err(UNREADABLE),
                };
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
                    fuel,
                    pools,
                });
            }
        }
        self.bases.retain(|handle, _| self.visited.contains(handle));
        let words = self.fuel_pointers.len() * FUEL_LAYOUT_BYTES;
        let missing = words.saturating_sub(self.fuel_words.len());
        if engine_allocation::reserve(&mut self.fuel_words, missing).is_ok() {
            self.fuel_words.resize(words, 0);
            gather_fuel(
                live,
                &self.fuel_pointers,
                &mut self.fuel_words,
                &mut output.rows,
            );
        } else {
            for (row, &pointer) in output.rows.iter_mut().zip(&self.fuel_pointers) {
                row.fuel = update_fuel(live, pointer);
            }
        }
        Ok(())
    }
}

const UNREADABLE: &str = "state air-base fuel scalar unreadable";
const FUEL_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: container::FUEL_DEMAND_RAW,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: container::FUEL_ALLOCATED_RAW,
        destination_offset: 8,
        len: 8,
    },
];
const FUEL_LAYOUT_BYTES: usize = 16;
const FUEL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&FUEL_FIELDS, FUEL_LAYOUT_BYTES).expect("valid record layout");

fn gather_fuel(
    memory: &impl Memory,
    pointers: &[usize],
    words: &mut [u8],
    rows: &mut [CarrierRegistration],
) {
    let word = |words: &[u8], row: usize, field: usize| {
        let at = row * FUEL_LAYOUT_BYTES + field * 8;
        i64::from_le_bytes(words[at..at + 8].try_into().unwrap())
    };
    let mut first = 0;
    while first < pointers.len() {
        let completed = memory.copy_records(
            RecordSources::Pointers(&pointers[first..]),
            &FUEL_LAYOUT,
            &mut words[first * FUEL_LAYOUT_BYTES..],
        );
        let failed = first + completed / FUEL_FIELDS.len();
        for row in first..failed.min(pointers.len()) {
            rows[row].fuel = ContainerFuel {
                demand: Ok(word(words, row, 0)),
                allocated: Ok(word(words, row, 1)),
            };
        }
        if failed >= pointers.len() {
            break;
        }
        rows[failed].fuel = if completed % FUEL_FIELDS.len() == 0 {
            ContainerFuel {
                demand: Err(UNREADABLE),
                allocated: scalar(memory, pointers[failed], container::FUEL_ALLOCATED_RAW),
            }
        } else {
            ContainerFuel {
                demand: Ok(word(words, failed, 0)),
                allocated: Err(UNREADABLE),
            }
        };
        first = failed + 1;
    }
}

fn update_fuel(memory: &impl Memory, pointer: usize) -> ContainerFuel {
    let addresses = [container::FUEL_DEMAND_RAW, container::FUEL_ALLOCATED_RAW].map(|offset| {
        pointer
            .checked_add(offset)
            .filter(|at| at.checked_add(8).is_some())
    });
    if let [Some(demand), Some(allocated)] = addresses {
        let mut words = [[0; 8]; 2];
        let completed = memory.copy_scattered(
            [demand, allocated],
            &mut words.each_mut().map(|word| word.as_mut_slice()),
        );
        ContainerFuel {
            demand: if completed != 0 {
                Ok(i64::from_le_bytes(words[0]))
            } else {
                Err("state air-base fuel scalar unreadable")
            },
            allocated: match completed {
                0 => scalar(memory, pointer, container::FUEL_ALLOCATED_RAW),
                1 => Err("state air-base fuel scalar unreadable"),
                _ => Ok(i64::from_le_bytes(words[1])),
            },
        }
    } else {
        ContainerFuel {
            demand: scalar(memory, pointer, container::FUEL_DEMAND_RAW),
            allocated: scalar(memory, pointer, container::FUEL_ALLOCATED_RAW),
        }
    }
}

fn scalar(memory: &impl Memory, pointer: usize, offset: usize) -> Result<i64, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("state air-base fuel scalar unreadable")
}
#[derive(Debug, Clone, Copy)]
struct SnapshotSpan {
    start: usize,
    end: usize,
    index: usize,
    maximum_end: usize,
}
fn prepare_snapshot(
    log: &RawLog<3>,
    index: &mut Vec<SnapshotSpan>,
    scratch: &mut Vec<SnapshotSpan>,
) -> Result<(), &'static str> {
    if index.len() == log.spans.iter().filter(|span| span.len != 0).count()
        && index.iter().all(|entry| {
            log.spans.get(entry.index).is_some_and(|span| {
                span.len != 0
                    && span.addr == entry.start
                    && span.addr.checked_add(span.len as usize) == Some(entry.end)
            })
        })
    {
        return Ok(());
    }
    index.clear();
    engine_allocation::reserve(index, log.spans.len())
        .map_err(|_| "state air-base snapshot index storage unavailable")?;
    for (ordinal, span) in log
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.len != 0)
    {
        let Some(end) = span.addr.checked_add(span.len as usize) else {
            index.clear();
            return Err("state air-base snapshot address overflow");
        };
        index.push(SnapshotSpan {
            start: span.addr,
            end,
            index: ordinal,
            maximum_end: 0,
        });
    }
    sort_snapshot(index, scratch);
    let mut maximum = 0;
    for span in index {
        maximum = maximum.max(span.end);
        span.maximum_end = maximum;
    }
    Ok(())
}
fn sort_snapshot(index: &mut Vec<SnapshotSpan>, scratch: &mut Vec<SnapshotSpan>) {
    let Some(&first) = index.first() else {
        return;
    };
    let varying = index
        .iter()
        .fold(0, |bits, span| bits | (span.start ^ first.start));
    if varying == 0 {
        return;
    }
    if scratch.capacity() < index.len()
        && engine_allocation::advisory(|| {
            engine_allocation::reserve(scratch, index.len().saturating_sub(scratch.len()))
        })
        .is_err()
    {
        index.sort_unstable_by_key(|span| (span.start, span.index));
        return;
    }
    scratch.resize(index.len(), first);
    for shift in (0..usize::BITS).step_by(8) {
        if (varying >> shift) & 255 == 0 {
            continue;
        }
        let mut positions = [0usize; 256];
        for span in index.iter() {
            positions[(span.start >> shift) & 255] += 1;
        }
        let mut next = 0;
        for position in &mut positions {
            let count = *position;
            *position = next;
            next += count;
        }
        for &span in index.iter() {
            let position = &mut positions[(span.start >> shift) & 255];
            scratch[*position] = span;
            *position += 1;
        }
        std::mem::swap(index, scratch);
    }
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
fn field<T: engine_native_memory::memory::Raw>(
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
        let expected = engine_sites::at(image, container::VTABLE_RVA)
            .ok_or("state air-base container identity mismatch")? as u64;
        if read::<u64>(recorded, pointer) != Some(expected) {
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
        result.push(Container {
            pointer,
            country,
            slot,
            pools,
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
        let expected = engine_sites::at(image, cair_wing_pool::VTABLE_RVA)
            .ok_or("state air-base pool identity mismatch")? as u64;
        if read::<u64>(recorded, pointer) != Some(expected) {
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

pub fn capture(job: &mut AirBasesJob, memory: &impl Memory, image: usize) {
    job.state_fuel.clear();
    if job.log.overflowed || job.registration_demand.is_none() {
        return;
    }
    let Some(store) = job.registration_store.as_deref_mut() else {
        return;
    };
    let counted = Counted {
        memory,
        bytes: Cell::new(0),
    };
    let _reads =
        collector_memory::read_accounting::table(replay_data::rows::TableId::AirBaseRegistrations);
    if store
        .state_inputs
        .capture(
            &counted,
            image,
            job.game_state,
            &job.log,
            &job.segments,
            &mut job.state_fuel,
        )
        .is_err()
    {
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

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.memory.copy_scattered(addresses, destinations);
        self.bytes.set(
            destinations[..completed]
                .iter()
                .fold(self.bytes.get(), |bytes, destination| {
                    bytes.saturating_add(destination.len() as u64)
                }),
        );
        completed
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.memory.copy_records(sources, layout, destination);
        self.bytes.set(
            self.bytes
                .get()
                .saturating_add(layout.completed_bytes(completed) as u64),
        );
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct ScalarMemory {
        demand: Cell<Option<i64>>,
        allocation: Cell<Option<i64>>,
        reads: Cell<usize>,
    }
    impl Memory for ScalarMemory {
        fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            let value = match at % 0x1000 {
                container::FUEL_DEMAND_RAW => self.demand.get(),
                container::FUEL_ALLOCATED_RAW => self.allocation.get(),
                _ => None,
            };
            let Some(value) = value else { return false };
            if bytes.len() != 8 {
                return false;
            }
            bytes.copy_from_slice(&value.to_le_bytes());
            true
        }
    }

    #[test]
    fn fuel_batches_keep_independent_faults_exact_spans_and_successful_byte_accounting() {
        use engine_native_memory::read_counts::{CountedMemory, ReadCounts};
        struct Batched {
            words: ScalarMemory,
            batches: Cell<usize>,
            singles: Cell<usize>,
            spans: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Batched {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.singles.set(self.singles.get() + 1);
                self.spans.borrow_mut().push((at, output.len()));
                self.words.copy(at, output)
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.batches.set(self.batches.get() + 1);
                for (n, (&at, output)) in addresses.iter().zip(destinations).enumerate() {
                    self.spans.borrow_mut().push((at, output.len()));
                    if !self.words.copy(at, output) {
                        return n;
                    }
                }
                N
            }
        }
        for (demand, allocation) in [
            (Some(-21), Some(7)),
            (Some(-21), None),
            (None, Some(7)),
            (None, None),
        ] {
            ReadCounts::take_current();
            let raw = CountedMemory::new(Batched {
                words: ScalarMemory {
                    demand: Cell::new(demand),
                    allocation: Cell::new(allocation),
                    reads: Cell::new(0),
                },
                batches: Cell::new(0),
                singles: Cell::new(0),
                spans: Default::default(),
            });
            let memory = Counted {
                memory: &raw,
                bytes: Cell::new(0),
            };
            let fuel = update_fuel(&memory, 0);
            assert_eq!(
                fuel.demand,
                demand.ok_or("state air-base fuel scalar unreadable")
            );
            assert_eq!(
                fuel.allocated,
                allocation.ok_or("state air-base fuel scalar unreadable")
            );
            assert_eq!(raw.0.batches.get(), 1);
            assert_eq!(raw.0.singles.get(), usize::from(demand.is_none()));
            assert_eq!(
                *raw.0.spans.borrow(),
                [
                    (container::FUEL_DEMAND_RAW, 8),
                    (container::FUEL_ALLOCATED_RAW, 8),
                ]
            );
            assert_eq!(
                memory.bytes.get(),
                8 * (u64::from(demand.is_some()) + u64::from(allocation.is_some()))
            );
            let counts = ReadCounts::take_current();
            assert_eq!((counts.spans, counts.bytes), (2, 16));
            assert_eq!(counts.scattered_calls, 1);
            assert_eq!(counts.copy_calls, u64::from(demand.is_none()));
        }
    }

    #[test]
    fn every_containers_fuel_is_one_gather_resumed_only_after_a_failed_word() {
        use engine_native_memory::read_counts::{CountedMemory, ReadCounts};
        let memory = CountedMemory::new(ScalarMemory {
            demand: Cell::new(Some(-21)),
            allocation: Cell::new(Some(7)),
            reads: Cell::new(0),
        });
        let mut rows = vec![CarrierRegistration::default(); 3];
        let mut words = vec![0; 3 * FUEL_LAYOUT_BYTES];
        ReadCounts::take_current();
        gather_fuel(&memory, &[0x1000, 0x3000], &mut words[..32], &mut rows[..2]);
        assert_eq!(ReadCounts::take_current().record_calls, 1);
        for row in &rows[..2] {
            assert_eq!((row.fuel.demand, row.fuel.allocated), (Ok(-21), Ok(7)));
        }
        gather_fuel(&memory, &[0x1000, 0x2001, 0x3000], &mut words, &mut rows);
        assert_eq!(ReadCounts::take_current().record_calls, 2);
        assert_eq!(
            rows.iter()
                .map(|row| (row.fuel.demand, row.fuel.allocated))
                .collect::<Vec<_>>(),
            [
                (Ok(-21), Ok(7)),
                (Err(UNREADABLE), Err(UNREADABLE)),
                (Ok(-21), Ok(7)),
            ]
        );
    }

    #[test]
    fn both_scalar_words_are_read_fresh_every_hour_without_writer_certificates() {
        let memory = ScalarMemory {
            demand: Cell::new(Some(-21)),
            allocation: Cell::new(Some(7)),
            reads: Cell::new(0),
        };
        let first = update_fuel(&memory, 0);
        assert_eq!(first.demand, Ok(-21));
        assert_eq!(first.allocated, Ok(7));
        memory.demand.set(Some(25));
        memory.allocation.set(Some(-11));
        let next = update_fuel(&memory, 0);
        assert_eq!(next.demand, Ok(25));
        assert_eq!(next.allocated, Ok(-11));
        assert_eq!(memory.reads.get(), 4);
        let quiet = update_fuel(&memory, 0);
        assert_eq!(quiet.demand, next.demand);
        assert_eq!(quiet.allocated, next.allocated);
        assert_eq!(memory.reads.get(), 6);
    }

    #[test]
    fn scalar_read_faults_refuse_only_the_failed_value_and_read_again_next_hour() {
        let memory = ScalarMemory {
            demand: Cell::new(None),
            allocation: Cell::new(Some(7)),
            reads: Cell::new(0),
        };
        let first = update_fuel(&memory, 0);
        assert_eq!(first.demand, Err("state air-base fuel scalar unreadable"));
        assert_eq!(first.allocated, Ok(7));
        memory.demand.set(Some(i64::MIN));
        memory.allocation.set(None);
        let next = update_fuel(&memory, 0);
        assert_eq!(next.demand, Ok(i64::MIN));
        assert_eq!(next.allocated, Err("state air-base fuel scalar unreadable"));
        memory.allocation.set(Some(i64::MAX));
        let recovered = update_fuel(&memory, 0);
        assert_eq!(recovered.demand, Ok(i64::MIN));
        assert_eq!(recovered.allocated, Ok(i64::MAX));
        assert_eq!(memory.reads.get(), 6);
    }

    #[test]
    fn overflowing_scalar_addresses_refuse_without_wrapping() {
        let memory = ScalarMemory {
            demand: Cell::new(Some(1)),
            allocation: Cell::new(Some(2)),
            reads: Cell::new(0),
        };
        let fuel = update_fuel(&memory, usize::MAX);
        assert_eq!(fuel.demand, Err("state air-base fuel scalar unreadable"));
        assert_eq!(fuel.allocated, Err("state air-base fuel scalar unreadable"));
        assert_eq!(memory.reads.get(), 0);
    }

    #[test]
    fn unchanged_registration_segments_refresh_every_containers_fuel_only() {
        let mut store = Store::default();
        let segment = HeldSegment {
            handle: 7,
            pointer: 0x1000,
            id: 1,
        };
        let pools = vec![CarrierPool {
            type_index: -4,
            subunit_index: 8,
            ..Default::default()
        }];
        store.bases.insert(
            segment.handle,
            Base {
                segment: segment.id,
                containers: Ok(vec![
                    Container {
                        pointer: 0x1000,
                        country: 1,
                        slot: 1,
                        pools: Ok(pools),
                    },
                    Container {
                        pointer: 0x2001,
                        country: 2,
                        slot: 2,
                        pools: Err("state air-base metadata field unreadable"),
                    },
                ]),
            },
        );
        let memory = ScalarMemory {
            demand: Cell::new(Some(-21)),
            allocation: Cell::new(Some(7)),
            reads: Cell::new(0),
        };
        let log = RawLog::<3>::new(0, 0);
        let mut packet = Packet::default();
        for (hour, demand, allocation) in [(1, -21, 7), (2, 33, -11), (3, 33, -11)] {
            memory.demand.set(Some(demand));
            memory.allocation.set(Some(allocation));
            store
                .capture(&memory, 0, 0, &log, &[segment], &mut packet)
                .unwrap();
            assert_eq!(packet.rows.len(), 2);
            assert_eq!(packet.rows[0].fuel.demand, Ok(demand));
            assert_eq!(packet.rows[0].fuel.allocated, Ok(allocation));
            assert_eq!(
                packet.rows[1].fuel.demand,
                Err("state air-base fuel scalar unreadable")
            );
            assert_eq!(
                packet.rows[1].fuel.allocated,
                Err("state air-base fuel scalar unreadable")
            );
            assert_eq!(packet.rows[0].pools, Ok((0, 1)));
            assert_eq!(
                packet.rows[1].pools,
                Err("state air-base metadata field unreadable")
            );
            assert_eq!(packet.pools.len(), 1);
            assert_eq!(packet.pools[0].type_index, -4);
            assert_eq!(packet.pools[0].subunit_index, 8);
            assert_eq!(memory.reads.get(), hour * 4);
        }
    }

    #[test]
    fn metadata_reads_owned_subfields_and_does_not_mask_a_failed_observation() {
        let source = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[7; 16],
        };
        let mut log = RawLog::<3>::new(3, 32);
        assert!(log.take(&source, 0x1000, 16, 0).is_some());
        let mut index = Vec::new();
        let mut scratch = Vec::new();
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        assert_eq!(
            read::<u64>(&snapshot, 0x1008),
            Some(u64::from_le_bytes([7; 8]))
        );
        log.spans.push(collector_raw::raw::Span {
            addr: 0x1008,
            len: 8,
            start: 0,
            mask: 0,
            ok: false,
            bulk: false,
        });
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        let live = ScalarMemory {
            demand: Cell::new(Some(1)),
            allocation: Cell::new(Some(2)),
            reads: Cell::new(0),
        };
        let metadata = MetadataMemory {
            recorded: &snapshot,
            live: &live,
        };
        assert_eq!(read::<u64>(&metadata, 0x1008), None);
        assert_eq!(live.reads.get(), 0);
    }

    #[test]
    fn snapshot_geometry_carries_but_current_values_and_failures_do_not() {
        let mut log = RawLog::<3>::new(3, 32);
        let mut index = Vec::new();
        let mut scratch = Vec::new();
        let read_at = |log: &RawLog<3>, index: &[SnapshotSpan], at| {
            read::<u64>(&SnapshotMemory { log, index }, at)
        };
        let first = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[7; 16],
        };
        assert!(log.take(&first, 0x1000, 16, 0).is_some());
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!(
            read_at(&log, &index, 0x1008),
            Some(u64::from_le_bytes([7; 8]))
        );
        log.clear();
        let second = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[9; 16],
        };
        assert!(log.take(&second, 0x1000, 16, 0).is_some());
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!(
            read_at(&log, &index, 0x1008),
            Some(u64::from_le_bytes([9; 8]))
        );
        log.spans[0].ok = false;
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!(read_at(&log, &index, 0x1008), None);
        log.clear();
        assert!(log.take(&second, 0x1000, 8, 0).is_some());
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!(
            read_at(&log, &index, 0x1000),
            Some(u64::from_le_bytes([9; 8]))
        );
        assert_eq!(read_at(&log, &index, 0x1008), None);
    }

    #[test]
    fn snapshot_address_passes_preserve_ordinal_order_and_last_covering_failures() {
        let mut log = RawLog::<3>::new(2048, 16);
        log.bytes.extend_from_slice(&[7; 16]);
        let mut random = 17usize;
        for ordinal in 0..1024 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            log.spans.push(collector_raw::raw::Span {
                addr: (random & (usize::MAX >> 1)) & !255,
                len: 16,
                start: 0,
                mask: 0,
                ok: ordinal % 3 != 0,
                bulk: false,
            });
        }
        for (address, ok) in [(0x1000, true), (0x1008, true), (0x1000, false)] {
            log.spans.push(collector_raw::raw::Span {
                addr: address,
                len: 16,
                start: 0,
                mask: 0,
                ok,
                bulk: false,
            });
        }
        let mut expected: Vec<_> = log
            .spans
            .iter()
            .enumerate()
            .map(|(ordinal, span)| (span.addr, ordinal))
            .collect();
        expected.sort_unstable();
        let mut index = Vec::new();
        let mut scratch = Vec::new();
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!(
            index
                .iter()
                .map(|span| (span.start, span.index))
                .collect::<Vec<_>>(),
            expected
        );
        let mut maximum = 0;
        for span in &index {
            maximum = maximum.max(span.end);
            assert_eq!(span.maximum_end, maximum);
        }
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        assert_eq!(snapshot.covering(0x1008, 8), Some(log.spans.len() - 1));
        assert_eq!(read::<u64>(&snapshot, 0x1008), None);
        assert_eq!(
            read::<u64>(&snapshot, 0x1010),
            Some(u64::from_le_bytes([7; 8]))
        );
        let capacities = (index.capacity(), scratch.capacity());
        index.clear();
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        assert_eq!((index.capacity(), scratch.capacity()), capacities);
        assert_eq!(
            index
                .iter()
                .map(|span| (span.start, span.index))
                .collect::<Vec<_>>(),
            expected
        );
        let mut no_scratch = Vec::new();
        index.clear();
        collector_test_support::with_allocations_denied(|| {
            prepare_snapshot(&log, &mut index, &mut no_scratch)
        })
        .unwrap();
        assert!(no_scratch.is_empty());
        assert_eq!(
            index
                .iter()
                .map(|span| (span.start, span.index))
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn a_refused_snapshot_index_cannot_certify_a_later_log() {
        let source = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[7; 16],
        };
        let mut log = RawLog::<3>::new(3, 32);
        assert!(log.take(&source, 0x1000, 16, 0).is_some());
        log.spans.push(collector_raw::raw::Span {
            addr: usize::MAX - 3,
            len: 8,
            start: 0,
            mask: 0,
            ok: true,
            bulk: false,
        });
        let mut index = Vec::new();
        let mut scratch = Vec::new();
        assert_eq!(
            prepare_snapshot(&log, &mut index, &mut scratch),
            Err("state air-base snapshot address overflow")
        );
        assert!(index.is_empty());
        log.spans.pop();
        prepare_snapshot(&log, &mut index, &mut scratch).unwrap();
        let snapshot = SnapshotMemory {
            log: &log,
            index: &index,
        };
        assert_eq!(
            read::<u64>(&snapshot, 0x1008),
            Some(u64::from_le_bytes([7; 8]))
        );
    }
}
