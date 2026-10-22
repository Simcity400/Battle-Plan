//! CCountryResources market and trade state.
//! Resource pools use one-based CStrategicResource indices. Read each pool's
//! data pointer independently; pool values are not inline arrays.

use crate::{
    Memory,
    batch_fallback::{Ahead, Batch, Link},
    ccountry,
    cpdx_array::Array,
    cstrategic_resource, cstrategic_resource_pool,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};

/// Resource-state primary vtable.
pub const VTABLE_RVA: usize = 0x2945cd0;
/// Owner raw country link, u32, used for exporter identity.
pub const OWNER: usize = 0x20;
/// Embedded produced CStrategicResourcePool requiring authentication.
pub const PRODUCED_POOL: usize = 0x28;
/// Produced-pool values pointer.
pub const PRODUCED_VALUES: usize = 0x30;
/// Adjustment-pool container header.
pub const ADJUSTMENT_POOL: usize = 0xd8;
/// Adjustment-values pointer to i64 entries.
pub const ADJUSTMENT_VALUES: usize = 0xe0;
/// Adjustment count, i32.
pub const ADJUSTMENT_COUNT: usize = 0xec;
/// Embedded imported CStrategicResourcePool requiring authentication.
pub const IMPORTED_POOL: usize = 0x188;
/// Imported-pool values pointer.
pub const IMPORTED_VALUES: usize = 0x190;
/// Embedded used CStrategicResourcePool requiring authentication.
pub const USED_POOL: usize = 0x238;
/// Special-used values pointer.
pub const SPECIAL_USED_VALUES: usize = 0x240;
/// Special-total values pointer.
pub const SPECIAL_TOTAL_VALUES: usize = 0x2f0;
/// Production-line values pointer.
pub const PRODUCTION_LINE_VALUES: usize = 0x3a0;
/// Embedded exportable CStrategicResourcePool requiring authentication.
pub const TO_EXPORT_POOL: usize = 0x450;
/// Exportable-pool values pointer.
pub const TO_EXPORT_VALUES: usize = 0x458;
/// Embedded base-export CStrategicResourcePool requiring authentication.
pub const BASE_EXPORT_POOL: usize = 0x500;
/// Base-export values pointer.
pub const BASE_EXPORT_VALUES: usize = 0x508;
/// Embedded exported CStrategicResourcePool requiring authentication.
pub const EXPORTED_POOL: usize = 0x5b0;
/// Resource-origin array, separate from exchange buckets.
pub const RESOURCE_ORIGINS: usize = 0x710;
/// Building resource overrides used for the state controller.
pub const BUILDING_RESOURCE_OVERRIDES: usize = 0x7a0;
/// Override entry stride.
pub const OVERRIDE_STRIDE: usize = 0x20;
/// Override entry building type token, i32, compared with the building's type.
pub const OVERRIDE_BUILDING: usize = 0x00;
/// Override resource array; each entry holds i32 resource index and i64
/// fixed5 quantity. A nonzero count establishes an override.
pub const OVERRIDE_RESOURCES: usize = 0x08;
/// Override resource-element stride.
pub const OVERRIDE_RESOURCE_STRIDE: usize = 0x10;
/// Per-resource exchange buckets.
pub const EXPORT_BUCKETS: usize = 0x728;
/// Exchange-bucket stride.
pub const EXPORT_BUCKET_STRIDE: usize = 0x18;
/// Import exchange pointer vector.
pub const IMPORT_ROUTES: usize = 0x740;
/// Import exchange count, i32.
pub const IMPORT_ROUTE_COUNT: usize = 0x74c;
/// Delivery-route pointer array selected by exchange queries.
pub const DELIVERY_ROUTES: usize = 0x788;
/// Delivery-route count, i32.
pub const DELIVERY_ROUTE_COUNT: usize = 0x794;

/// Second specialized demand pool.
pub const SPECIAL_TOTAL_POOL: usize = 0x2e8;
/// Third specialized demand pool supplying production demand.
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

/// Read pools using this hour's complete authenticated descriptor census.
/// The caller invokes cstrategic_resource::walk and requires Rows before this
/// reader. Resource keys preserve database indices, independently of pointers
/// or process token numbers. Advisory headers provide sizing only; captures
/// authenticate original headers and grow again before publication.
pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<[usize; 3], &'static str> {
    storage_demand_with_country(memory, base, state, |_, _| Ok(()))
}

/// Share this existing sizing census with dependent holder storage.
pub fn storage_demand_with_country(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut visit: impl FnMut(ccountry::Country, usize) -> Result<(), &'static str>,
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
    output: &mut [Row],
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        descriptors,
        crate::walk::Buffer::Fixed(output),
    )
}

pub fn walk_growing(
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
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_buffer_with_country(memory, base, game_state, descriptors, output, |_, _| {})
}

/// Visit each authenticated country/resource holder inside this census. The
/// caller demand-filters any extra operands; no independent roster is walked.
pub fn walk_buffer_with_country(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    mut output: crate::walk::Buffer<'_, Row>,
    mut visit: impl FnMut(ccountry::Country, usize),
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
            // The next slot's census entry and resources link are read while
            // this country's values are, its resources object's identity and
            // pool headers hinted; each is used at its own place.
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
            // Descriptors that fit are read; the rest are counted unread
            // without any access, exactly as the per-descriptor loop did.
            output.ensure(
                len.checked_add(descriptors.len())
                    .ok_or("country resource row count overflow")?,
            )?;
            let stored = descriptors.len().min(output.len() - len);
            let mut first = 0;
            while first < stored {
                let chunk = (stored - first).min(VALUE_CHUNK);
                // The census is consecutive from id 1, so the chunk's records
                // are one block in each pool: nine spans for the chunk. A
                // block that does not read whole is read again record by
                // record, as the serial walk reads them.
                let whole = value_blocks(memory, &pools, descriptors[first].id, chunk, &mut blocks);
                let mut outside = None;
                let mut completed = chunk * 9;
                if !whole {
                    crate::batch_fallback::record(Batch::CountryResources);
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
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    resources_linked(memory, base, countries, country, Link::Unread)
}

/// [`resources`] with the country's resources link (`CCountry+RESOURCES`)
/// as the walk holds it ([`Link`]).
fn resources_linked<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
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
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    // The vtable and owner, read back to back: one guarded gather of the
    // object, the first field it did not complete failing as its read did.
    const FIELDS: [RecordField; 2] = crate::memory::packed_fields([(0, 8), (OWNER, 4)]);
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
        // Unreachable for a readable vtable; the reads it replaces.
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
    // Vtable, data pointer and two signed header words are exactly 24 bytes.
    // A single guarded scatter reads the nine independent headers once per
    // country, after hinting all their lines so the misses overlap; the hint
    // observes nothing and the copies are the scatter's, in order.
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

/// Reads the records of `chunk` consecutive resources from `first` in each
/// pool as one block per pool, into `blocks` at the pool stride; whether
/// all nine read whole.
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

/// The serial reads of `descriptors`' records, nine per descriptor in pool
/// order, into `records`: the first descriptor outside a pool, and how many
/// records read.
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

/// Resource descriptors read per round: one block per pool, or nine records
/// each when a block does not read whole.
const VALUE_CHUNK: usize = 32;
const VALUE_STRIDE: usize = cstrategic_resource_pool::VALUE_STRIDE;
/// One pool value record. Nine of them, one per pool, describe one resource.
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

/// Synthetic images of `countries` countries, each with a resources object
/// whose nine pools hold `resources` values, for the gather test (offset
/// image) and the cost bench (real addresses).
#[cfg(test)]
pub(crate) mod scale_support {
    use super::*;
    use crate::ccountry::synthetic::World;

    pub(crate) trait Image {
        fn alloc(&mut self, len: usize) -> usize;
        fn u64(&mut self, address: usize, value: u64);
        fn u32(&mut self, address: usize, value: u32);
        fn array(&mut self, header: usize, data: usize, count: u32);
        fn vtable(&self, rva: usize) -> u64;
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
        fn vtable(&self, rva: usize) -> u64 {
            (self.module + rva) as u64
        }
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    impl Image for crate::crelation_status::bench::Arena {
        fn alloc(&mut self, len: usize) -> usize {
            crate::crelation_status::bench::Arena::alloc(self, len)
        }
        fn u64(&mut self, address: usize, value: u64) {
            crate::crelation_status::bench::Arena::u64(self, address, value);
        }
        fn u32(&mut self, address: usize, value: u32) {
            crate::crelation_status::bench::Arena::u32(self, address, value);
        }
        fn array(&mut self, header: usize, data: usize, count: u32) {
            self.vector(header, data, count);
        }
        fn vtable(&self, rva: usize) -> u64 {
            self.rva(rva)
        }
    }

    /// The descriptors of `resources` resources, tokens 100 onwards.
    pub(crate) fn descriptors(resources: u32) -> Vec<cstrategic_resource::Row> {
        (1..=resources)
            .map(|id| cstrategic_resource::Row {
                id,
                token: 99 + id,
                ..Default::default()
            })
            .collect()
    }

    /// Gives country `objects[slot]` (slots 1.., `objects[0]` unused) a
    /// resources object owned by its id, with nine pools of `resources`
    /// values each.
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

    /// An offset image read through `SliceMemory`, module base 0.
    pub(crate) fn resource_world(countries: u32, resources: u32) -> World {
        let mut w = World::new(0, countries, ccountry::RESOURCES + 8);
        let objects: Vec<usize> = (0..=countries).map(|slot| w.object(slot)).collect();
        populate(&mut w, &objects, resources);
        w
    }

    /// A real-address image for `LiveMemory`: the arena, whose base is the
    /// module base, and the game state.
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    pub(crate) fn resource_arena(
        countries: usize,
        resources: u32,
    ) -> (crate::crelation_status::bench::Arena, usize) {
        use crate::crelation_status::bench::{Arena, countries as census};
        let object = ccountry::RESOURCES + 8;
        let pools = 9 * ((resources as usize + 1) * VALUE_STRIDE + 16);
        let per_country = 12 + object + 16 + 0x800 + 16 + pools;
        let mut arena = Arena::new(0x10_0000 + (countries + 1) * per_country);
        let (game_state, objects) = census(&mut arena, countries, object);
        populate(&mut arena, &objects, resources);
        (arena, game_state)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::{
        cgame_state, cresource_exchange as trade, cstrategic_resource_database as database,
    };
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
                f.u64(country, ccountry::VTABLE_RVA as u64);
                f.u32(country + ccountry::ID, raw);
                f.u64(country + ccountry::RESOURCES, resources as u64);
                f.u64(resources, VTABLE_RVA as u64);
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
                        cstrategic_resource_pool::VTABLE_RVA as u64,
                    );
                    f.array(resources + offset + 8, data, 2);
                    f.put(data, &[0; 32]);
                    f.u64(data + 16, (4_294_967_297_i64 + pool as i64) as u64);
                    f.u32(data + 24, 19);
                }
            }
            f.put(0xa000, &[0; trade::SIZE]);
            f.u64(0xa000, trade::VTABLE_RVA as u64);
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
            f.u64(0xd000, cstrategic_resource::VTABLE_RVA as u64);
            f.u32(0xd000 + cstrategic_resource::INDEX, 1);
            f.u32(0xd000 + cstrategic_resource::TOKEN_ID, 19);
            f.put(0xd000 + cstrategic_resource::ACTIVE, &[1]);
            f.u64(0xd000 + cstrategic_resource::FACTORY_COST, 4_294_967_300);
            f.u64(database::SINGLETON_RVA, 0xe000);
            f.array(0xe000 + database::ITEMS, 0xe100, 2);
            f.u64(0xe100, 0xdf00);
            f.u64(0xe108, 0xd000);
            f.array(0xe000 + database::KEYS, 0xe200, 1);
            f.u32(0xe200, 19);
            f
        }
        pub fn descriptors(&self) -> [cstrategic_resource::Row; 1] {
            let mut rows = [cstrategic_resource::Row::default(); 1];
            let result = cstrategic_resource::walk(self, 0, &mut rows);
            assert_eq!(result, Outcome::Rows { len: 1 });
            rows
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Fixture;

    /// Country `slot`'s record of resource `id` in pool `pool` of
    /// [`super::scale_support::resource_world`].
    fn record(w: &crate::ccountry::synthetic::World, slot: u32, pool: usize, id: u32) -> usize {
        let memory = w.memory();
        let object = crate::read::<u64>(&memory, w.object(slot) + crate::ccountry::RESOURCES)
            .unwrap() as usize;
        let data = crate::read::<u64>(&memory, object + POOLS[pool] + 8).unwrap() as usize;
        data + id as usize * VALUE_STRIDE
    }

    /// The batched walk over `memory` and the serial walk over `serial`
    /// (each its own source, so a one-shot fault meets each walk) into
    /// `rows` rows: the batched walk's result, then the serial walk's.
    fn both(
        memory: &impl Memory,
        serial_memory: &impl Memory,
        w: &crate::ccountry::synthetic::World,
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
            crate::walk::Buffer::Fixed(&mut serial),
        );
        [(new, batched), (old, serial)]
    }

    /// On healthy memory the batched walk returns the serial walk's rows and
    /// outcome, truncated or not, reading each pool's
    /// values as one span: 9 value spans per country instead of 63.
    #[test]
    fn batched_values_match_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        use crate::memory::{CountedMemory, ReadCounts};
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
            crate::walk::Buffer::Fixed(&mut serial),
        );
        let old_reads = ReadCounts::take_current();
        prior.add_current();
        assert_eq!(new, Outcome::Rows { len: rows });
        assert_eq!((new, &batched), (old, &serial));
        let n = u64::from(COUNT);
        // The owner gather replaces two copies, the nine blocks replace the
        // values' record gather and are one scatter.
        assert_eq!(new_reads.copy_calls + 2 * n, old_reads.copy_calls);
        assert_eq!(new_reads.record_calls, old_reads.record_calls);
        assert_eq!(new_reads.scattered_calls, old_reads.scattered_calls + n);
        assert_eq!(new_reads.spans + 54 * n, old_reads.spans);
        // A buffer that holds part of a country's rows truncates both alike.
        let [batched, serial] = both(&w.memory(), &w.memory(), &w, &descriptors, rows - 3);
        assert_eq!(batched, serial);
        assert!(matches!(
            batched.0,
            Outcome::Truncated { unread_rows: 3, .. }
        ));
    }

    /// The faults the equality tests place: a value record's padding, which
    /// the serial walk never reads, a value record, the resources link of a
    /// country read ahead and one of its pool headers.
    fn fault_sites(w: &crate::ccountry::synthetic::World) -> [std::ops::Range<usize>; 4] {
        let padding = record(w, 3, 4, 2) + 12;
        let value = record(w, 3, 4, 5);
        let link = w.object(4) + crate::ccountry::RESOURCES;
        let object = crate::read::<u64>(&w.memory(), link).unwrap() as usize;
        let header = object + POOLS[6];
        [
            padding..padding + 4,
            value..value + 12,
            link..link + 8,
            header..header + 24,
        ]
    }

    /// This thread's CountryResources fallbacks.
    fn fallbacks() -> u64 {
        crate::batch_fallback::count(crate::batch_fallback::Batch::CountryResources)
    }

    /// A persistent fault gives the serial walk's result: a block it breaks
    /// is read again record by record and a link whose read ahead failed is
    /// read again at its turn, each once and counted once.
    #[test]
    fn a_persistent_fault_gives_the_serial_result() {
        use super::scale_support::{descriptors, resource_world};
        let w = resource_world(6, 7);
        let descriptors = descriptors(7);
        for (site, (range, counted)) in fault_sites(&w).into_iter().zip([1, 1, 1, 0]).enumerate() {
            let before = fallbacks();
            let memory = crate::batch_fallback::Faulty::new(w.memory(), vec![range], false);
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

    /// A one-shot fault never refuses more than the serial walk: a block it
    /// breaks reads again record by record, and a link read ahead that
    /// faulted is read again at its turn, each counted once.
    #[test]
    fn a_one_shot_fault_refuses_no_more_than_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        let w = resource_world(6, 7);
        let descriptors = descriptors(7);
        let [healthy, _] = both(&w.memory(), &w.memory(), &w, &descriptors, 42);
        for (site, (range, counted)) in fault_sites(&w).into_iter().zip([1, 1, 1, 0]).enumerate() {
            let once = || crate::batch_fallback::Faulty::new(w.memory(), vec![range.clone()], true);
            let before = fallbacks();
            let [batched, serial] = both(&once(), &once(), &w, &descriptors, 42);
            assert_eq!(fallbacks() - before, counted, "site {site}");
            if site < 3 {
                // Read again after the fault: the healthy rows, where the
                // serial walk refuses the value and the link.
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

    /// With 33 resources a country's values are two blocks per pool: the
    /// batched walk matches the serial walk at every capacity around the
    /// block edge, a fault in the second block falls back for that block
    /// only, and a value past the stored rows is never read.
    #[test]
    fn batched_values_cross_the_block_edge_as_the_serial_walk() {
        use super::scale_support::{descriptors, resource_world};
        use crate::batch_fallback::Faulty;
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
        // Country 3 stores five rows; its twentieth value is never read.
        let unread = record(&w, 3, 0, 20);
        let memory = Faulty::new(
            w.memory(),
            std::iter::once(unread..unread + 12).collect(),
            false,
        );
        let before = fallbacks();
        let [batched, serial] = both(&memory, &memory, &w, &descriptors, 66 + 5);
        assert_eq!(batched, serial);
        assert!(matches!(batched.0, Outcome::Truncated { len: 71, .. }));
        assert_eq!(fallbacks() - before, 0);
    }

    /// A null census slot and an alias slot are skipped as the serial walk
    /// skips them, read ahead or not: a link read ahead past a null slot is
    /// never read ahead, and a link read ahead that faulted once is read
    /// again at its turn and counted.
    #[test]
    fn null_and_alias_slots_are_skipped_as_the_serial_walk_skips_them() {
        use super::scale_support::{descriptors, resource_world};
        use crate::batch_fallback::Faulty;
        let mut w = resource_world(8, 7);
        let descriptors = descriptors(7);
        let table = crate::ccountry::synthetic::START + 0x1000;
        let links = table + 9 * 8;
        // Slot 3 is null and slot 5 an alias of country 4.
        w.u64(table + 3 * 8, 0);
        w.u64(table + 5 * 8, w.object(4) as u64);
        w.u32(links + 5 * 4, 4);
        let rows = 6 * 7;
        let [healthy, serial] = both(&w.memory(), &w.memory(), &w, &descriptors, rows);
        assert_eq!(healthy, serial);
        assert_eq!(healthy.0, Outcome::Rows { len: rows });
        let link = |slot: u32| {
            let at = w.object(slot) + crate::ccountry::RESOURCES;
            at..at + 8
        };
        // Country 2's link is read ahead at country 1's turn; country 4's is
        // not, since slot 3 is null.
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

    #[test]
    #[ignore = "synthetic acquisition cost; release, affinity 0xF"]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn acquisition_cost() {
        use super::scale_support::{descriptors, resource_arena};
        use crate::memory::{CountedMemory, LiveMemory, ReadCounts};
        let countries = std::env::var("BATTLE_PLAN_RESOURCE_COST_COUNTRIES").or_else(|_| std::env::var("HOI4_RESOURCE_COST_COUNTRIES"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_usize);
        let resources = 7;
        let (arena, state) = resource_arena(countries, resources);
        let base = arena.base();
        let descriptors = descriptors(resources);
        // SAFETY: the arena stays allocated and unchanged while read.
        let memory = CountedMemory::new(unsafe { LiveMemory::new() });
        let mut rows = vec![super::Row::default(); countries * resources as usize];
        for (label, gathered) in [("serial", false), ("batched", true)] {
            let mut samples = Vec::new();
            let mut reads = ReadCounts::ZERO;
            for _ in 0..40 {
                ReadCounts::take_current();
                let start = std::time::Instant::now();
                let walk = if gathered {
                    super::walk(&memory, base, state, &descriptors, &mut rows)
                } else {
                    super::baseline::walk_buffer(
                        &memory,
                        base,
                        state,
                        &descriptors,
                        crate::walk::Buffer::Fixed(&mut rows),
                    )
                };
                samples.push(start.elapsed().as_nanos() as f64);
                reads = ReadCounts::take_current();
                assert_eq!(walk, Outcome::Rows { len: rows.len() });
                std::hint::black_box(&rows);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "country_resources {label} countries={countries} median_ns_per_country={:.1} copy_calls={} scattered_calls={} record_calls={} spans={}",
                samples[20] / countries as f64,
                reads.copy_calls,
                reads.scattered_calls,
                reads.record_calls,
                reads.spans
            );
        }
    }
    use super::*;

    #[test]
    fn resource_pool_rows_preserve_raw_widths_and_alias_country_identity() {
        let mut fixture = Fixture::new();
        let descriptors = fixture.descriptors();
        fixture.u64(0x10090, (-4_294_967_297_i64) as u64);
        let mut rows = [Row::default(); 2];
        assert_eq!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!((rows[0].country, rows[0].resource), (7, 1));
        assert_eq!(rows[0].produced, 4_294_967_297);
        assert_eq!(rows[0].adjustment, -4_294_967_297);
        assert_eq!(rows[0].exported, 4_294_967_305);
        assert_eq!(rows[1].country, 2);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &descriptors, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    /// Several descriptors share one gather, so a fault in a later descriptor
    /// must report its own reason at its own position while the earlier ones
    /// still decode, and a descriptor outside its pool keeps its own reason.
    #[test]
    fn a_mid_batch_descriptor_fault_keeps_its_reason_and_position() {
        let mut fixture = Fixture::new();
        let token = fixture.descriptors()[0].token;
        // Give every pool a third slot so a second descriptor lands inside it.
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
        let mut rows = [Row::default(); 4];
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
        // A descriptor beyond its pool cannot be reached through `walk`: the
        // pool header check already rejects a census wider than the array.
        assert_eq!(
            walk(
                &fixture,
                0,
                0x1000,
                &[descriptor(1), descriptor(2), descriptor(3)],
                &mut [Row::default(); 6]
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
        let mut rows = [Row::default(); 2];
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
