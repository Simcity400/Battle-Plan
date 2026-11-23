use super::*;

pub fn verify_country_occurrences<M: Memory>(
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut [ResolvedLink],
    items: &[CountryOccurrence],
) -> bool {
    let mut resolver = Resolver { cells: resolved };
    items
        .iter()
        .all(|item| carrier::country_slot(countries, &mut resolver, item.country) == Ok(item.slot))
}

pub fn state_country_order_demand(
    memory: &impl Memory,
    pointer: usize,
) -> Result<usize, &'static str> {
    Reader { memory, base: 0 }
        .array(pointer, REGISTERED_CONTAINERS)
        .map(|array| array.count as usize)
}

#[allow(clippy::too_many_arguments)]
pub fn state_country_order<M: Memory>(
    memory: &M,
    image: usize,
    pointer: usize,
    source: &Row,
    countries: &ccountry::Reader<'_, M>,
    resolved: &mut [ResolvedLink],
    output: &mut [CountryOccurrence],
    scratch: &mut [u8],
) -> Result<usize, &'static str> {
    let reader = Reader {
        memory,
        base: image,
    };
    let mut identity = [0_u8; 16];
    if !engine_native_memory::read_slice(memory, pointer, &mut identity)
        || engine_sites::at(image, VTABLE_RVA).map(|value| value as u64)
            != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != source.id
        || source.state.is_none()
    {
        return Err("state air-base summary identity mismatch");
    }
    let dense = reader.array(pointer, REGISTERED_CONTAINERS)?;
    let indexed = reader.array(pointer, COUNTRY_CONTAINERS)?;
    let count = dense.count as usize;
    if output.len() < count {
        return Err("state air-base country-order storage unavailable");
    }
    let bytes = count
        .checked_mul(8)
        .ok_or("state air-base dense byte count overflow")?;
    let scratch = scratch
        .get_mut(..bytes)
        .ok_or("state air-base country-order byte storage unavailable")?;
    if bytes != 0 && !engine_native_memory::read_slice(memory, dense.data, scratch) {
        return Err("state air-base dense snapshot unreadable");
    }
    let mut resolver = Resolver { cells: resolved };
    for ordinal in 0..dense.count {
        let at = ordinal as usize * 8;
        let container =
            usize::try_from(u64::from_le_bytes(scratch[at..at + 8].try_into().unwrap()))
                .map_err(|_| "state air-base dense occurrence overflow")?;
        if container == 0 {
            return Err("state air-base dense occurrence is null");
        }
        reader.vtable(container, ccountry_air_container::VTABLE_RVA)?;
        let mut span = [0_u8; CONTAINER_SPAN];
        let country_at = container
            .checked_add(ccountry_air_container::COUNTRY)
            .ok_or("state air-base country address overflow")?;
        if !engine_native_memory::read_slice(memory, country_at, &mut span) {
            return Err("state air-base container country snapshot unreadable");
        }
        let raw = u32::from_le_bytes(span[..4].try_into().unwrap());
        let slot = carrier::country_slot(countries, &mut resolver, raw)?;
        let indexed_address = indexed
            .element(slot, 8)
            .ok_or("state air-base dense country slot is outside its index")?;
        if engine_native_memory::read::<u64>(memory, indexed_address) != Some(container as u64) {
            return Err("state air-base dense country differs from its indexed container");
        }
        output[ordinal as usize] = CountryOccurrence { country: raw, slot };
    }
    Ok(count)
}
