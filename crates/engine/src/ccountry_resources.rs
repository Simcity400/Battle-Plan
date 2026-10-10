//! CCountryResources market and trade state.
//! Resource pools use one-based CStrategicResource indices. Read each pool's
//! data pointer independently; pool values are not inline arrays.

use crate::{
    Memory, ccountry,
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
/// Override entry building type token, i32 at +0, compared with CBuilding+8.
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
    mut output: crate::walk::Buffer<'_, Row>,
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
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let pointer = resources(memory, base, &countries, country)?;
            let pools = pools(memory, base, pointer, count)?;
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
                let mut outside = None;
                'stage: for slot in 0..chunk {
                    for (pool, address) in pools.iter().zip(&mut addresses[slot * 9..slot * 9 + 9])
                    {
                        match pool.element(descriptors[first + slot].id, VALUE_STRIDE) {
                            Some(element) => *address = element,
                            None => {
                                outside = Some(slot);
                                break 'stage;
                            }
                        }
                    }
                }
                let staged = outside.unwrap_or(chunk);
                let completed = memory.copy_records(
                    RecordSources::Pointers(&addresses[..staged * 9]),
                    &VALUE_LAYOUT,
                    &mut records[..staged * 9 * 12],
                );
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
                        let start = (slot * 9 + value) * 12;
                        *pool = cstrategic_resource_pool::from_record(
                            records[start..start + 12].try_into().unwrap(),
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
    let pointer = country
        .pointer
        .checked_add(ccountry::RESOURCES)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country resources pointer unreadable or null")?;
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("country resources vtable mismatch");
    }
    let owner = pointer
        .checked_add(OWNER)
        .and_then(|p| read::<u32>(memory, p))
        .ok_or("country resources owner unreadable")?;
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
    // A single guarded scatter reads the nine independent headers once/country.
    let mut headers = [[0_u8; 24]; 9];
    if memory.copy_scattered(addresses, &mut headers.each_mut().map(|h| h.as_mut_slice())) != 9 {
        return Err("resource pool header unreadable");
    }
    let mut pools = [Array { data: 0, count: 0 }; 9];
    for (pool, header) in pools.iter_mut().zip(headers) {
        *pool = cstrategic_resource_pool::from_header(base, header, count)?;
    }
    Ok(pools)
}

/// Resource descriptors gathered per guarded round; each needs nine records.
const VALUE_CHUNK: usize = 14;
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
