
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::ccountry;
use crate::{ccountry_resources, cstrategic_resource};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29d8e60);
pub const SIZE: usize = 0xf0;
pub const CONVOYS_ASSIGNED: usize = 0x28;
pub const CONVOYS_REQUIRED: usize = 0x2c;
pub const EFFICIENCY_A: usize = 0x48;
pub const EFFICIENCY_B: usize = 0x50;
pub const EXPORTER_RESOURCES: usize = 0x80;
pub const IMPORTER: usize = 0x88;
pub const IMPORTER_INDEX_CANDIDATE: usize = 0x8c;
pub const RESOURCE: usize = 0x90;
pub const DELIVERED: usize = 0x98;
pub const REQUIRED_FACTORIES: usize = 0xe0;
pub const ASSIGNED_FACTORIES: usize = 0xe4;
pub const NO_TRADE_INFLUENCE: usize = 0xe8;
pub const NO_RESOURCES: usize = 0xe9;
pub const CONSTRUCTOR_RVA: Rva = Rva(0xd05480);

pub const REFERENCE: usize = 0x08;
pub const REGISTERED: usize = 0x10;
pub const OWNER: usize = 0x18;

#[derive(Debug, Clone, Copy)]
pub struct Ends {
    pub exporter_resources: usize,
    pub exporter: u32,
    pub importer: u32,
    pub exporter_country: engine_native_memory::country_identity::Country,
}

pub fn authenticate<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    pointer: usize,
    importer: engine_native_memory::country_identity::Country,
) -> Result<([u8; SIZE], Ends), &'static str> {
    let bytes = read::<[u8; SIZE]>(memory, pointer).ok_or("trade exchange unreadable")?;
    let u32_at = |offset: usize| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("fixed field width"),
        )
    };
    let u64_at = |offset: usize| {
        u64::from_le_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .expect("fixed field width"),
        )
    };
    if u64_at(0) != engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64 {
        return Err("trade exchange vtable mismatch");
    }
    if bytes[REGISTERED] == 0 {
        return Err("trade exchange has no registered native handle");
    }
    let exporter_resources = usize::try_from(u64_at(EXPORTER_RESOURCES))
        .ok()
        .filter(|&p| p != 0)
        .ok_or("trade exporter resources pointer invalid")?;
    let exporter = exporter_resources
        .checked_add(ccountry_resources::OWNER)
        .and_then(|p| read::<u32>(memory, p))
        .ok_or("trade exporter unreadable")?;
    let exporter_country = countries.resolve(exporter)?;
    if ccountry_resources::resources(memory, base, countries, exporter_country)?
        != exporter_resources
    {
        return Err("trade exporter resources backlink mismatch");
    }
    let raw_importer = u32_at(IMPORTER);
    let importer_country = countries.resolve(raw_importer)?;
    if countries.resolve(u32_at(OWNER))?.pointer != importer_country.pointer {
        return Err("trade convoy owner disagrees with importer");
    }
    if importer_country.pointer != importer.pointer {
        return Err("trade source country mismatch");
    }
    if exporter_country.pointer == importer_country.pointer {
        return Err("trade endpoints are the same country");
    }
    Ok((
        bytes,
        Ends {
            exporter_resources,
            exporter,
            importer: raw_importer,
            exporter_country,
        },
    ))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub exporter: u32,
    pub importer: u32,
    pub resource: u32,
    pub import_ordinal: u32,
    pub export_ordinal: u32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub efficiency_a: i64,
    pub efficiency_b: i64,
    pub delivered: i64,
    pub required_factories: i32,
    pub assigned_factories: i32,
    pub no_trade_influence: u8,
    pub no_resources: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Membership {
    pointer: usize,
    counterpart_resources: usize,
    resource: u32,
    output_index: usize,
    ordinal: Option<u32>,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    output: &mut Vec<Row>,
    scratch: &mut Vec<Membership>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        descriptors,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(scratch),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut scratch: engine_native_memory::walk::Buffer<'_, Membership>,
) -> Outcome {
    let result = (|| {
        for (index, resource) in descriptors.iter().enumerate() {
            if resource.id != index as u32 + 1 || resource.address == 0 {
                return Err("trade resource descriptor census is incomplete");
            }
        }
        let reader = Reader {
            memory,
            base,
            countries: ccountry::Reader::new(memory, base, game_state)?,
            descriptors,
        };
        let mut found_country = false;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..reader.countries.count() {
            let Some(country) = reader.countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let resources =
                ccountry_resources::resources(memory, base, &reader.countries, country)?;
            let source = reader.array(resources, ccountry_resources::IMPORT_ROUTES, 8)?;
            output.ensure(
                len.checked_add(source.count as usize)
                    .ok_or("trade row count overflow")?,
            )?;
            scratch.ensure(
                len.checked_add(source.count as usize)
                    .ok_or("trade membership count overflow")?,
            )?;
            let capacity = output.len().min(scratch.len());
            for ordinal in 0..source.count {
                let pointer = reader.pointer(source, ordinal)?;
                if pointer == 0 {
                    return Err("null trade exchange in live vector");
                }
                if len == capacity {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("trade unread row count overflow")?;
                    continue;
                }
                let (row, mut membership) = reader.row(pointer, country, ordinal)?;
                membership.output_index = len;
                output[len] = row;
                scratch[len] = membership;
                len += 1;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        let scratch = &mut scratch[..len];
        scratch.sort_unstable_by_key(|entry| output[entry.output_index].id);
        if scratch
            .windows(2)
            .any(|pair| output[pair[0].output_index].id == output[pair[1].output_index].id)
        {
            return Err("duplicate trade exchange handle");
        }
        reader.reciprocals(scratch, &mut output[..len])?;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    countries: ccountry::Reader<'a, M>,
    descriptors: &'a [cstrategic_resource::Row],
}

impl<M: Memory> Reader<'_, M> {
    fn array_at(&self, pointer: usize, stride: usize) -> Result<Array, &'static str> {
        Array::read(self.memory, pointer, stride).ok_or("trade vector unreadable or invalid")
    }

    fn array(&self, pointer: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
        self.array_at(
            pointer
                .checked_add(offset)
                .ok_or("trade vector address overflow")?,
            stride,
        )
    }

    fn pointer(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|p| read::<u64>(self.memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("trade pointer unreadable")
    }

    fn buckets(&self, resources: usize) -> Result<Array, &'static str> {
        let buckets = self.array(
            resources,
            ccountry_resources::EXPORT_BUCKETS,
            ccountry_resources::EXPORT_BUCKET_STRIDE,
        )?;
        if buckets.count as usize != self.descriptors.len() + 1 {
            return Err("trade buckets disagree with resource database count");
        }
        Ok(buckets)
    }

    fn reciprocals(
        &self,
        scratch: &mut [Membership],
        output: &mut [Row],
    ) -> Result<(), &'static str> {
        let key = |entry: &Membership| (entry.counterpart_resources, entry.resource);
        scratch.sort_unstable_by_key(|entry| (key(entry), entry.pointer));
        let mut start = 0;
        while start < scratch.len() {
            let group_key = key(&scratch[start]);
            let count = scratch[start..].partition_point(|entry| key(entry) == group_key);
            let group = &mut scratch[start..start + count];
            let buckets = self.buckets(group_key.0)?;
            let source = self.array_at(
                buckets
                    .element(group_key.1, ccountry_resources::EXPORT_BUCKET_STRIDE)
                    .ok_or("trade resource bucket missing")?,
                8,
            )?;
            for ordinal in 0..source.count {
                let pointer = self.pointer(source, ordinal)?;
                if let Ok(index) = group.binary_search_by_key(&pointer, |entry| entry.pointer)
                    && group[index].ordinal.replace(ordinal).is_some()
                {
                    return Err("duplicate trade exchange in counterpart vector");
                }
            }
            for entry in group {
                let ordinal = entry
                    .ordinal
                    .ok_or("trade exchange absent from counterpart vector")?;
                output[entry.output_index].export_ordinal = ordinal;
            }
            start += count;
        }
        Ok(())
    }

    fn row(
        &self,
        pointer: usize,
        country: engine_native_memory::country_identity::Country,
        ordinal: u32,
    ) -> Result<(Row, Membership), &'static str> {
        let (bytes, ends) =
            authenticate(self.memory, self.base, &self.countries, pointer, country)?;
        let u32_at = |offset| {
            u32::from_le_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("fixed field width"),
            )
        };
        let u64_at = |offset| {
            u64::from_le_bytes(
                bytes[offset..offset + 8]
                    .try_into()
                    .expect("fixed field width"),
            )
        };
        let (exporter, importer, exporter_resources) =
            (ends.exporter, ends.importer, ends.exporter_resources);
        let descriptor = self
            .descriptors
            .iter()
            .find(|r| r.address as u64 == u64_at(RESOURCE))
            .ok_or("trade resource descriptor absent from database")?;
        Ok((
            Row {
                id: u64_at(REFERENCE),
                exporter,
                importer,
                resource: descriptor.id,
                import_ordinal: ordinal,
                export_ordinal: 0,
                convoys_assigned: u32_at(CONVOYS_ASSIGNED) as i32,
                convoys_required: u32_at(CONVOYS_REQUIRED) as i32,
                efficiency_a: u64_at(EFFICIENCY_A) as i64,
                efficiency_b: u64_at(EFFICIENCY_B) as i64,
                delivered: u64_at(DELIVERED) as i64,
                required_factories: u32_at(REQUIRED_FACTORIES) as i32,
                assigned_factories: u32_at(ASSIGNED_FACTORIES) as i32,
                no_trade_influence: bytes[NO_TRADE_INFLUENCE],
                no_resources: bytes[NO_RESOURCES],
            },
            Membership {
                pointer,
                counterpart_resources: exporter_resources,
                resource: descriptor.id,
                output_index: 0,
                ordinal: None,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccountry_resources::test_support::Fixture;
    use std::cell::RefCell;

    #[test]
    fn importer_census_preserves_native_handles_fields_and_capacity() {
        let mut fixture = Fixture::new();
        fixture.u32(0xa000 + CONVOYS_ASSIGNED, i32::MIN as u32);
        fixture.u32(0xa000 + CONVOYS_REQUIRED, i32::MAX as u32);
        fixture.u64(0xa000 + EFFICIENCY_A, i64::MIN as u64);
        fixture.u64(0xa000 + EFFICIENCY_B, i64::MAX as u64);
        fixture.put(0xa000 + NO_TRADE_INFLUENCE, &[0xff, 0x80]);
        let resources = fixture.descriptors();
        let mut rows = vec![Row::default(); 1];
        let mut scratch = vec![Membership::default(); 1];
        assert_eq!(
            walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows,
            [Row {
                id: 0x1234_5678_0000_0033,
                exporter: 7,
                importer: 2,
                resource: 1,
                import_ordinal: 0,
                export_ordinal: 0,
                convoys_assigned: i32::MIN,
                convoys_required: i32::MAX,
                efficiency_a: i64::MIN,
                efficiency_b: i64::MAX,
                delivered: 4_294_967_299,
                required_factories: 9,
                assigned_factories: 8,
                no_trade_influence: 0xff,
                no_resources: 0x80,
            }]
        );
    }

    #[test]
    fn missing_or_duplicated_reciprocal_trade_links_are_unavailable() {
        let mut fixture = Fixture::new();
        let resources = fixture.descriptors();
        let mut rows = vec![Row::default(); 2];
        let mut scratch = vec![Membership::default(); 2];
        fixture.array(0xc000 + ccountry_resources::EXPORT_BUCKET_STRIDE, 0, 0);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Unavailable {
                reason: "trade exchange absent from counterpart vector"
            }
        );
        fixture.array(0xc000 + ccountry_resources::EXPORT_BUCKET_STRIDE, 0xc100, 2);
        fixture.u64(0xc108, 0xa000);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Unavailable {
                reason: "duplicate trade exchange in counterpart vector"
            }
        );
        fixture.array(0xc000 + ccountry_resources::EXPORT_BUCKET_STRIDE, 0xc100, 1);
        fixture.put(0xa000 + REGISTERED, &[0]);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Unavailable {
                reason: "trade exchange has no registered native handle"
            }
        );
    }

    const COUNT: usize = 64;
    const IMPORTS: usize = 0x20000;
    const EXPORTS: usize = 0x21000;
    const OBJECTS: usize = 0x30000;

    fn high_degree_fixture() -> Fixture {
        let mut fixture = Fixture::new();
        let mut template = [0_u8; SIZE];
        assert!(fixture.copy(0xa000, &mut template));
        fixture.array(
            0x7000 + ccountry_resources::IMPORT_ROUTES,
            IMPORTS,
            COUNT as u32,
        );
        fixture.array(
            0xc000 + ccountry_resources::EXPORT_BUCKET_STRIDE,
            EXPORTS,
            COUNT as u32,
        );
        for index in 0..COUNT {
            let pointer = OBJECTS + index * 0x100;
            fixture.put(pointer, &template);
            fixture.u64(pointer + REFERENCE, ((index as u64 + 1) << 32) | 0x33);
            fixture.u64(pointer + DELIVERED, 4_294_967_299 + index as u64);
            fixture.u64(IMPORTS + index * 8, pointer as u64);
            fixture.u64(EXPORTS + (COUNT - index - 1) * 8, pointer as u64);
        }
        fixture
    }

    struct Counted<'a> {
        memory: &'a Fixture,
        directional_reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }

    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if (IMPORTS..IMPORTS + COUNT * 8).contains(&address)
                || (EXPORTS..EXPORTS + COUNT * 8).contains(&address)
            {
                self.directional_reads
                    .borrow_mut()
                    .push((address, destination.len()));
            }
            self.fault != Some(address) && self.memory.copy(address, destination)
        }
    }

    #[test]
    fn high_degree_trade_preserves_primary_order_and_scans_reciprocals_once() {
        let fixture = high_degree_fixture();
        let resources = fixture.descriptors();
        let memory = Counted {
            memory: &fixture,
            directional_reads: RefCell::new(Vec::new()),
            fault: None,
        };
        let mut rows = vec![Row::default(); COUNT];
        let mut scratch = vec![Membership::default(); COUNT];
        assert_eq!(
            walk(&memory, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Rows { len: COUNT }
        );
        let expected: Vec<_> = [IMPORTS, EXPORTS]
            .into_iter()
            .flat_map(|base| (0..COUNT).map(move |index| (base + index * 8, 8)))
            .collect();
        assert_eq!(*memory.directional_reads.borrow(), expected);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                *row,
                Row {
                    id: ((index as u64 + 1) << 32) | 0x33,
                    exporter: 7,
                    importer: 2,
                    resource: 1,
                    import_ordinal: index as u32,
                    export_ordinal: (COUNT - index - 1) as u32,
                    convoys_assigned: 0,
                    convoys_required: 0,
                    efficiency_a: 100_000,
                    efficiency_b: 99_999,
                    delivered: 4_294_967_299 + index as i64,
                    required_factories: 9,
                    assigned_factories: 8,
                    no_trade_influence: 0,
                    no_resources: 0,
                }
            );
        }
    }

    #[test]
    fn an_exporter_replaced_between_two_trades_is_authenticated_afresh() {
        struct Replaced<'a> {
            memory: &'a Fixture,
            replaced: std::cell::Cell<bool>,
        }
        impl Memory for Replaced<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == OBJECTS + 0x100 {
                    self.replaced.set(true);
                }
                if self.replaced.get() && address == 0x6000 + ccountry_resources::OWNER {
                    destination.copy_from_slice(&2_u32.to_le_bytes());
                    return destination.len() == 4;
                }
                self.memory.copy(address, destination)
            }
        }
        let fixture = high_degree_fixture();
        let resources = fixture.descriptors();
        let memory = Replaced {
            memory: &fixture,
            replaced: std::cell::Cell::new(false),
        };
        let mut rows = vec![Row::default(); COUNT];
        let mut scratch = vec![Membership::default(); COUNT];
        assert_eq!(
            walk(&memory, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Unavailable {
                reason: "trade exporter resources backlink mismatch"
            }
        );
        assert!(memory.replaced.get());
    }

    #[test]
    fn high_degree_trade_rejects_wrong_missing_duplicate_and_unreadable_memberships() {
        let mut rows = vec![Row::default(); COUNT];
        let mut scratch = vec![Membership::default(); COUNT];
        for (address, replacement, reason) in [
            (
                EXPORTS,
                0xdead_beef,
                "trade exchange absent from counterpart vector",
            ),
            (
                EXPORTS,
                OBJECTS as u64,
                "duplicate trade exchange in counterpart vector",
            ),
            (
                OBJECTS + (COUNT - 1) * 0x100 + REFERENCE,
                (1_u64 << 32) | 0x33,
                "duplicate trade exchange handle",
            ),
            (
                OBJECTS + RESOURCE,
                0xdf00,
                "trade resource descriptor absent from database",
            ),
            (
                IMPORTS + (COUNT - 1) * 8,
                OBJECTS as u64,
                "duplicate trade exchange handle",
            ),
        ] {
            let mut fixture = high_degree_fixture();
            let resources = fixture.descriptors();
            fixture.u64(address, replacement);
            assert_eq!(
                walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
                Outcome::Unavailable { reason }
            );
        }
        let fixture = high_degree_fixture();
        let resources = fixture.descriptors();
        for (fault, expected_reads) in [
            (IMPORTS + (COUNT - 1) * 8, COUNT),
            (EXPORTS + (COUNT - 1) * 8, COUNT * 2),
        ] {
            let memory = Counted {
                memory: &fixture,
                directional_reads: RefCell::new(Vec::new()),
                fault: Some(fault),
            };
            assert_eq!(
                walk(&memory, 0, 0x1000, &resources, &mut rows, &mut scratch),
                Outcome::Unavailable {
                    reason: "trade pointer unreadable"
                }
            );
            let trace = memory.directional_reads.borrow();
            assert_eq!(trace.len(), expected_reads);
            assert_eq!(trace.last(), Some(&(fault, 8)));
        }
        let mut fixture = high_degree_fixture();
        fixture.u32(OBJECTS + IMPORTER, 7);
        assert_eq!(
            walk(&fixture, 0, 0x1000, &resources, &mut rows, &mut scratch),
            Outcome::Unavailable {
                reason: "trade convoy owner disagrees with importer"
            }
        );
    }

}
