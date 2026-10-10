use super::*;

/// Attach/cadence storage demand from registries and retained array/name
/// headers. No resource amounts, modifier values, route pairs or core links
/// are acquired. Resource, province-node and core checks select these same
/// populations; the worker authenticates its own original headers and grows
/// if they changed after this hint.
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
        address(base, cstrategic_resource_database::SINGLETON_RVA)?,
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
    let vtable = address(base, cstate::VTABLE_RVA)? as u64;
    crate::cpdx_array::pointers(
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
            core_links = add(core_links, cores.count as usize)?;
            let accumulator = array(pointer, cstate::BASE_MODIFIERS, modifier_entry::STRIDE)?;
            let container = address(pointer, cstate::DYNAMIC_MODIFIERS)?;
            if read::<u64>(memory, container)
                != Some(address(base, cdynamic_modifier_container::VTABLE_RVA)? as u64)
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
                    != Some(address(base, dynamic_modifier_entry::VTABLE_RVA)? as u64)
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
    // A row-count upper bound also covers all distinct name dictionary keys.
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
            != Some(address(base, ccountry_resources::VTABLE_RVA)? as u64)
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

/// The native IDs of every state the state array holds (a state's ID is its
/// slot), into `output`: the owners the sliced state tables are scanned
/// over. One guarded copy per 512 slots; a state is not authenticated here,
/// the walk does that when it reads one.
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
                // Null slots do not contribute to the actual census count.
                output.push(first + index as u32);
            }
        }
        first += count as u32;
    }
    Ok(())
}

/// One state's core countries (+0xB0, count +0xBC): raw country references
/// in stored order at `links[start..start + len]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoreRow {
    pub state: u32,
    pub start: usize,
    pub len: usize,
}

/// The core lists of the census states `selected` accepts, one row per state
/// (an empty list included), in census order. Each state is authenticated
/// (vtable and ID) with its list header in one record batch of 64; each
/// nonempty list is one guarded copy. A state whose list does not fit in
/// `links` has no row and is counted; `required` is the links every
/// selected state needs.
#[allow(clippy::too_many_arguments)]
pub fn state_cores(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[u32],
    selected: &dyn Fn(u32) -> bool,
    rows: &mut [CoreRow],
    links: &mut [u32],
    required: &mut usize,
) -> Outcome {
    state_cores_buffer(
        memory,
        base,
        game_state,
        census,
        selected,
        Buffer::Fixed(rows),
        Buffer::Fixed(links),
        required,
    )
}

/// Worker-only acquisition; the original core-list headers size owned links.
#[allow(clippy::too_many_arguments)]
pub fn state_cores_growing(
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

#[allow(clippy::too_many_arguments)]
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
    const RECORD: usize = 32;
    const FIELDS: [RecordField; 3] = [
        field(0, 0, 8),
        field(cstate::ID, 8, 4),
        field(cstate::CORES, 16, 16),
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, RECORD).expect("valid record layout");
    *required = 0;
    let result = (|| {
        let root = address(game_state, cgame_state::STATES)
            .ok()
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state array unreadable or invalid")?;
        let vtable = address(base, cstate::VTABLE_RVA)? as u64;
        let (mut len, mut used, mut unread) = (0, 0, 0_u32);
        let mut ids = [0_u32; BATCH];
        let mut pointers = [0_usize; BATCH];
        let mut copied = [0_u8; BATCH * RECORD];
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
            let mut words = [0_u8; 8];
            for index in 0..count {
                let at = root
                    .element(ids[index], 8)
                    .ok_or("state pointer unreadable")?;
                if !memory.copy(at, &mut words) {
                    return Err("state pointer unreadable");
                }
                pointers[index] = usize::try_from(u64::from_le_bytes(words))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("state pointer null")?;
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
                let count = cores.count as usize;
                *required = required
                    .checked_add(count)
                    .ok_or("state core link count overflow")?;
                // Core checks reach every stored core occurrence.
                rows.ensure(len + 1)?;
                links.ensure(used + count)?;
                let (Some(row), Some(slot)) =
                    (rows.get_mut(len), links.get_mut(used..used + count))
                else {
                    unread += 1;
                    continue;
                };
                if count != 0 {
                    let mut bytes = [0_u8; 4 * 64];
                    for (chunk, out) in slot.chunks_mut(64).enumerate() {
                        let bytes = &mut bytes[..out.len() * 4];
                        let copied = cores
                            .element((chunk * 64) as u32, 4)
                            .is_some_and(|at| memory.copy(at, bytes));
                        if !copied {
                            return Err("state core list unreadable");
                        }
                        for (link, word) in out.iter_mut().zip(bytes.chunks_exact(4)) {
                            *link = u32::from_le_bytes(word.try_into().unwrap());
                        }
                    }
                }
                *row = CoreRow {
                    state: ids[index],
                    start: used,
                    len: count,
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
