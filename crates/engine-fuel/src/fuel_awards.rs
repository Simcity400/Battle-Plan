//! Applied fuel operands, observed without game calls. The serial owned-state
//! country loop applies fuel updates before the hourly boundary. Rate refresh
//! also has other callers; later rates cannot reconstruct historical awards.
use crate::{
    Memory, cair_base, ccountry, ccountry_air_container, ccountry_fuel_status as fuel, cfleet,
    cgame_state, cpdx_array::Array, creference_registry, cstrategic_air, cstrategic_air_manager,
    cunit, msvc_string, read,
};

/// Before the normal pending-gain increment, after refresh and incoming LL.
/// The overwritten MOV+ADD is eight bytes, with no incoming interior branch.
pub const BEGIN: (usize, [u8; 16]) = (
    0x0117_e333,
    [
        0x48, 0x8b, 0x5e, 0x18, 0x48, 0x03, 0x5e, 0x20, 0x4c, 0x39, 0x6e, 0x38, 0x7d, 0x14, 0x48,
        0x8b,
    ],
);
/// All awards and Q15 consumer debits have settled, before pending/ring clear.
/// Post-update eight-byte load span before the original clear continuation.
pub const END: (usize, [u8; 16]) = (
    0x0117_f143,
    [
        0x48, 0x8b, 0x4b, 0x48, 0x48, 0x8b, 0x41, 0x40, 0x48, 0x01, 0x83, 0xa8, 0, 0, 0, 0x48,
    ],
);
/// Division receipt writes, including unrolled and residual paths. RAX is the division, RDX the
/// actual signed fixed5 receipt with parity rounding. Chained unwind entries lead to a handler-free
/// body; each five-byte return point remains in its original entry.
pub const LAND: [(usize, [u8; 16]); 5] = [
    (
        0x0117_9d0d,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x0117_9d52,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x0117_9d97,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x0117_9ddd,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x48, 0x83, 0xed, 1, 0x0f, 0x85,
        ],
    ),
    (
        0x0117_9e4a,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x48, 0x83, 0xef, 1, 0x75, 0xb2,
        ],
    ),
];
/// Complete receipt addition, preserving the original return. RCX task force,
/// RDX signed fixed5 receipt. This leaf has no unwind entry or handler.
pub const NAVAL: (usize, [u8; 16]) = (
    0x00dc_aa90,
    [
        0x48, 1, 0x91, 0xf8, 4, 0, 0, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
    ],
);
/// Non-null-container receipt store: RDI base, RDX container, RBX receipt. The seven-byte store
/// ends before register restoration; the five-byte return point remains inside the handler-free
/// body.
pub const AIR: (usize, [u8; 16]) = (
    0x00ca_8d81,
    [
        0x48, 0x89, 0x9a, 0xf0, 0, 0, 0, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x83, 0xc4, 0x20,
    ],
);
/// Unconditional fuel-reset prefix, RSI status. Appearance reset reaches it during runtime reset,
/// country reuse and initialization; a pointer and zero identity can recur. The eight-byte span
/// follows the handler-free prologue and contains no interior branch or return address. The
/// five-byte return point shares its unwind state.
pub const RESET: (usize, [u8; 16]) = (
    0x0117_97c9,
    [
        0x48, 0x89, 0x46, 0x30, 0x48, 0x89, 0x6e, 0x38, 0x48, 0x89, 0x6e, 0x40, 0x89, 0x6e, 0x54,
        0x8b,
    ],
);

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
    /// Own gain components: base, state modifiers, oil and negative-cost gain,
    /// all fixed5. Resolve the content-loaded breakdown templates rather than
    /// substituting a built-in refinery name.
    pub gains: [i64; 4],
    pub own_gain: i64,
    /// Q15: incoming LL, outgoing LL, positive-cost debit, total other debit.
    pub transfers: [i64; 4],
    pub consumed: i64,
    pub received: [i64; 3],
    pub current: i64,
}

/// Fixed5-to-Q15 conversion with truncation toward zero. The quotient/remainder
/// form avoids overflowing a direct product.
pub fn quantity(raw: i64) -> i64 {
    (raw % 100_000) * 0x4000_0000 / 0xc350_0000 + (raw / 100_000) * 0x8000
}

fn value<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("fuel award operand unreadable or overflowed")
}
fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: usize) -> Result<(), &'static str> {
    if pointer == 0 || base.checked_add(rva).map(|p| p as u64) != read::<u64>(memory, pointer) {
        return Err("fuel award object identity mismatch");
    }
    Ok(())
}
/// The `(offset, width)` fields of `object`, in order, in one guarded copy
/// of exactly those spans: the first unreadable field refuses with the
/// message `value` gives, as reading them one by one would.
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
/// Where [`start`] found a fuel status's country: the owner it authenticated
/// through the country tables, and that country. A later read of the same
/// root in the same world and hour may take it ([`start_held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    pub owner: usize,
    pub country: ccountry::Country,
}

pub fn start(memory: &impl Memory, base: usize, status: usize) -> Result<Started, &'static str> {
    Ok(start_held(memory, base, status, None)?.0)
}

/// [`start`], taking `held` (this status's owner, located by the same
/// fuel-update invocation's BEGIN) when the status still names that owner: then
/// the owner is not authenticated through the country tables again (its id,
/// link, slot and identity reads). From BEGIN to END the fuel update and its
/// callees (resource and lend-lease helpers, unit consumption through the
/// cached consumption getter, allocation with its request builders, and receipt
/// writers) write fuel operands, unit fuel and temporary arrays only: no
/// country object, id, link or slot, and no reference registration. Every other
/// read is made in its native order, the backlink and instance included, so the
/// reads are a fresh read's with only that chain left out. Also returns whether
/// `held` was taken.
pub fn start_held(
    memory: &impl Memory,
    base: usize,
    status: usize,
    held: Option<&Located>,
) -> Result<(Started, Located, bool), &'static str> {
    auth(memory, base, status, fuel::VTABLE_RVA)?;
    let game_state = value::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
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
    Ok((started, Located { owner, country }, taken))
}
/// The status payload [`start_held`] reads after the identity chain, in one
/// guarded copy of its exact fields, as the [`Started`] of `country` and its
/// `country_instance`.
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

/// [`settle`], reading the status again through [`start_held`]; also returns whether it took
/// `held`. With `held` (located by this status's own BEGIN in the same invocation) and the same
/// status, only the payload is read again. Between BEGIN and END nothing writes a country object
/// (see [`start_held`]), and no fuel-update store writes the status's identity words: the initial
/// update writes only three operand fields and the ring through RSI before its tail jump; its
/// continuation writes the fields through the end of the operands through RBX up to END. Allocation
/// writes the status's first operand through R13, and the receipt writer passed the status writes
/// only through RDX, R10, R11 and the army. Its vtable, owner, the owner's backlink and instance
/// therefore read as BEGIN read them. The controls BEGIN's start passed (the game-state singleton
/// and both country headers, whose objects that range neither frees nor replaces) pass again:
/// omitted reads could only repeat BEGIN's values and outcomes. This holds for a fuel-update
/// invocation that runs from BEGIN to END without exceptional dispatch.
pub fn settle_held(
    memory: &impl Memory,
    base: usize,
    begun: Started,
    status: usize,
    held: Option<&Located>,
) -> Result<(Settled, bool), &'static str> {
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
    // These are differences of native book entries, never a tank-net estimate.
    // Bookkeeping adds the exact fixed5 input before aggregate conversion.
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
    // The three consumers' RECEIVED words, within the validated array.
    let receipt = |index: usize| {
        (
            index * crate::fuel_consumer_data::STRIDE + crate::fuel_consumer_data::RECEIVED,
            8,
        )
    };
    let received = fields(memory, consumers.data, [receipt(0), receipt(1), receipt(2)])?
        .map(i64::from_le_bytes);
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recipient {
    /// 0 division, 1 country/base air container, 2 task force (native consumers).
    pub kind: u32,
    pub reference: u64,
    /// Recipient's stored roster owner; the consuming country is separate.
    pub owner: u32,
    /// Air host reference, zero for a state base. Never guess a wing receipt.
    pub host: u64,
}
/// The reference registry headers one fuel-update invocation's unit
/// recipients resolve through: a header read successfully is taken again by
/// the later recipients of the same invocation, whose bucket probes are
/// still read. Nothing in that invocation registers a reference
/// ([`start_held`]). [`Self::default`] starts afresh.
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

/// [`unit`], resolving through `registry`; also returns whether it took a
/// header `registry` held.
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
pub fn air(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
) -> Result<Recipient, &'static str> {
    Ok(air_held(memory, base, pointer, container, consuming, None)?.0)
}

/// [`air`], taking `held` (the consuming country its fuel update
/// invocation's BEGIN located, [`start_held`]) as the country the consuming
/// id and, when it is the same id, the container's owner resolve to: those
/// two resolutions are not read again. Every other read is made in its
/// native order. Also returns whether `held` was taken.
pub fn air_held(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
    held: Option<&ccountry::Country>,
) -> Result<(Recipient, bool), &'static str> {
    let held = held.filter(|country| country.id == consuming);
    auth(memory, base, pointer, cair_base::VTABLE_RVA)?;
    auth(memory, base, container, ccountry_air_container::VTABLE_RVA)?;
    let reference = value::<u64>(memory, pointer, cair_base::HANDLE)?;
    if reference >> 32 == 0
        || creference_registry::resolve(memory, base, reference)? != Some(pointer)
    {
        return Err("fuel air base registry identity mismatch");
    }
    let game_state = value::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let owner = value::<u32>(memory, container, ccountry_air_container::COUNTRY)?;
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
    let containers = Array::read(
        memory,
        pointer
            .checked_add(cair_base::COUNTRY_CONTAINERS)
            .ok_or("fuel air container address overflow")?,
        8,
    )
    .ok_or("fuel air container vector invalid")?;
    if containers
        .element(country.canonical_slot, 8)
        .and_then(|p| read::<u64>(memory, p))
        != Some(container as u64)
    {
        return Err("fuel air container backlink mismatch");
    }
    Ok((
        Recipient {
            kind: 1,
            reference,
            owner,
            host: value(memory, pointer, cair_base::HOST_REFERENCE)?,
        },
        held.is_some(),
    ))
}

/// Off-hour log sizing from live request rosters, including duplicates.
/// Each queue route is bounded by its own roster count.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub entries: usize,
    pub names: usize,
    pub name_stride: usize,
}

pub struct StoredName(msvc_string::Descriptor);
impl StoredName {
    pub fn byte_len(&self) -> usize {
        self.0.len
    }
    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        self.0.copy(memory, output)
    }
}
/// Stored division and task-force name caches. Never run name generation;
/// the retained, possibly stale language bytes supply the displayed name.
pub fn stored_name(
    memory: &impl Memory,
    pointer: usize,
    kind: u32,
) -> Result<StoredName, &'static str> {
    let address = match kind {
        0 => (value::<u64>(
            memory,
            pointer,
            crate::carmy_combat_inputs::NAME_GROUP_MEMBER,
        )? as usize)
            .checked_add(0x60)
            .ok_or("fuel recipient name address overflow")?,
        2 => pointer
            .checked_add(0x258)
            .ok_or("fuel recipient name address overflow")?,
        _ => return Err("fuel recipient has no native unit name"),
    };
    msvc_string::Descriptor::read(memory, address).map(StoredName)
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
