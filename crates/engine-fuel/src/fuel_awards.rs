use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read};
use engine_native_air::{
    cair_base,
    ccountry_air_container,
    cstrategic_air,
    cstrategic_air_manager,
};
use engine_native::{ccountry, cfleet, cgame_state, cunit};
use crate::ccountry_fuel_status as fuel;
use engine_native_parts::creference_registry;
use engine_sites::Rva;

pub const BEGIN: Rva = Rva(0x0117_e3a3);
pub const END: Rva = Rva(0x0117_f1b3);
pub const LAND: [Rva; 5] = [
    Rva(0x0117_9d7d),
    Rva(0x0117_9dc2),
    Rva(0x0117_9e07),
    Rva(0x0117_9e4d),
    Rva(0x0117_9eba),
];
pub const NAVAL: Rva = Rva(0x00dc_ab00);
pub const AIR: Rva = Rva(0x00ca_8df1);
pub const RESET: Rva = Rva(0x0117_9839);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Started {
    pub status: usize,
    pub country: u32,
    pub country_instance: u32,
    pub pending_gain: i64,
    pub pending_consumed: i64,
    pub pending_other: i64,
    pub base: i64,
    pub states: i64,
    pub cost: i64,
    pub lend_lease_in: i64,
    pub history_index: i32,
    pub remaining_hours: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settled {
    pub gains: [i64; 4],
    pub own_gain: i64,
    pub transfers: [i64; 4],
    pub consumed: i64,
    pub received: [i64; 3],
    pub current: i64,
}

pub fn quantity(raw: i64) -> i64 {
    (raw % 100_000) * 0x4000_0000 / 0xc350_0000 + (raw / 100_000) * 0x8000
}

fn value<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("fuel award operand unreadable or overflowed")
}
fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|p| read(memory, p))
        .ok_or("fuel award operand unreadable or overflowed")
}
fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: Rva) -> Result<(), &'static str> {
    const MISMATCH: &str = "fuel award object identity mismatch";
    if pointer == 0 {
        return Err(MISMATCH);
    }
    let expected = engine_sites::at(base, rva).ok_or(MISMATCH)?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err(MISMATCH);
    }
    Ok(())
}
fn fields<const N: usize>(
    memory: &impl Memory,
    object: usize,
    spans: [(usize, usize); N],
) -> Result<[[u8; 8]; N], &'static str> {
    const UNREADABLE: &str = "fuel award operand unreadable or overflowed";
    let mut addresses = [0; N];
    for (address, (offset, _)) in addresses.iter_mut().zip(spans) {
        *address = object.checked_add(offset).ok_or(UNREADABLE)?;
    }
    let mut values = [[0; 8]; N];
    let mut widths = spans.into_iter().map(|(_, width)| width);
    let mut destinations = values
        .each_mut()
        .map(|value| &mut value[..widths.next().unwrap_or(0)]);
    if memory.copy_scattered(addresses, &mut destinations) != N {
        return Err(UNREADABLE);
    }
    Ok(values)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    pub owner: usize,
    pub country: engine_native_memory::country_identity::Country,
}

pub fn start(memory: &impl Memory, base: usize, status: usize) -> Result<Started, &'static str> {
    Ok(start_held(memory, base, status, None)?.0)
}

#[derive(Clone, Copy)]
struct ResolvedSites {
    located: usize,
    base: usize,
    fuel_vtable: Option<usize>,
    game_state: Option<usize>,
    country_vtable: Option<usize>,
    army_vtable: Option<usize>,
    task_force_vtable: Option<usize>,
    air_base_vtable: Option<usize>,
    air_container_vtable: Option<usize>,
}

std::thread_local! {
    static RESOLVED_SITES: std::cell::Cell<Option<ResolvedSites>> = const { std::cell::Cell::new(None) };
}

#[inline]
fn cached_sites(base: usize) -> Option<ResolvedSites> {
    let located = engine_sites::located().map_or(0, |l| l as *const _ as usize);
    if located == 0 {
        return None;
    }
    if let Some(sites) = RESOLVED_SITES.get()
        && sites.located == located
        && sites.base == base
    {
        return Some(sites);
    }
    let sites = ResolvedSites {
        located,
        base,
        fuel_vtable: engine_sites::resolve(base, fuel::VTABLE_RVA).ok(),
        game_state: engine_sites::resolve(base, cgame_state::SINGLETON_RVA).ok(),
        country_vtable: engine_sites::resolve(base, ccountry::VTABLE_RVA).ok(),
        army_vtable: engine_sites::resolve(base, engine_native::carmy::VTABLE_RVA).ok(),
        task_force_vtable: engine_sites::resolve(base, engine_native::ctask_force::VTABLE_RVA).ok(),
        air_base_vtable: engine_sites::resolve(
            base,
            engine_native_parts::cair_base_layout::VTABLE_RVA,
        )
        .ok(),
        air_container_vtable: engine_sites::resolve(base, ccountry_air_container::VTABLE_RVA).ok(),
    };
    RESOLVED_SITES.set(Some(sites));
    Some(sites)
}

pub fn start_held(
    memory: &impl Memory,
    base: usize,
    status: usize,
    held: Option<&Located>,
) -> Result<(Started, Located, bool), &'static str> {
    const MISMATCH: &str = "fuel award object identity mismatch";
    const UNREADABLE: &str = "fuel award operand unreadable or overflowed";
    if status == 0 {
        return Err(MISMATCH);
    }
    let sites = cached_sites(base);
    let expected = sites
        .and_then(|s| s.fuel_vtable)
        .or_else(|| engine_sites::at(base, fuel::VTABLE_RVA))
        .ok_or(MISMATCH)?;
    let singleton = sites
        .and_then(|s| s.game_state)
        .or_else(|| engine_sites::at(base, cgame_state::SINGLETON_RVA));
    let mut status_block = [0_u8; fuel::COUNTRY + 8];
    let mut singleton_bytes = [0_u8; 8];
    let status_ok = singleton.is_some_and(|singleton| {
        memory.copy_scattered_prefetched(
            [status, singleton],
            &mut [&mut status_block[..], &mut singleton_bytes[..]],
        ) == 2
    });
    if !status_ok {
        auth(memory, base, status, fuel::VTABLE_RVA)?;
        let game_state = global::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let owner = value::<u64>(memory, status, fuel::COUNTRY)? as usize;
        let (country, taken) = match held {
            Some(held) if held.owner == owner => (held.country, true),
            _ => (countries.of_pointer(owner)?, false),
        };
        if value::<u64>(memory, owner, ccountry::FUEL_STATUS)? != status as u64 {
            return Err("fuel award owner backlink mismatch");
        }
        let country_instance = value(memory, owner, ccountry::SURRENDER_TARGET)?;
        let started = payload(memory, status, country.id, country_instance)?;
        return Ok((started, Located { owner, country }, taken));
    }
    let u64_at = |offset: usize| u64::from_le_bytes(status_block[offset..offset + 8].try_into().unwrap());
    let i64_at = |offset: usize| i64::from_le_bytes(status_block[offset..offset + 8].try_into().unwrap());
    let i32_at = |offset: usize| i32::from_le_bytes(status_block[offset..offset + 4].try_into().unwrap());
    if u64_at(0) != expected as u64 {
        return Err(MISMATCH);
    }
    let owner = u64_at(fuel::COUNTRY) as usize;
    let game_state = u64::from_le_bytes(singleton_bytes) as usize;
    let expected_country = sites
        .and_then(|s| s.country_vtable)
        .or_else(|| engine_sites::at(base, ccountry::VTABLE_RVA))
        .map(|at| at as u64);
    let mut owner_vtable = [0_u8; 8];
    let mut backlink = [0_u8; 8];
    let mut surrender = [0_u8; 4];
    let mut owner_id = [0_u8; 4];
    let mut countries_header = [0_u8; 16];
    let mut links_header = [0_u8; 16];
    let second_ok = (|| -> Option<()> {
        let addresses = [
            owner,
            owner.checked_add(ccountry::FUEL_STATUS)?,
            owner.checked_add(ccountry::SURRENDER_TARGET)?,
            owner.checked_add(ccountry::ID)?,
            game_state.checked_add(cgame_state::COUNTRIES)?,
            game_state.checked_add(cgame_state::COUNTRY_LINKS)?,
        ];
        (memory.copy_scattered_prefetched(
            addresses,
            &mut [
                &mut owner_vtable[..],
                &mut backlink[..],
                &mut surrender[..],
                &mut owner_id[..],
                &mut countries_header[..],
                &mut links_header[..],
            ],
        ) == 6)
            .then_some(())
    })()
    .is_some();
    let (country, taken, country_instance) = if second_ok {
        let authenticated = memory.authenticated_countries(base, game_state).is_some();
        let countries = if authenticated {
            ccountry::Reader::new(memory, base, game_state)?
        } else {
            let countries = Array::from_header(countries_header, 8)
                .ok_or("country array unreadable or invalid")?;
            let links = Array::from_header(links_header, 4)
                .ok_or("country links unreadable or invalid")?;
            ccountry::Reader::from_arrays(memory, base, game_state, [countries, links])?
        };
        let vtable_ok = expected_country == Some(u64::from_le_bytes(owner_vtable));
        let (country, taken) = match held {
            Some(held) if held.owner == owner => (held.country, true),
            _ if authenticated || vtable_ok => {
                let id = u32::from_le_bytes(owner_id);
                let resolved = match (id != 0)
                    .then(|| countries.verify_canonical_pointer(owner, id))
                    .flatten()
                {
                    Some(country) => country,
                    None => countries.of_pointer(owner)?,
                };
                (resolved, false)
            }
            _ => (countries.of_pointer(owner)?, false),
        };
        if u64::from_le_bytes(backlink) != status as u64 {
            return Err("fuel award owner backlink mismatch");
        }
        (country, taken, u32::from_le_bytes(surrender))
    } else {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (country, taken) = match held {
            Some(held) if held.owner == owner => (held.country, true),
            _ => (countries.of_pointer(owner)?, false),
        };
        if value::<u64>(memory, owner, ccountry::FUEL_STATUS)? != status as u64 {
            return Err("fuel award owner backlink mismatch");
        }
        let country_instance = value(memory, owner, ccountry::SURRENDER_TARGET).map_err(|_| UNREADABLE)?;
        (country, taken, country_instance)
    };
    let started = Started {
        status,
        country: country.id,
        country_instance,
        pending_gain: i64_at(0x68),
        pending_consumed: i64_at(0x70),
        pending_other: i64_at(0x90),
        base: i64_at(fuel::GAIN),
        states: i64_at(fuel::GAIN_FROM_STATES),
        cost: i64_at(fuel::COST),
        lend_lease_in: i64_at(fuel::LEND_LEASE_GAIN),
        history_index: i32_at(fuel::HISTORY_INDEX),
        remaining_hours: i32_at(fuel::REMAINING_HOURS),
    };
    Ok((started, Located { owner, country }, taken))
}
fn payload(
    memory: &impl Memory,
    status: usize,
    country: u32,
    country_instance: u32,
) -> Result<Started, &'static str> {
    let [
        pending_gain,
        pending_consumed,
        pending_other,
        gain,
        states,
        cost,
        lend_lease_in,
        history_index,
        remaining_hours,
    ] = fields(
        memory,
        status,
        [
            (0x68, 8),
            (0x70, 8),
            (0x90, 8),
            (fuel::GAIN, 8),
            (fuel::GAIN_FROM_STATES, 8),
            (fuel::COST, 8),
            (fuel::LEND_LEASE_GAIN, 8),
            (fuel::HISTORY_INDEX, 4),
            (fuel::REMAINING_HOURS, 4),
        ],
    )?;
    let half = |bytes: [u8; 8]| i32::from_le_bytes(bytes[..4].try_into().unwrap());
    Ok(Started {
        status,
        country,
        country_instance,
        pending_gain: i64::from_le_bytes(pending_gain),
        pending_consumed: i64::from_le_bytes(pending_consumed),
        pending_other: i64::from_le_bytes(pending_other),
        base: i64::from_le_bytes(gain),
        states: i64::from_le_bytes(states),
        cost: i64::from_le_bytes(cost),
        lend_lease_in: i64::from_le_bytes(lend_lease_in),
        history_index: half(history_index),
        remaining_hours: half(remaining_hours),
    })
}
pub fn settle(
    memory: &impl Memory,
    base: usize,
    begun: Started,
    status: usize,
) -> Result<Settled, &'static str> {
    Ok(settle_held(memory, base, begun, status, None)?.0)
}

pub fn settle_held(
    memory: &impl Memory,
    base: usize,
    begun: Started,
    status: usize,
    held: Option<&Located>,
) -> Result<(Settled, bool), &'static str> {
    const SETTLE_SPAN: usize = fuel::REMAINING_HOURS + 4 - fuel::CURRENT;
    if held.is_some()
        && status == begun.status
        && let Some(start_addr) = status.checked_add(fuel::CURRENT)
    {
        let mut block = [0_u8; SETTLE_SPAN];
        if engine_native_memory::read_slice(memory, start_addr, &mut block) {
            let i64_at = |offset: usize| {
                let at = offset - fuel::CURRENT;
                i64::from_le_bytes(block[at..at + 8].try_into().unwrap())
            };
            let i32_at = |offset: usize| {
                let at = offset - fuel::CURRENT;
                i32::from_le_bytes(block[at..at + 4].try_into().unwrap())
            };
            let now = Started {
                status,
                country: begun.country,
                country_instance: begun.country_instance,
                pending_gain: i64_at(0x68),
                pending_consumed: i64_at(0x70),
                pending_other: i64_at(0x90),
                base: i64_at(fuel::GAIN),
                states: i64_at(fuel::GAIN_FROM_STATES),
                cost: i64_at(fuel::COST),
                lend_lease_in: i64_at(fuel::LEND_LEASE_GAIN),
                history_index: i32_at(fuel::HISTORY_INDEX),
                remaining_hours: i32_at(fuel::REMAINING_HOURS),
            };
            if now.history_index != begun.history_index
                || now.remaining_hours != begun.remaining_hours
                || [now.base, now.states, now.cost, now.lend_lease_in]
                    != [begun.base, begun.states, begun.cost, begun.lend_lease_in]
            {
                return Err("fuel award root or applied operands changed");
            }
            let own_gain = now.pending_gain.wrapping_sub(begun.pending_gain);
            let cost_gain = if begun.cost < 0 {
                begun.cost.wrapping_neg()
            } else {
                0
            };
            let oil = own_gain
                .wrapping_sub(begun.base)
                .wrapping_sub(begun.states)
                .wrapping_sub(cost_gain);
            let other = now.pending_other.wrapping_sub(begun.pending_other);
            let outgoing = i64_at(fuel::LEND_LEASE_CONSUMPTION);
            let consumers_at = fuel::CONSUMERS - fuel::CURRENT;
            let header: [u8; 16] = block[consumers_at..consumers_at + 16].try_into().unwrap();
            let consumers = Array::from_header(header, crate::fuel_consumer_data::STRIDE)
                .filter(|a| a.count == fuel::MAX_CONSUMERS)
                .ok_or("fuel award consumers unreadable or invalid")?;
            let received = read_consumer_receipts(memory, consumers.data)?;
            let settled = Settled {
                gains: [begun.base, begun.states, oil, cost_gain],
                own_gain,
                transfers: [
                    begun.lend_lease_in,
                    outgoing,
                    other.wrapping_sub(outgoing),
                    other,
                ],
                consumed: now.pending_consumed.wrapping_sub(begun.pending_consumed),
                received,
                current: i64_at(fuel::CURRENT),
            };
            return Ok((settled, true));
        }
    }
    let (now, taken) = match held {
        Some(_) if status == begun.status => (
            payload(memory, status, begun.country, begun.country_instance)?,
            true,
        ),
        _ => {
            let (now, _, taken) = start_held(memory, base, status, held)?;
            (now, taken)
        }
    };
    if now.status != begun.status
        || now.country != begun.country
        || now.history_index != begun.history_index
        || now.remaining_hours != begun.remaining_hours
        || [now.base, now.states, now.cost, now.lend_lease_in]
            != [begun.base, begun.states, begun.cost, begun.lend_lease_in]
    {
        return Err("fuel award root or applied operands changed");
    }
    let own_gain = now.pending_gain.wrapping_sub(begun.pending_gain);
    let cost_gain = if begun.cost < 0 {
        begun.cost.wrapping_neg()
    } else {
        0
    };
    let oil = own_gain
        .wrapping_sub(begun.base)
        .wrapping_sub(begun.states)
        .wrapping_sub(cost_gain);
    let other = now.pending_other.wrapping_sub(begun.pending_other);
    let outgoing = value(memory, status, fuel::LEND_LEASE_CONSUMPTION)?;
    let consumers = Array::read(
        memory,
        status
            .checked_add(fuel::CONSUMERS)
            .ok_or("fuel consumer address overflow")?,
        crate::fuel_consumer_data::STRIDE,
    )
    .filter(|a| a.count == fuel::MAX_CONSUMERS)
    .ok_or("fuel award consumers unreadable or invalid")?;
    let received = read_consumer_receipts(memory, consumers.data)?;
    let settled = Settled {
        gains: [begun.base, begun.states, oil, cost_gain],
        own_gain,
        transfers: [
            begun.lend_lease_in,
            outgoing,
            other.wrapping_sub(outgoing),
            other,
        ],
        consumed: now.pending_consumed.wrapping_sub(begun.pending_consumed),
        received,
        current: value(memory, status, fuel::CURRENT)?,
    };
    Ok((settled, taken))
}

fn read_consumer_receipts(memory: &impl Memory, data: usize) -> Result<[i64; 3], &'static str> {
    let mut block = [0_u8; 3 * crate::fuel_consumer_data::STRIDE];
    if engine_native_memory::read_slice(memory, data, &mut block) {
        return Ok(std::array::from_fn(|index| {
            let at = index * crate::fuel_consumer_data::STRIDE + crate::fuel_consumer_data::RECEIVED;
            i64::from_le_bytes(block[at..at + 8].try_into().unwrap())
        }));
    }
    let receipt = |index: usize| {
        (
            index * crate::fuel_consumer_data::STRIDE + crate::fuel_consumer_data::RECEIVED,
            8,
        )
    };
    Ok(fields(memory, data, [receipt(0), receipt(1), receipt(2)])?.map(i64::from_le_bytes))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recipient {
    pub kind: u32,
    pub reference: u64,
    pub owner: u32,
    pub host: u64,
}
#[derive(Default)]
pub struct Registry(creference_registry::HeaderResolver);

pub fn unit(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    kind: u32,
) -> Result<Recipient, &'static str> {
    Ok(unit_with(memory, base, pointer, kind, &mut Registry::default())?.0)
}

pub fn unit_with(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    kind: u32,
    registry: &mut Registry,
) -> Result<(Recipient, bool), &'static str> {
    let class = match kind {
        0 => 0,
        2 => 1,
        _ => return Err("fuel recipient kind unsupported"),
    };
    let mut taken = false;
    let (reference, actual_class) =
        cunit::counter_identity_with(memory, base, pointer, |handle| {
            taken = registry.0.holds(base, handle);
            registry.0.resolve(memory, base, handle)
        })?;
    if actual_class != class {
        return Err("fuel recipient unit class mismatch");
    }
    Ok((
        Recipient {
            kind,
            reference,
            owner: value(memory, pointer, cunit::ROSTER_OWNER)?,
            host: 0,
        },
        taken,
    ))
}

pub fn unit_with_name(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    kind: u32,
    registry: Option<&mut Registry>,
) -> Result<(Recipient, Result<StoredName, &'static str>, bool), &'static str> {
    let class = match kind {
        0 => 0_u32,
        2 => 1_u32,
        _ => return Err("fuel recipient kind unsupported"),
    };
    let sites = cached_sites(base);
    let expected_vtable = if class == 0 {
        sites
            .and_then(|s| s.army_vtable)
            .or_else(|| engine_sites::at(base, engine_native::carmy::VTABLE_RVA))
    } else {
        sites
            .and_then(|s| s.task_force_vtable)
            .or_else(|| engine_sites::at(base, engine_native::ctask_force::VTABLE_RVA))
    }
    .ok_or("counter unit module overflow")? as u64;
    if pointer != 0
        && let Some(owner_addr) = pointer.checked_add(cunit::ROSTER_OWNER)
    {
        let mut header = [0_u8; 32];
        let mut owner_bytes = [0_u8; 4];
        let name_read = if kind == 0 {
            pointer
                .checked_add(engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER)
                .and_then(|member_addr| {
                    let mut member_bytes = [0_u8; 8];
                    (memory.copy_scattered_prefetched(
                        [pointer, owner_addr, member_addr],
                        &mut [&mut header[..], &mut owner_bytes[..], &mut member_bytes[..]],
                    ) == 3)
                        .then_some((
                            u64::from_le_bytes(member_bytes) as usize,
                            [0_u8; msvc_string::STRIDE],
                        ))
                })
        } else {
            pointer.checked_add(TASK_FORCE_NAME).and_then(|name_addr| {
                let mut name_header = [0_u8; msvc_string::STRIDE];
                (memory.copy_scattered_prefetched(
                    [pointer, owner_addr, name_addr],
                    &mut [&mut header[..], &mut owner_bytes[..], &mut name_header[..]],
                ) == 3)
                    .then_some((name_addr, name_header))
            })
        };
        if let Some((name_ptr, name_header)) = name_read {
            let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
            let actual_class = u32::from_le_bytes(header[8..12].try_into().unwrap());
            let handle = u64::from_le_bytes(header[24..32].try_into().unwrap());
            if actual_class != class || handle >> 32 == 0 || vtable != expected_vtable {
                return Err(if actual_class != class && [0, 1, 13].contains(&actual_class) {
                    "fuel recipient unit class mismatch"
                } else {
                    "counter unit identity invalid"
                });
            }
            let (resolved, taken, gathered_name) = match registry {
                Some(registry) if kind == 0 => {
                    if let Some((header, cell, taken)) =
                        registry.0.first_cell(memory, base, handle)?
                    {
                        if let Some(name_at) =
                            name_ptr.checked_add(engine_native::cname_group_member::CACHED_NAME)
                        {
                            let mut bucket = [0_u8; 24];
                            let mut desc = [0_u8; msvc_string::STRIDE];
                            if memory.copy_scattered_prefetched(
                                [cell, name_at],
                                &mut [&mut bucket, &mut desc],
                            ) == 2
                            {
                                (
                                    registry.0.resolve_gathered(memory, header, handle, bucket)?,
                                    taken,
                                    Some((name_at, desc)),
                                )
                            } else {
                                (registry.0.resolve(memory, base, handle)?, taken, None)
                            }
                        } else {
                            (registry.0.resolve(memory, base, handle)?, taken, None)
                        }
                    } else {
                        (None, false, None)
                    }
                }
                Some(registry) => {
                    let taken = registry.0.holds(base, handle);
                    (registry.0.resolve(memory, base, handle)?, taken, None)
                }
                None => (creference_registry::resolve(memory, base, handle)?, false, None),
            };
            if resolved != pointer.checked_add(0x10) {
                return Err("counter unit registry identity mismatch");
            }
            let name = if kind == 0 {
                match gathered_name {
                    Some((at, desc)) => StoredName::from_header_eager(memory, at, desc),
                    None => name_ptr
                        .checked_add(engine_native::cname_group_member::CACHED_NAME)
                        .ok_or("fuel recipient name address overflow")
                        .and_then(|at| {
                            read::<[u8; msvc_string::STRIDE]>(memory, at)
                                .ok_or("string header unreadable")
                                .and_then(|desc| StoredName::from_header_eager(memory, at, desc))
                        }),
                }
            } else {
                StoredName::from_header_eager(memory, name_ptr, name_header)
            };
            return Ok((
                Recipient {
                    kind,
                    reference: handle,
                    owner: u32::from_le_bytes(owner_bytes),
                    host: 0,
                },
                name,
                taken,
            ));
        }
    }
    let (recipient, taken) = match registry {
        Some(registry) => unit_with(memory, base, pointer, kind, registry)?,
        None => (unit(memory, base, pointer, kind)?, false),
    };
    let name = stored_name(memory, pointer, kind);
    Ok((recipient, name, taken))
}
pub fn air(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
) -> Result<Recipient, &'static str> {
    Ok(air_held(memory, base, pointer, container, consuming, None)?.0)
}

pub fn air_held(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
    held: Option<&engine_native_memory::country_identity::Country>,
) -> Result<(Recipient, bool), &'static str> {
    air_held_with(memory, base, pointer, container, consuming, held, None)
}

pub fn air_held_with(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
    held: Option<&engine_native_memory::country_identity::Country>,
    registry: Option<&mut Registry>,
) -> Result<(Recipient, bool), &'static str> {
    const MISMATCH: &str = "fuel award object identity mismatch";
    let held = held.filter(|country| country.id == consuming);
    if pointer == 0 || container == 0 {
        return Err(MISMATCH);
    }
    let sites = cached_sites(base);
    let expected_base = sites
        .and_then(|s| s.air_base_vtable)
        .or_else(|| engine_sites::at(base, engine_native_parts::cair_base_layout::VTABLE_RVA))
        .ok_or(MISMATCH)?;
    let expected_container = sites
        .and_then(|s| s.air_container_vtable)
        .or_else(|| engine_sites::at(base, ccountry_air_container::VTABLE_RVA))
        .ok_or(MISMATCH)?;
    let (
        mut base_vtable,
        mut handle_bytes,
        mut containers_header,
        mut host_bytes,
        mut container_vtable,
        mut owner_bytes,
    ) = ([0_u8; 8], [0_u8; 8], [0_u8; 16], [0_u8; 8], [0_u8; 8], [0_u8; 4]);
    let scattered_ok = (|| -> Option<()> {
        let addresses = [
            pointer,
            pointer.checked_add(engine_native_parts::cair_base_layout::HANDLE)?,
            pointer.checked_add(cair_base::COUNTRY_CONTAINERS)?,
            pointer.checked_add(engine_native_parts::cair_base_layout::HOST_REFERENCE)?,
            container,
            container.checked_add(ccountry_air_container::COUNTRY)?,
        ];
        (memory.copy_scattered_prefetched(
            addresses,
            &mut [
                &mut base_vtable,
                &mut handle_bytes,
                &mut containers_header,
                &mut host_bytes,
                &mut container_vtable,
                &mut owner_bytes,
            ],
        ) == 6)
            .then_some(())
    })()
    .is_some();
    let (reference, owner, containers, host) = if scattered_ok {
        if u64::from_le_bytes(base_vtable) != expected_base as u64
            || u64::from_le_bytes(container_vtable) != expected_container as u64
        {
            return Err(MISMATCH);
        }
        (
            u64::from_le_bytes(handle_bytes),
            u32::from_le_bytes(owner_bytes),
            Array::from_header(containers_header, 8),
            Some(u64::from_le_bytes(host_bytes)),
        )
    } else {
        auth(memory, base, pointer, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
        auth(memory, base, container, ccountry_air_container::VTABLE_RVA)?;
        let reference = value::<u64>(memory, pointer, engine_native_parts::cair_base_layout::HANDLE)?;
        let owner = value::<u32>(memory, container, ccountry_air_container::COUNTRY)?;
        (reference, owner, None, None)
    };
    let mut gathered_backlink: Option<u64> = None;
    let resolved = match (registry, held, containers) {
        (Some(registry), Some(country), Some(containers))
            if owner == consuming && reference >> 32 != 0 =>
        {
            if let Some((header, cell, _)) = registry.0.first_cell(memory, base, reference)? {
                if let Some(slot_at) = containers.element(country.canonical_slot, 8) {
                    let mut bucket = [0_u8; 24];
                    let mut backlink = [0_u8; 8];
                    if memory.copy_scattered_prefetched(
                        [cell, slot_at],
                        &mut [&mut bucket, &mut backlink],
                    ) == 2
                    {
                        gathered_backlink = Some(u64::from_le_bytes(backlink));
                        registry.0.resolve_gathered(memory, header, reference, bucket)?
                    } else {
                        registry.0.resolve(memory, base, reference)?
                    }
                } else {
                    registry.0.resolve(memory, base, reference)?
                }
            } else {
                None
            }
        }
        (Some(registry), _, _) => registry.0.resolve(memory, base, reference)?,
        (None, _, _) => creference_registry::resolve(memory, base, reference)?,
    };
    if reference >> 32 == 0 || resolved != Some(pointer) {
        return Err("fuel air base registry identity mismatch");
    }
    let country = match held {
        Some(country) if owner == consuming => *country,
        _ => {
            let game_state = global::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
            let countries = ccountry::Reader::new(memory, base, game_state)?;
            let country = match held {
                Some(country) => *country,
                None => countries.resolve(consuming)?,
            };
            let owner_pointer = match held {
                Some(country) if owner == consuming => country.pointer,
                _ => countries.resolve(owner)?.pointer,
            };
            if owner_pointer != country.pointer {
                return Err("fuel air recipient consumes another country");
            }
            country
        }
    };
    if let Some(backlink) = gathered_backlink {
        if backlink != container as u64 {
            return Err("fuel air container backlink mismatch");
        }
    } else {
        let containers = match containers {
            Some(containers) => containers,
            None => Array::read(
                memory,
                pointer
                    .checked_add(cair_base::COUNTRY_CONTAINERS)
                    .ok_or("fuel air container address overflow")?,
                8,
            )
            .ok_or("fuel air container vector invalid")?,
        };
        if containers
            .element(country.canonical_slot, 8)
            .and_then(|p| read::<u64>(memory, p))
            != Some(container as u64)
        {
            return Err("fuel air container backlink mismatch");
        }
    }
    let host = match host {
        Some(host) => host,
        None => value(
            memory,
            pointer,
            engine_native_parts::cair_base_layout::HOST_REFERENCE,
        )?,
    };
    Ok((
        Recipient {
            kind: 1,
            reference,
            owner,
            host,
        },
        held.is_some(),
    ))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub entries: usize,
    pub names: usize,
    pub name_stride: usize,
}

const EAGER_NAME_BYTES: usize = 128;

pub enum StoredName {
    Deferred(msvc_string::Descriptor),
    Loaded {
        bytes: [std::mem::MaybeUninit<u8>; EAGER_NAME_BYTES],
        len: usize,
    },
}

impl StoredName {
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Deferred(desc) => desc.len,
            Self::Loaded { len, .. } => *len,
        }
    }
    pub fn loaded_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Deferred(_) => None,
            Self::Loaded { bytes, len } => {
                // SAFETY: `from_header_eager` initializes `bytes[..span]` with `span = *len + 1`.
                Some(unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), *len) })
            }
        }
    }
    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        match self {
            Self::Deferred(desc) => desc.copy(memory, output),
            Self::Loaded { bytes, len } => {
                let output = output.get_mut(..*len).ok_or("string output too short")?;
                // SAFETY: `from_header_eager` initializes `bytes[..span]` with `span = *len + 1`.
                let source =
                    unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), *len) };
                output.copy_from_slice(source);
                Ok(())
            }
        }
    }
    fn from_header_eager(
        memory: &impl Memory,
        address: usize,
        header: [u8; msvc_string::STRIDE],
    ) -> Result<Self, &'static str> {
        if let Some((source, span)) = msvc_string::heap_span(&header)
            && span <= EAGER_NAME_BYTES
        {
            let len = span - 1;
            let mut bytes = [std::mem::MaybeUninit::<u8>::uninit(); EAGER_NAME_BYTES];
            if let Some(copied) =
                engine_native_memory::memory::read_slice_uninit(memory, source, &mut bytes[..span])
            {
                if copied[len] != 0 {
                    return Err("string terminator unreadable or invalid");
                }
                return Ok(Self::Loaded { bytes, len });
            }
        }
        msvc_string::Descriptor::from_header(memory, address, header).map(Self::Deferred)
    }
}
const TASK_FORCE_NAME: usize = 0x258;

pub fn stored_name(
    memory: &impl Memory,
    pointer: usize,
    kind: u32,
) -> Result<StoredName, &'static str> {
    let address = match kind {
        0 => (value::<u64>(
            memory,
            pointer,
            engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER,
        )? as usize)
            .checked_add(engine_native::cname_group_member::CACHED_NAME)
            .ok_or("fuel recipient name address overflow")?,
        2 => pointer
            .checked_add(TASK_FORCE_NAME)
            .ok_or("fuel recipient name address overflow")?,
        _ => return Err("fuel recipient has no native unit name"),
    };
    msvc_string::Descriptor::read(memory, address).map(StoredName::Deferred)
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    if game_state == 0 {
        return Ok(Demand::default());
    }
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let manager = value::<u64>(memory, game_state, cgame_state::STRATEGIC_AIR_MANAGER)? as usize;
    auth(memory, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
    let airs = Array::read(
        memory,
        manager
            .checked_add(cstrategic_air_manager::COUNTRY_AIR)
            .ok_or("fuel census air registry address overflow")?,
        8,
    )
    .ok_or("fuel census air registry invalid")?;
    let mut count = Demand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let divisions = Array::read(
            memory,
            country
                .pointer
                .checked_add(ccountry::DIVISIONS)
                .ok_or("fuel census division roster address overflow")?,
            8,
        )
        .ok_or("fuel census division roster invalid")?;
        let fleets = Array::read(
            memory,
            country
                .pointer
                .checked_add(ccountry::FLEETS)
                .ok_or("fuel census fleet roster address overflow")?,
            8,
        )
        .ok_or("fuel census fleet roster invalid")?;
        let mut recipients = divisions.count as usize;
        for index in 0..divisions.count {
            let division = divisions
                .element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .ok_or("fuel census division pointer unreadable")?
                as usize;
            if let Ok(name) = stored_name(memory, division, 0) {
                count.name_stride = count.name_stride.max(name.byte_len());
                count.names = count
                    .names
                    .checked_add(name.byte_len())
                    .ok_or("fuel census name size overflow")?;
            }
        }
        for index in 0..fleets.count {
            let fleet = fleets
                .element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .ok_or("fuel census fleet pointer unreadable")? as usize;
            auth(memory, base, fleet, cfleet::VTABLE_RVA)?;
            let forces = Array::read(
                memory,
                fleet
                    .checked_add(cfleet::TASK_FORCES)
                    .ok_or("fuel census task-force roster address overflow")?,
                8,
            )
            .ok_or("fuel census task-force roster invalid")?;
            recipients = recipients
                .checked_add(forces.count as usize)
                .ok_or("fuel census size overflow")?;
            for index in 0..forces.count {
                let force = forces
                    .element(index, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .ok_or("fuel census task-force pointer unreadable")?
                    as usize;
                if let Ok(name) = stored_name(memory, force, 2) {
                    count.name_stride = count.name_stride.max(name.byte_len());
                    count.names = count
                        .names
                        .checked_add(name.byte_len())
                        .ok_or("fuel census name size overflow")?;
                }
            }
        }
        let air = airs
            .element(slot, 8)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("fuel census country air pointer unreadable")? as usize;
        if air != 0 {
            auth(memory, base, air, cstrategic_air::VTABLE_RVA)?;
            for offset in [
                cstrategic_air::FIRST_AIR_SITES,
                cstrategic_air::SECOND_AIR_SITES,
            ] {
                let sites = Array::read(
                    memory,
                    air.checked_add(offset)
                        .ok_or("fuel census air site address overflow")?,
                    8,
                )
                .ok_or("fuel census air sites invalid")?;
                recipients = recipients
                    .checked_add(sites.count as usize)
                    .ok_or("fuel census size overflow")?;
            }
        }
        count.entries = count
            .entries
            .checked_add(recipients)
            .and_then(|n| n.checked_add(1))
            .ok_or("fuel census size overflow")?;
    }
    Ok(count)
}

#[cfg(test)]
#[path = "fuel_awards/tests.rs"]
mod tests;
