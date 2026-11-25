use super::*;

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<(Capacity, usize), &'static str> {
    let array = |pointer, offset, stride| {
        Array::read(memory, address(pointer, offset)?, stride)
            .ok_or("raid state sizing array unreadable or invalid")
    };
    let add = |left: usize, right: usize| {
        left.checked_add(right)
            .ok_or("raid state sizing count overflow")
    };
    let mul = |left: usize, right: usize| {
        left.checked_mul(right)
            .ok_or("raid state sizing count overflow")
    };
    let database = read::<u64>(
        memory,
        site(base, cstrategic_resource_database::SINGLETON_RVA)?,
    )
    .and_then(|pointer| usize::try_from(pointer).ok())
    .filter(|&pointer| pointer != 0)
    .ok_or("resource database pointer unreadable or null")?;
    let resource_items = array(database, cstrategic_resource_database::ITEMS, 8)?;
    let resource_keys = array(database, cstrategic_resource_database::KEYS, 4)?;
    if resource_items.count == 0 || resource_keys.count != resource_items.count - 1 {
        return Err("resource database item and key counts disagree");
    }
    let mut demand = Capacity::zero();
    demand.resources = resource_keys.count as usize;
    let states = array(game_state, cgame_state::STATES, 8)?;
    if states.count < 2 {
        return Err("state array unreadable or invalid");
    }
    demand.states = states.count as usize;
    demand.resource_rows = mul(demand.states, demand.resources)?;
    let pool_bytes = mul(demand.resources + 1, cstrategic_resource_pool::VALUE_STRIDE)?;
    let mut core_links = 0;
    let vtable = site(base, cstate::VTABLE_RVA)? as u64;
    engine_native_memory::cpdx_array::pointers(
        memory,
        states.slice(1, states.count - 1, 8)?,
        "state sizing pointer unreadable",
        |index, pointer| {
            if pointer == 0 {
                return Ok(());
            }
            if read::<u64>(memory, pointer) != Some(vtable)
                || read::<u32>(memory, address(pointer, cstate::ID)?) != Some(index + 1)
            {
                return Err("state record disagrees with its census slot");
            }
            let provinces = array(pointer, cstate::PROVINCES, 8)?;
            need(&mut demand.provinces, provinces.count as usize);
            let cores = array(pointer, cstate::CORES, 4)?;
            let claims = array(pointer, cstate::CLAIMS, 4)?;
            core_links = add(
                core_links,
                add(cores.count as usize, claims.count as usize)?,
            )?;
            let locations = array(
                pointer,
                cstate::STRATEGIC_LOCATIONS,
                cstate::STRATEGIC_LOCATION_STRIDE,
            )?;
            core_links = add(core_links, mul(locations.count as usize, 2)?)?;
            for ordinal in 0..provinces.count {
                let province = provinces
                    .element(ordinal, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|&pointer| pointer != 0)
                    .ok_or("state province sizing pointer unreadable or null")?;
                let tokens = array(province, cprovince::STRATEGIC_LOCATIONS, 4)?;
                core_links = add(core_links, mul(tokens.count as usize, 2)?)?;
            }
            let accumulator = array(pointer, cstate::BASE_MODIFIERS, modifier_entry::STRIDE)?;
            let container = address(pointer, cstate::DYNAMIC_MODIFIERS)?;
            if read::<u64>(memory, container)
                != Some(site(base, cdynamic_modifier_container::VTABLE_RVA)? as u64)
                || read::<u64>(
                    memory,
                    address(container, cdynamic_modifier_container::COUNTRY)?,
                ) != Some(0)
                || read::<u32>(
                    memory,
                    address(container, cdynamic_modifier_container::STATE)?,
                ) != Some(index + 1)
            {
                return Err("state dynamic modifier container identity mismatch");
            }
            let modifiers = array(
                container,
                cdynamic_modifier_container::ENTRIES,
                dynamic_modifier_entry::STRIDE,
            )?;
            demand.modifier_rows = add(demand.modifier_rows, modifiers.count as usize)?;
            need(
                &mut demand.batch_bytes,
                add(
                    pool_bytes,
                    add(
                        mul(accumulator.count as usize, modifier_entry::STRIDE)?,
                        mul(modifiers.count as usize, dynamic_modifier_entry::STRIDE)?,
                    )?,
                )?,
            );
            for ordinal in 0..modifiers.count {
                let entry = modifiers
                    .element(ordinal, dynamic_modifier_entry::STRIDE)
                    .ok_or("state modifier sizing entry overflow")?;
                if read::<u64>(memory, entry)
                    != Some(site(base, dynamic_modifier_entry::VTABLE_RVA)? as u64)
                {
                    return Err("state modifier sizing entry vtable mismatch");
                }
                let definition =
                    read::<u64>(memory, address(entry, dynamic_modifier_entry::DEFINITION)?)
                        .and_then(|pointer| usize::try_from(pointer).ok())
                        .filter(|&pointer| pointer != 0)
                        .ok_or("state modifier sizing definition unreadable or null")?;
                let (_, len) = cdynamic_modifier::token(memory, base, definition)?;
                demand.token_bytes = add(demand.token_bytes, len)?;
            }
            Ok(())
        },
    )?;
    demand.definitions = demand.modifier_rows;
    demand.name_bytes = demand.token_bytes;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let [_, links] = countries.arrays();
    demand.countries = (countries.count() as usize).max(links.count as usize);
    let supply_root = csupply_system::read_root(memory, base, game_state)?;
    let companions = array(
        supply_root,
        csupply_system::COUNTRY_COMPANION_DATA,
        supply_companion::STRIDE,
    )?;
    if companions.count != countries.count() {
        return Err("country supply array counts differ from country slots");
    }
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let resources = read::<u64>(memory, address(country.pointer, ccountry::RESOURCES)?)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("country resources pointer unreadable or null")?;
        if read::<u64>(memory, resources)
            != Some(site(base, ccountry_resources::VTABLE_RVA)? as u64)
        {
            return Err("country resources vtable mismatch");
        }
        let owner = read::<u32>(memory, address(resources, ccountry_resources::OWNER)?)
            .ok_or("country resources owner unreadable")?;
        if countries.resolve(owner)?.pointer != country.pointer {
            return Err("country resources owner disagrees with country");
        }
        let origins = array(resources, ccountry_resources::RESOURCE_ORIGINS, 8)?;
        need(&mut demand.origins, origins.count as usize);
        let overrides = array(
            resources,
            ccountry_resources::BUILDING_RESOURCE_OVERRIDES,
            ccountry_resources::OVERRIDE_STRIDE,
        )?;
        need(&mut demand.entries, overrides.count as usize);
        for ordinal in 0..overrides.count {
            let entry = overrides
                .element(ordinal, ccountry_resources::OVERRIDE_STRIDE)
                .ok_or("override sizing entry overflow")?;
            let elements = array(
                entry,
                ccountry_resources::OVERRIDE_RESOURCES,
                ccountry_resources::OVERRIDE_RESOURCE_STRIDE,
            )?;
            demand.override_rows = add(demand.override_rows, (elements.count as usize).max(1))?;
        }
        let companion = companions
            .element(slot, supply_companion::STRIDE)
            .ok_or("country supply companion address overflow")?;
        let routes = array(companion, supply_companion::ROUTES_SUBTYPE_ZERO, 8)?;
        need(&mut demand.routes, routes.count as usize);
    }
    Ok((demand, core_links))
}

pub fn state_census(
    memory: &impl Memory,
    game_state: usize,
    output: &mut Vec<u32>,
) -> std::io::Result<()> {
    output.clear();
    let root = address(game_state, cgame_state::STATES)
        .ok()
        .and_then(|at| Array::read(memory, at, 8))
        .filter(|array| array.count >= 2)
        .ok_or(std::io::ErrorKind::InvalidData)?;
    crate::allocation::reserve(output, root.count as usize - 1)?;
    let mut bytes = [0_u8; 512 * 8];
    let mut first = 1;
    while first < root.count {
        let count = ((root.count - first) as usize).min(512);
        let start = root
            .element(first, 8)
            .ok_or(std::io::ErrorKind::InvalidData)?;
        if !memory.copy(start, &mut bytes[..count * 8]) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        for index in 0..count {
            if u64_at(&bytes, index * 8) != 0 {
                output.push(first + index as u32);
            }
        }
        first += count as u32;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoreRow {
    pub state: u32,
    pub start: usize,
    pub len: usize,
    pub claims_start: usize,
    pub claims_len: usize,
    pub locations_start: usize,
    pub locations_len: usize,
    pub province_locations_start: usize,
    pub province_locations_len: usize,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the census and selection come from the caller, and the rows, links and required count are caller-owned"
)]
pub fn state_cores(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[u32],
    selected: &dyn Fn(u32) -> bool,
    rows: &mut Vec<CoreRow>,
    links: &mut Vec<u32>,
    required: &mut usize,
) -> Outcome {
    state_cores_buffer(
        memory,
        base,
        game_state,
        census,
        selected,
        Buffer::Growing(rows),
        Buffer::Growing(links),
        required,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the census and selection come from the caller, and the row and link buffers and required count are caller-owned"
)]
fn state_cores_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[u32],
    selected: &dyn Fn(u32) -> bool,
    mut rows: Buffer<'_, CoreRow>,
    mut links: Buffer<'_, u32>,
    required: &mut usize,
) -> Outcome {
    const RECORD: usize = 80;
    const FIELDS: [RecordField; 6] = [
        field(0, 0, 8),
        field(cstate::ID, 8, 4),
        field(cstate::CORES, 16, 16),
        field(cstate::CLAIMS, 32, 16),
        field(cstate::STRATEGIC_LOCATIONS, 48, 16),
        field(cstate::PROVINCES, 64, 16),
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, RECORD).expect("valid record layout");
    const PROVINCE_RECORD: usize = 36;
    const PROVINCE_FIELDS: [RecordField; 4] = [
        field(0, 0, 8),
        field(cprovince::STATE, 8, 8),
        field(cprovince::ID, 16, 4),
        field(cprovince::STRATEGIC_LOCATIONS, 20, 16),
    ];
    const PROVINCE_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&PROVINCE_FIELDS, PROVINCE_RECORD).expect("valid record layout");
    *required = 0;
    let result = (|| {
        let root = address(game_state, cgame_state::STATES)
            .ok()
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state array unreadable or invalid")?;
        let vtable = site(base, cstate::VTABLE_RVA)? as u64;
        let province_vtable = site(base, cprovince::VTABLE_RVA)? as u64;
        let (mut len, mut used, mut unread) = (0, 0, 0_u32);
        let mut ids = [0_u32; BATCH];
        let mut ptr_addrs = [0_usize; BATCH];
        let mut ptr_bytes = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut copied = [0_u8; BATCH * RECORD];
        let mut prov_ptr_bytes = [0_u8; BATCH * 8];
        let mut prov_ptrs = [0_usize; BATCH];
        let mut prov_records = [0_u8; BATCH * PROVINCE_RECORD];
        let mut province_tokens: Vec<(u32, Array)> = Vec::new();
        let mut chosen = census.iter().copied().filter(|&state| selected(state));
        loop {
            let mut count = 0;
            for state in chosen.by_ref().take(BATCH) {
                ids[count] = state;
                count += 1;
            }
            if count == 0 {
                break;
            }
            for index in 0..count {
                ptr_addrs[index] = root
                    .element(ids[index], 8)
                    .ok_or("state pointer unreadable")?;
            }
            let copied_ptrs = memory.copy_records(
                RecordSources::Pointers(&ptr_addrs[..count]),
                &ITEM_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            for index in 0..copied_ptrs {
                pointers[index] = usize::try_from(u64_at(&ptr_bytes, index * 8))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("state pointer null")?;
            }
            if copied_ptrs < count {
                return Err("state pointer unreadable");
            }
            records(
                memory,
                &pointers[..count],
                &LAYOUT,
                &mut copied,
                "state core list unreadable",
            )?;
            for index in 0..count {
                let record = &copied[index * RECORD..(index + 1) * RECORD];
                if u64_at(record, 0) != vtable || u32_at(record, 8) != ids[index] {
                    return Err("state record disagrees with its census slot");
                }
                let cores =
                    Array::from_header(header16(record, 16), 4).ok_or("state core list invalid")?;
                let claims = Array::from_header(header16(record, 32), 4)
                    .ok_or("state claim list invalid")?;
                let locations =
                    Array::from_header(header16(record, 48), cstate::STRATEGIC_LOCATION_STRIDE)
                        .ok_or("state strategic location list invalid")?;
                let provinces = Array::from_header(header16(record, 64), 8)
                    .ok_or("state province list invalid")?;
                province_tokens.clear();
                let mut province_count = 0_usize;
                let mut ordinal = 0_u32;
                while ordinal < provinces.count {
                    let chunk = ((provinces.count - ordinal) as usize).min(BATCH);
                    let start = provinces
                        .element(ordinal, 8)
                        .ok_or("state province pointer unreadable or null")?;
                    if !memory.copy(start, &mut prov_ptr_bytes[..chunk * 8]) {
                        return Err("state province pointer unreadable or null");
                    }
                    let mut live = 0;
                    let mut overflow = None;
                    for slot in 0..chunk {
                        let ptr = usize::try_from(u64_at(&prov_ptr_bytes, slot * 8))
                            .ok()
                            .filter(|&pointer| pointer != 0)
                            .ok_or("state province pointer unreadable or null")?;
                        if let Err(reason) = address(ptr, cprovince::STATE)
                            .and_then(|_| address(ptr, cprovince::ID))
                            .and_then(|_| address(ptr, cprovince::STRATEGIC_LOCATIONS))
                        {
                            overflow = Some(reason);
                            break;
                        }
                        prov_ptrs[slot] = ptr;
                        live += 1;
                    }
                    let copied_fields = memory.copy_records(
                        RecordSources::Pointers(&prov_ptrs[..live]),
                        &PROVINCE_LAYOUT,
                        &mut prov_records[..live * PROVINCE_RECORD],
                    );
                    let complete = copied_fields / PROVINCE_FIELDS.len();
                    for slot in 0..complete {
                        let prec =
                            &prov_records[slot * PROVINCE_RECORD..(slot + 1) * PROVINCE_RECORD];
                        if u64_at(prec, 0) != province_vtable
                            || u64_at(prec, 8) != pointers[index] as u64
                        {
                            return Err("state province disagrees with its state");
                        }
                        let id = u32_at(prec, 16);
                        if id == 0 {
                            return Err("state province id unreadable or zero");
                        }
                        let tokens = Array::from_header(header16(prec, 20), 4)
                            .ok_or("province strategic location list invalid")?;
                        if tokens.count != 0 {
                            province_count = province_count
                                .checked_add(tokens.count as usize)
                                .ok_or("state core link count overflow")?;
                            if province_tokens.len() == province_tokens.capacity() {
                                crate::allocation::reserve(&mut province_tokens, 1).map_err(
                                    |_| "province strategic location allocation failed",
                                )?;
                            }
                            province_tokens.push((id, tokens));
                        }
                    }
                    if complete < live {
                        let prec = &prov_records
                            [complete * PROVINCE_RECORD..(complete + 1) * PROVINCE_RECORD];
                        let partial = copied_fields % PROVINCE_FIELDS.len();
                        if partial == 0 || u64_at(prec, 0) != province_vtable {
                            return Err("state province disagrees with its state");
                        }
                        if partial == 1 || u64_at(prec, 8) != pointers[index] as u64 {
                            return Err("state province disagrees with its state");
                        }
                        if partial == 2 || u32_at(prec, 16) == 0 {
                            return Err("state province id unreadable or zero");
                        }
                        return Err("province strategic location list invalid");
                    }
                    if let Some(reason) = overflow {
                        return Err(reason);
                    }
                    ordinal += chunk as u32;
                }
                let (core_count, claim_count) = (cores.count as usize, claims.count as usize);
                let location_count = locations.count as usize;
                let count = [
                    core_count,
                    claim_count,
                    2 * location_count,
                    2 * province_count,
                ]
                .into_iter()
                .try_fold(0_usize, usize::checked_add)
                .ok_or("state core link count overflow")?;
                *required = required
                    .checked_add(count)
                    .ok_or("state core link count overflow")?;
                rows.ensure(len + 1)?;
                links.ensure(used + count)?;
                let (Some(row), Some(slot)) =
                    (rows.get_mut(len), links.get_mut(used..used + count))
                else {
                    unread += 1;
                    continue;
                };
                let (core_slot, rest) = slot.split_at_mut(core_count);
                let (claim_slot, rest) = rest.split_at_mut(claim_count);
                let (location_slot, mut rest) = rest.split_at_mut(2 * location_count);
                copy_links(memory, cores, core_slot, "state core list unreadable")?;
                copy_links(memory, claims, claim_slot, "state claim list unreadable")?;
                copy_pairs(memory, locations, location_slot)?;
                for &(id, tokens) in &province_tokens {
                    let words = 2 * tokens.count as usize;
                    if words > rest.len() {
                        return Err("province strategic location list changed while read");
                    }
                    let (entries, after) = rest.split_at_mut(words);
                    copy_paired_tokens(memory, tokens, id, entries)?;
                    rest = after;
                }
                if !rest.is_empty() {
                    return Err("province strategic location list changed while read");
                }
                let locations_start = used + core_count + claim_count;
                *row = CoreRow {
                    state: ids[index],
                    start: used,
                    len: core_count,
                    claims_start: used + core_count,
                    claims_len: claim_count,
                    locations_start,
                    locations_len: location_count,
                    province_locations_start: locations_start + 2 * location_count,
                    province_locations_len: province_count,
                };
                len += 1;
                used += count;
            }
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn copy_paired_tokens(
    memory: &impl Memory,
    list: Array,
    province: u32,
    out: &mut [u32],
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; 4 * 64];
    for (chunk, out) in out.chunks_mut(128).enumerate() {
        let bytes = &mut bytes[..out.len() * 2];
        let copied = list
            .element((chunk * 64) as u32, 4)
            .is_some_and(|at| memory.copy(at, bytes));
        if !copied {
            return Err("province strategic location list unreadable");
        }
        for (entry, word) in out.chunks_exact_mut(2).zip(bytes.chunks_exact(4)) {
            entry.copy_from_slice(&[province, u32::from_le_bytes(word.try_into().unwrap())]);
        }
    }
    Ok(())
}

fn copy_pairs(memory: &impl Memory, list: Array, out: &mut [u32]) -> Result<(), &'static str> {
    let mut bytes = [0_u8; 8 * 32];
    for (chunk, out) in out.chunks_mut(64).enumerate() {
        let bytes = &mut bytes[..out.len() * 4];
        let copied = list
            .element((chunk * 32) as u32, cstate::STRATEGIC_LOCATION_STRIDE)
            .is_some_and(|at| memory.copy(at, bytes));
        if !copied {
            return Err("state strategic location list unreadable");
        }
        for (word, value) in out.iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_le_bytes(value.try_into().unwrap());
        }
    }
    Ok(())
}

fn copy_links(
    memory: &impl Memory,
    list: Array,
    out: &mut [u32],
    error: &'static str,
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; 4 * 64];
    for (chunk, out) in out.chunks_mut(64).enumerate() {
        let bytes = &mut bytes[..out.len() * 4];
        let copied = list
            .element((chunk * 64) as u32, 4)
            .is_some_and(|at| memory.copy(at, bytes));
        if !copied {
            return Err(error);
        }
        for (link, word) in out.iter_mut().zip(bytes.chunks_exact(4)) {
            *link = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    Ok(())
}
