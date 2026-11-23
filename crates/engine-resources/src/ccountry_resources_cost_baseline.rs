use super::*;

const VALUE_CHUNK: usize = 14;

pub(super) fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
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

fn resources<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::RESOURCES)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("country resources pointer unreadable or null")?;
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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
