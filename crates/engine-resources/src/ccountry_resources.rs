
use engine_native_memory::{
    Memory,
    batch_fallback::{Batch, Link},
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use engine_native::ccountry::{self, Ahead};
use crate::{cstrategic_resource, cstrategic_resource_pool};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29d8f00);
pub const OWNER: usize = 0x20;
pub const PRODUCED_POOL: usize = 0x28;
pub const ADJUSTMENT_POOL: usize = 0xd8;
pub const IMPORTED_POOL: usize = 0x188;
pub const USED_POOL: usize = 0x238;
pub const SPECIAL_USED_VALUES: usize = 0x240;
pub const TO_EXPORT_POOL: usize = 0x450;
pub const BASE_EXPORT_POOL: usize = 0x500;
pub const EXPORTED_POOL: usize = 0x5b0;
pub const RESOURCE_ORIGINS: usize = 0x710;
pub const BUILDING_RESOURCE_OVERRIDES: usize = 0x7a0;
pub const OVERRIDE_STRIDE: usize = 0x20;
pub const OVERRIDE_BUILDING: usize = 0x00;
pub const OVERRIDE_RESOURCES: usize = 0x08;
pub const OVERRIDE_RESOURCE_STRIDE: usize = 0x10;
pub const EXPORT_BUCKETS: usize = 0x728;
pub const EXPORT_BUCKET_STRIDE: usize = 0x18;
pub const IMPORT_ROUTES: usize = 0x740;
pub const DELIVERY_ROUTES: usize = 0x788;

pub const SPECIAL_TOTAL_POOL: usize = 0x2e8;
pub const PRODUCTION_LINE_POOL: usize = 0x398;

const POOLS: [usize; 9] = [
    PRODUCED_POOL,
    ADJUSTMENT_POOL,
    IMPORTED_POOL,
    USED_POOL,
    SPECIAL_TOTAL_POOL,
    PRODUCTION_LINE_POOL,
    TO_EXPORT_POOL,
    BASE_EXPORT_POOL,
    EXPORTED_POOL,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub resource: u32,
    pub produced: i64,
    pub adjustment: i64,
    pub imported: i64,
    pub used: i64,
    pub special_total: i64,
    pub production_demand: i64,
    pub to_export: i64,
    pub base_export: i64,
    pub exported: i64,
}

pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<[usize; 3], &'static str> {
    storage_demand_with_country(memory, base, state, |_, _| Ok(()))
}

pub fn storage_demand_with_country(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut visit: impl FnMut(engine_native_memory::country_identity::Country, usize) -> Result<(), &'static str>,
) -> Result<[usize; 3], &'static str> {
    let definitions = crate::cstrategic_resource_database::Reader::new(memory, base)?
        .count()
        .saturating_sub(1) as usize;
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut pools = 0_usize;
    let mut exchanges = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let manager = resources(memory, base, &countries, country)?;
        visit(country, manager)?;
        let routes = manager
            .checked_add(IMPORT_ROUTES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("resource sizing import routes unreadable or invalid")?;
        pools = pools
            .checked_add(definitions)
            .ok_or("resource sizing row demand overflow")?;
        exchanges = exchanges
            .checked_add(routes.count as usize)
            .ok_or("resource sizing exchange demand overflow")?;
    }
    Ok([definitions, pools, exchanges])
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        descriptors,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_buffer_with_country(memory, base, game_state, descriptors, output, |_, _| {})
}

pub fn walk_buffer_with_country(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut visit: impl FnMut(engine_native_memory::country_identity::Country, usize),
) -> Outcome {
    let result = (|| {
        let count = u32::try_from(descriptors.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("resource descriptor count overflow")?;
        for (index, descriptor) in descriptors.iter().enumerate() {
            if descriptor.id != index as u32 + 1 {
                return Err("resource descriptor census is incomplete or unordered");
            }
        }
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut addresses = [0_usize; VALUE_CHUNK * 9];
        let mut records = [0_u8; VALUE_CHUNK * 9 * 12];
        let mut blocks = [[0_u8; VALUE_CHUNK * VALUE_STRIDE]; 9];
        let mut ahead = None;
        for slot in 1..countries.count() {
            let (listed, link) = Ahead::take(&mut ahead, &countries, slot);
            let Some(country) = listed? else {
                continue;
            };
            found_country = true;
            let pointer = resources_linked(memory, base, &countries, country, link)?;
            visit(country, pointer);
            let pools = pools(memory, base, pointer, count)?;
            ahead = Ahead::read(
                memory,
                &countries,
                slot + 1,
                ccountry::RESOURCES,
                |object| {
                    memory.prefetch(object, OWNER + 4);
                    for offset in POOLS {
                        if let Some(header) = object.checked_add(offset) {
                            memory.prefetch(header, 24);
                        }
                    }
                },
            );
            output.ensure(
                len.checked_add(descriptors.len())
                    .ok_or("country resource row count overflow")?,
            )?;
            let stored = descriptors.len().min(output.len() - len);
            let mut first = 0;
            while first < stored {
                let chunk = (stored - first).min(VALUE_CHUNK);
                let whole = value_blocks(memory, &pools, descriptors[first].id, chunk, &mut blocks);
                let mut outside = None;
                let mut completed = chunk * 9;
                if !whole {
                    engine_native_memory::batch_fallback::record(Batch::CountryResources);
                    (outside, completed) = value_records(
                        memory,
                        &pools,
                        &descriptors[first..first + chunk],
                        &mut addresses,
                        &mut records,
                    );
                }
                for slot in 0..chunk {
                    if outside == Some(slot) {
                        return Err("resource value outside pool");
                    }
                    if completed < (slot + 1) * 9 {
                        return Err("resource value record unreadable");
                    }
                    let descriptor = &descriptors[first + slot];
                    let mut values = [0_i64; 9];
                    for (value, pool) in values.iter_mut().enumerate() {
                        let record = if whole {
                            let start = slot * VALUE_STRIDE;
                            &blocks[value][start..start + 12]
                        } else {
                            let start = (slot * 9 + value) * 12;
                            &records[start..start + 12]
                        };
                        *pool = cstrategic_resource_pool::from_record(
                            record.try_into().unwrap(),
                            descriptor.token,
                        )?;
                    }
                    output[len] = Row {
                        country: country.id,
                        resource: descriptor.id,
                        produced: values[0],
                        adjustment: values[1],
                        imported: values[2],
                        used: values[3],
                        special_total: values[4],
                        production_demand: values[5],
                        to_export: values[6],
                        base_export: values[7],
                        exported: values[8],
                    };
                    len += 1;
                }
                first += chunk;
            }
            for _ in stored..descriptors.len() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("resource unread row count overflow")?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub(crate) fn resources<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    resources_linked(memory, base, countries, country, Link::Unread)
}

fn resources_linked<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    link: Link,
) -> Result<usize, &'static str> {
    let pointer = link
        .resolve(
            memory,
            country,
            ccountry::RESOURCES,
            Batch::CountryResources,
        )
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country resources pointer unreadable or null")?;
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    const FIELDS: [RecordField; 2] = engine_native_memory::memory::packed_fields([(0, 8), (OWNER, 4)]);
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, 12).expect("valid record layout");
    let owner = if pointer.checked_add(OWNER + 4).is_some() {
        let mut bytes = [0_u8; 12];
        let copied = memory.copy_records(RecordSources::Pointers(&[pointer]), &LAYOUT, &mut bytes);
        if copied < 1 || u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
            return Err("country resources vtable mismatch");
        }
        if copied < 2 {
            return Err("country resources owner unreadable");
        }
        u32::from_le_bytes(bytes[8..12].try_into().unwrap())
    } else {
        if read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("country resources vtable mismatch");
        }
        return Err("country resources owner unreadable");
    };
    if countries.resolve(owner)?.pointer != country.pointer {
        return Err("country resources owner disagrees with country");
    }
    Ok(pointer)
}

fn pools(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    count: u32,
) -> Result<[Array; 9], &'static str> {
    let mut addresses = [0_usize; 9];
    for (address, offset) in addresses.iter_mut().zip(POOLS) {
        *address = pointer
            .checked_add(offset)
            .ok_or("resource pool address overflow")?;
    }
    let mut headers = [[0_u8; 24]; 9];
    if memory
        .copy_scattered_prefetched(addresses, &mut headers.each_mut().map(|h| h.as_mut_slice()))
        != 9
    {
        return Err("resource pool header unreadable");
    }
    let mut pools = [Array { data: 0, count: 0 }; 9];
    for (pool, header) in pools.iter_mut().zip(headers) {
        *pool = cstrategic_resource_pool::from_header(base, header, count)?;
    }
    Ok(pools)
}

fn value_blocks(
    memory: &impl Memory,
    pools: &[Array; 9],
    first: u32,
    chunk: usize,
    blocks: &mut [[u8; VALUE_CHUNK * VALUE_STRIDE]; 9],
) -> bool {
    let len = (chunk - 1) * VALUE_STRIDE + 12;
    let last = first + (chunk as u32 - 1);
    let mut starts = [0_usize; 9];
    for (start, pool) in starts.iter_mut().zip(pools) {
        match (
            pool.element(first, VALUE_STRIDE),
            pool.element(last, VALUE_STRIDE),
        ) {
            (Some(element), Some(_)) => *start = element,
            _ => return false,
        }
    }
    memory.copy_scattered_prefetched(
        starts,
        &mut blocks.each_mut().map(|block| &mut block[..len]),
    ) == 9
}

fn value_records(
    memory: &impl Memory,
    pools: &[Array; 9],
    descriptors: &[cstrategic_resource::Row],
    addresses: &mut [usize; VALUE_CHUNK * 9],
    records: &mut [u8; VALUE_CHUNK * 9 * 12],
) -> (Option<usize>, usize) {
    let mut outside = None;
    'stage: for (slot, descriptor) in descriptors.iter().enumerate() {
        for (pool, address) in pools.iter().zip(&mut addresses[slot * 9..slot * 9 + 9]) {
            match pool.element(descriptor.id, VALUE_STRIDE) {
                Some(element) => *address = element,
                None => {
                    outside = Some(slot);
                    break 'stage;
                }
            }
        }
    }
    let staged = outside.unwrap_or(descriptors.len());
    let completed = memory.copy_records(
        RecordSources::Pointers(&addresses[..staged * 9]),
        &VALUE_LAYOUT,
        &mut records[..staged * 9 * 12],
    );
    (outside, completed)
}

const VALUE_CHUNK: usize = 32;
const VALUE_STRIDE: usize = cstrategic_resource_pool::VALUE_STRIDE;
const VALUE_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 12,
}];
const VALUE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&VALUE_FIELD, 12).expect("valid record layout");

#[cfg(test)]
#[path = "ccountry_resources_cost_baseline.rs"]
mod baseline;

#[cfg(test)]
pub(crate) mod scale_support {
    use super::*;
    use engine_native::ccountry::synthetic::World;

    pub(crate) trait Image {
        fn alloc(&mut self, len: usize) -> usize;
        fn u64(&mut self, address: usize, value: u64);
        fn u32(&mut self, address: usize, value: u32);
        fn array(&mut self, header: usize, data: usize, count: u32);
        fn vtable(&self, rva: engine_sites::Rva) -> u64;
    }

    impl Image for World {
        fn alloc(&mut self, len: usize) -> usize {
            World::alloc(self, len)
        }
        fn u64(&mut self, address: usize, value: u64) {
            World::u64(self, address, value);
        }
        fn u32(&mut self, address: usize, value: u32) {
            World::u32(self, address, value);
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            World::array(self, header, data, count);
        }
        fn vtable(&self, rva: engine_sites::Rva) -> u64 {
            (self.module + rva.0) as u64
        }
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    impl Image for engine_native::crelation::status::bench::Arena {
        fn alloc(&mut self, len: usize) -> usize {
            engine_native::crelation::status::bench::Arena::alloc(self, len)
        }
        fn u64(&mut self, address: usize, value: u64) {
            engine_native::crelation::status::bench::Arena::u64(self, address, value);
        }
        fn u32(&mut self, address: usize, value: u32) {
            engine_native::crelation::status::bench::Arena::u32(self, address, value);
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            self.vector(header, data, count);
        }
        fn vtable(&self, rva: engine_sites::Rva) -> u64 {
            self.rva(rva)
        }
    }

    pub(crate) fn descriptors(resources: u32) -> Vec<cstrategic_resource::Row> {
        (1..=resources)
            .map(|id| cstrategic_resource::Row {
                id,
                token: 99 + id,
                ..Default::default()
            })
            .collect()
    }

    pub(crate) fn populate(image: &mut impl Image, objects: &[usize], resources: u32) {
        for (slot, &country) in objects.iter().enumerate().skip(1) {
            let object = image.alloc(0x800);
            image.u64(object, image.vtable(VTABLE_RVA));
            image.u32(object + OWNER, slot as u32);
            image.u64(country + ccountry::RESOURCES, object as u64);
            for (pool, offset) in POOLS.into_iter().enumerate() {
                let data = image.alloc((resources as usize + 1) * VALUE_STRIDE);
                image.u64(
                    object + offset,
                    image.vtable(cstrategic_resource_pool::VTABLE_RVA),
                );
                image.array(object + offset + 8, data, resources + 1);
                for id in 1..=resources {
                    let at = data + id as usize * VALUE_STRIDE;
                    image.u64(at, (slot * 1_000 + pool * 10) as u64 + u64::from(id));
                    image.u32(at + 8, 99 + id);
                }
            }
        }
    }

    pub(crate) fn resource_world(countries: u32, resources: u32) -> World {
        let mut w = World::new(0, countries, ccountry::RESOURCES + 8);
        let objects: Vec<usize> = (0..=countries).map(|slot| w.object(slot)).collect();
        populate(&mut w, &objects, resources);
        w
    }

}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use engine_native::cgame_state;
    use crate::{cresource_exchange as trade, cstrategic_resource_database as database};
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct Fixture {
        pub bytes: BTreeMap<usize, u8>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = address.checked_add(index).and_then(|p| self.bytes.get(&p))
                else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl Fixture {
        pub fn put(&mut self, address: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.bytes.insert(address + index, *byte);
            }
        }
        pub fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
        pub fn new() -> Self {
            let mut f = Self::default();
            f.array(0x1000 + cgame_state::COUNTRIES, 0x1500, 3);
            f.u64(0x1508, 0x2000);
            f.u64(0x1510, 0x4000);
            f.array(0x1000 + cgame_state::COUNTRY_LINKS, 0x1600, 8);
            f.u32(0x1604, 1);
            f.u32(0x1608, 2);
            f.u32(0x161c, 1);
            for (ordinal, (country, resources, raw)) in [(0x2000, 0x6000, 7), (0x4000, 0x7000, 2)]
                .into_iter()
                .enumerate()
            {
                f.u64(country, ccountry::VTABLE_RVA.0 as u64);
                f.u32(country + ccountry::ID, raw);
                f.u64(country + ccountry::RESOURCES, resources as u64);
                f.u64(resources, VTABLE_RVA.0 as u64);
                f.u32(resources + OWNER, raw);
                f.array(resources + IMPORT_ROUTES, 0, 0);
                let buckets = 0xc000 + ordinal * 0x200;
                f.array(resources + EXPORT_BUCKETS, buckets, 2);
                f.array(buckets, 0, 0);
                f.array(buckets + EXPORT_BUCKET_STRIDE, 0, 0);
                for (pool, offset) in POOLS.into_iter().enumerate() {
                    let data = 0x10000 + ordinal * 0x2000 + pool * 0x80;
                    f.u64(
                        resources + offset,
                        cstrategic_resource_pool::VTABLE_RVA.0 as u64,
                    );
                    f.array(resources + offset + 8, data, 2);
                    f.put(data, &[0; 32]);
                    f.u64(data + 16, (4_294_967_297_i64 + pool as i64) as u64);
                    f.u32(data + 24, 19);
                }
            }
            f.put(0xa000, &[0; trade::SIZE]);
            f.u64(0xa000, trade::VTABLE_RVA.0 as u64);
            f.u64(0xa000 + trade::REFERENCE, 0x1234_5678_0000_0033);
            f.put(0xa000 + trade::REGISTERED, &[1]);
            f.u32(0xa000 + trade::OWNER, 2);
            f.u64(0xa000 + trade::EXPORTER_RESOURCES, 0x6000);
            f.u32(0xa000 + trade::IMPORTER, 2);
            f.u64(0xa000 + trade::RESOURCE, 0xd000);
            f.u64(0xa000 + trade::DELIVERED, 4_294_967_299);
            f.u64(0xa000 + trade::EFFICIENCY_A, 100_000);
            f.u64(0xa000 + trade::EFFICIENCY_B, 99_999);
            f.u32(0xa000 + trade::REQUIRED_FACTORIES, 9);
            f.u32(0xa000 + trade::ASSIGNED_FACTORIES, 8);
            f.array(0x7000 + IMPORT_ROUTES, 0xb100, 1);
            f.u64(0xb100, 0xa000);
            f.array(0xc000 + EXPORT_BUCKET_STRIDE, 0xc100, 1);
            f.u64(0xc100, 0xa000);
            f.u64(0xd000, cstrategic_resource::VTABLE_RVA.0 as u64);
            f.u32(0xd000 + cstrategic_resource::INDEX, 1);
            f.u32(0xd000 + cstrategic_resource::TOKEN_ID, 19);
            f.put(0xd000 + cstrategic_resource::ACTIVE, &[1]);
            f.u64(0xd000 + cstrategic_resource::FACTORY_COST, 4_294_967_300);
            f.u64(database::SINGLETON_RVA.0, 0xe000);
            f.array(0xe000 + database::ITEMS, 0xe100, 2);
            f.u64(0xe100, 0xdf00);
            f.u64(0xe108, 0xd000);
            f.array(0xe000 + database::KEYS, 0xe200, 1);
            f.u32(0xe200, 19);
            f
        }
        pub fn descriptors(&self) -> Vec<cstrategic_resource::Row> {
            let mut rows = vec![cstrategic_resource::Row::default(); 1];
            let result = cstrategic_resource::walk(self, 0, &mut rows);
            assert_eq!(result, Outcome::Rows { len: 1 });
            rows
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Fixture;

    fn record(w: &engine_native::ccountry::synthetic::World, slot: u32, pool: usize, id: u32) -> usize {
        let memory = w.memory();
        let object = engine_native_memory::read::<u64>(&memory, w.object(slot) + engine_native::ccountry::RESOURCES)
            .unwrap() as usize;
        let data = engine_native_memory::read::<u64>(&memory, object + POOLS[pool] + 8).unwrap() as usize;
        data + id as usize * VALUE_STRIDE
    }

    fn both(
        memory: &impl Memory,
        serial_memory: &impl Memory,
        w: &engine_native::ccountry::synthetic::World,
        descriptors: &[cstrategic_resource::Row],
        rows: usize,
    ) -> [(Outcome, Vec<Row>); 2] {
        let mut batched = vec![Row::default(); rows];
        let mut serial = batched.clone();
        let new = walk(memory, 0, w.game_state(), descriptors, &mut batched);
        let old = super::baseline::walk_buffer(
            serial_memory,
            0,
            w.game_state(),
            descriptors,
            engine_native_memory::walk::Buffer::Growing(&mut serial),
        );
        [(new, batched), (old, serial)]
    }

    #[test]
    fn batched_values_match_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        use engine_native_memory::memory::{CountedMemory, ReadCounts};
        const COUNT: u32 = 100;
        const RESOURCES: u32 = 7;
        let w = resource_world(COUNT, RESOURCES);
        let descriptors = descriptors(RESOURCES);
        let memory = CountedMemory::new(w.memory());
        let rows = (COUNT * RESOURCES) as usize;
        let prior = ReadCounts::take_current();
        let mut batched = vec![Row::default(); rows];
        let new = walk(&memory, 0, w.game_state(), &descriptors, &mut batched);
        let new_reads = ReadCounts::take_current();
        let mut serial = vec![Row::default(); rows];
        let old = super::baseline::walk_buffer(
            &memory,
            0,
            w.game_state(),
            &descriptors,
            engine_native_memory::walk::Buffer::Growing(&mut serial),
        );
        let old_reads = ReadCounts::take_current();
        prior.add_current();
        assert_eq!(new, Outcome::Rows { len: rows });
        assert_eq!((new, &batched), (old, &serial));
        let n = u64::from(COUNT);
        assert_eq!(new_reads.copy_calls + 2 * n, old_reads.copy_calls);
        assert_eq!(new_reads.record_calls, old_reads.record_calls);
        assert_eq!(new_reads.scattered_calls, old_reads.scattered_calls + n);
        assert_eq!(new_reads.spans + 54 * n, old_reads.spans);
    }

    fn fault_sites(w: &engine_native::ccountry::synthetic::World) -> [std::ops::Range<usize>; 4] {
        let padding = record(w, 3, 4, 2) + 12;
        let value = record(w, 3, 4, 5);
        let link = w.object(4) + engine_native::ccountry::RESOURCES;
        let object = engine_native_memory::read::<u64>(&w.memory(), link).unwrap() as usize;
        let header = object + POOLS[6];
        [
            padding..padding + 4,
            value..value + 12,
            link..link + 8,
            header..header + 24,
        ]
    }

    fn fallbacks() -> u64 {
        engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::CountryResources)
    }

    #[test]
    fn a_persistent_fault_gives_the_serial_result() {
        use super::scale_support::{descriptors, resource_world};
        let w = resource_world(6, 7);
        let descriptors = descriptors(7);
        for (site, (range, counted)) in fault_sites(&w).into_iter().zip([1, 1, 1, 0]).enumerate() {
            let before = fallbacks();
            let memory = engine_native_memory::batch_fallback::Faulty::new(w.memory(), vec![range], false);
            let [batched, serial] = both(&memory, &memory, &w, &descriptors, 42);
            assert_eq!(batched, serial, "site {site}");
            assert_eq!(fallbacks() - before, counted, "site {site}");
            if site == 0 {
                assert_eq!(batched.0, Outcome::Rows { len: 42 });
            } else {
                assert!(
                    matches!(batched.0, Outcome::Unavailable { .. }),
                    "site {site}"
                );
            }
        }
    }

    #[test]
    fn a_one_shot_fault_refuses_no_more_than_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        let w = resource_world(6, 7);
        let descriptors = descriptors(7);
        let [healthy, _] = both(&w.memory(), &w.memory(), &w, &descriptors, 42);
        for (site, (range, counted)) in fault_sites(&w).into_iter().zip([1, 1, 1, 0]).enumerate() {
            let once = || engine_native_memory::batch_fallback::Faulty::new(w.memory(), vec![range.clone()], true);
            let before = fallbacks();
            let [batched, serial] = both(&once(), &once(), &w, &descriptors, 42);
            assert_eq!(fallbacks() - before, counted, "site {site}");
            if site < 3 {
                assert_eq!(batched, healthy, "site {site}");
                assert_eq!(
                    matches!(serial.0, Outcome::Unavailable { .. }),
                    site != 0,
                    "site {site}"
                );
            } else {
                assert_eq!(batched, serial, "site {site}");
            }
        }
    }

    #[test]
    fn batched_values_cross_the_block_edge_as_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        use engine_native_memory::batch_fallback::Faulty;
        const RESOURCES: u32 = 33;
        let w = resource_world(6, RESOURCES);
        let descriptors = descriptors(RESOURCES);
        let all = 6 * RESOURCES as usize;
        for rows in [all, 32, 33, 34, 65, 66 + 32, 0] {
            let [batched, serial] = both(&w.memory(), &w.memory(), &w, &descriptors, rows);
            assert_eq!(batched, serial, "rows {rows}");
        }
        let second = record(&w, 3, 2, 33);
        let second = second..second + 12;
        let before = fallbacks();
        let memory = Faulty::new(w.memory(), vec![second.clone()], false);
        let [batched, serial] = both(&memory, &memory, &w, &descriptors, all);
        assert_eq!(batched, serial);
        assert!(matches!(batched.0, Outcome::Unavailable { .. }));
        assert_eq!(fallbacks() - before, 1);
        let before = fallbacks();
        let once = || Faulty::new(w.memory(), vec![second.clone()], true);
        let [batched, serial] = both(&once(), &once(), &w, &descriptors, all);
        assert_eq!(batched.0, Outcome::Rows { len: all });
        assert!(matches!(serial.0, Outcome::Unavailable { .. }));
        assert_eq!(fallbacks() - before, 1);
    }

    #[test]
    fn null_and_alias_slots_are_skipped_as_the_serial_walk_skips_them() {
        use super::scale_support::{descriptors, resource_world};
        use engine_native_memory::batch_fallback::Faulty;
        let mut w = resource_world(8, 7);
        let descriptors = descriptors(7);
        let table = engine_native::ccountry::synthetic::START + 0x1000;
        let links = table + 9 * 8;
        w.u64(table + 3 * 8, 0);
        w.u64(table + 5 * 8, w.object(4) as u64);
        w.u32(links + 5 * 4, 4);
        let rows = 6 * 7;
        let [healthy, serial] = both(&w.memory(), &w.memory(), &w, &descriptors, rows);
        assert_eq!(healthy, serial);
        assert_eq!(healthy.0, Outcome::Rows { len: rows });
        let link = |slot: u32| {
            let at = w.object(slot) + engine_native::ccountry::RESOURCES;
            at..at + 8
        };
        for (slot, counted) in [(2, 1), (4, 0)] {
            let before = fallbacks();
            let once = || Faulty::new(w.memory(), vec![link(slot)], true);
            let [batched, serial] = both(&once(), &once(), &w, &descriptors, rows);
            assert_eq!(fallbacks() - before, counted, "slot {slot}");
            assert!(
                matches!(serial.0, Outcome::Unavailable { .. }),
                "slot {slot}"
            );
            if counted == 1 {
                assert_eq!(batched, healthy, "slot {slot}");
            } else {
                assert_eq!(batched, serial, "slot {slot}");
            }
            let memory = Faulty::new(w.memory(), vec![link(slot)], false);
            let [batched, serial] = both(&memory, &memory, &w, &descriptors, rows);
            assert_eq!(batched, serial, "slot {slot}");
        }
    }

    use super::*;

    #[test]
    fn resource_pool_rows_preserve_raw_widths_and_alias_country_identity() {
        let mut fixture = Fixture::new();
        let descriptors = fixture.descriptors();
        fixture.u64(0x10090, (-4_294_967_297_i64) as u64);
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!((rows[0].country, rows[0].resource), (7, 1));
        assert_eq!(rows[0].produced, 4_294_967_297);
        assert_eq!(rows[0].adjustment, -4_294_967_297);
        assert_eq!(rows[0].exported, 4_294_967_305);
        assert_eq!(rows[1].country, 2);
    }

    #[test]
    fn a_mid_batch_descriptor_fault_keeps_its_reason_and_position() {
        let mut fixture = Fixture::new();
        let token = fixture.descriptors()[0].token;
        for (ordinal, resources) in [0x6000_usize, 0x7000].into_iter().enumerate() {
            for (pool, offset) in POOLS.into_iter().enumerate() {
                let data = 0x10000 + ordinal * 0x2000 + pool * 0x80;
                fixture.array(resources + offset + 8, data, 3);
                fixture.put(data + 32, &[0; 16]);
                fixture.u64(data + 32, 1_000 + pool as u64);
                fixture.u32(data + 40, token);
            }
        }
        let descriptor = |id| cstrategic_resource::Row {
            id,
            token,
            ..Default::default()
        };
        let descriptors = [descriptor(1), descriptor(2)];
        let mut rows = vec![Row::default(); 4];
        assert_eq!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Rows { len: 4 }
        );
        assert_eq!(
            (rows[0].country, rows[0].resource, rows[0].produced),
            (7, 1, 4_294_967_297)
        );
        assert_eq!(
            (rows[1].country, rows[1].resource, rows[1].produced),
            (7, 2, 1_000)
        );
        assert_eq!((rows[2].country, rows[3].resource), (2, 2));
        assert_eq!(
            walk(
                &fixture,
                0,
                0x1000,
                &[descriptor(1), descriptor(2), descriptor(3)],
                &mut vec![Row::default(); 6]
            ),
            Outcome::Unavailable {
                reason: "resource pool count disagrees with database or capacity"
            }
        );
        fixture.bytes.remove(&0x10020);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Unavailable {
                reason: "resource value record unreadable"
            }
        );
    }

    #[test]
    fn pool_tokens_headers_and_resource_census_cannot_silently_shift_values() {
        let mut fixture = Fixture::new();
        let descriptors = fixture.descriptors();
        let mut rows = vec![Row::default(); 2];
        fixture.u32(0x10018, 20);
        assert!(matches!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Unavailable { .. }
        ));
        fixture.u32(0x10018, 19);
        fixture.u32(0x6000 + PRODUCED_POOL + 0x14, u32::MAX);
        assert!(matches!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            walk(&fixture, 0, 0x1000, &[], &mut rows),
            Outcome::Unavailable { .. }
        ));
    }
}
