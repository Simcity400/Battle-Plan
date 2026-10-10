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
    0x010e_2643,
    [
        0x48, 0x8b, 0x5e, 0x18, 0x48, 0x03, 0x5e, 0x20, 0x4c, 0x39, 0x6e, 0x38, 0x7d, 0x14, 0x48,
        0x8b,
    ],
);
/// All awards and Q15 consumer debits have settled, before pending/ring clear.
/// Post-update eight-byte load span before the original clear continuation.
pub const END: (usize, [u8; 16]) = (
    0x010e_3453,
    [
        0x48, 0x8b, 0x4b, 0x48, 0x48, 0x8b, 0x41, 0x40, 0x48, 0x01, 0x83, 0xa8, 0, 0, 0, 0x48,
    ],
);
/// Division receipt writes, including unrolled and residual paths. RAX is the
/// division, RDX the actual signed fixed5 receipt with parity rounding. Chained
/// unwind entries lead to a handler-free body; each site+5 remains in its original entry.
pub const LAND: [(usize, [u8; 16]); 5] = [
    (
        0x010d_e1ad,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x010d_e1f2,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x010d_e237,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x49, 0x81, 0xf9, 0xa0, 0x86, 1,
        ],
    ),
    (
        0x010d_e27d,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x48, 0x83, 0xed, 1, 0x0f, 0x85,
        ],
    ),
    (
        0x010d_e2ea,
        [
            0x48, 1, 0x90, 0x20, 6, 0, 0, 0x49, 1, 0x13, 0x48, 0x83, 0xef, 1, 0x75, 0xb2,
        ],
    ),
];
/// Complete receipt addition, preserving the original return. RCX task force,
/// RDX signed fixed5 receipt. This leaf has no unwind entry or handler.
pub const NAVAL: (usize, [u8; 16]) = (
    0x00d4_ff10,
    [
        0x48, 1, 0x91, 0xf8, 4, 0, 0, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
    ],
);
/// Non-null-container receipt store: RDI base, RDX container, RBX receipt.
/// The seven-byte store ends before register restoration; site+5 remains
/// inside the handler-free body.
pub const AIR: (usize, [u8; 16]) = (
    0x00c3_c401,
    [
        0x48, 0x89, 0x9a, 0xf0, 0, 0, 0, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x83, 0xc4, 0x20,
    ],
);
/// Unconditional fuel-reset prefix, RSI status. Appearance reset reaches it
/// during runtime reset, country reuse and initialization; a pointer and zero
/// identity can recur. The eight-byte span follows the handler-free prologue
/// and contains no interior branch or return address. site+5 shares its unwind state.
pub const RESET: (usize, [u8; 16]) = (
    0x010d_dc69,
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
pub fn start(memory: &impl Memory, base: usize, status: usize) -> Result<Started, &'static str> {
    auth(memory, base, status, fuel::VTABLE_RVA)?;
    let game_state = value::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let owner = value::<u64>(memory, status, fuel::COUNTRY)? as usize;
    let country = countries.of_pointer(owner)?;
    if value::<u64>(memory, owner, ccountry::FUEL_STATUS)? != status as u64 {
        return Err("fuel award owner backlink mismatch");
    }
    Ok(Started {
        status,
        country: country.id,
        country_instance: value(memory, owner, ccountry::ID_INSTANCE)?,
        pending_gain: value(memory, status, 0x68)?,
        pending_consumed: value(memory, status, 0x70)?,
        pending_other: value(memory, status, 0x90)?,
        base: value(memory, status, fuel::GAIN)?,
        states: value(memory, status, fuel::GAIN_FROM_STATES)?,
        cost: value(memory, status, fuel::COST)?,
        lend_lease_in: value(memory, status, fuel::LEND_LEASE_GAIN)?,
        history_index: value(memory, status, fuel::HISTORY_INDEX)?,
        remaining_hours: value(memory, status, fuel::REMAINING_HOURS)?,
    })
}
pub fn settle(
    memory: &impl Memory,
    base: usize,
    begun: Started,
    status: usize,
) -> Result<Settled, &'static str> {
    let now = start(memory, base, status)?;
    if now.status != begun.status
        || now.country != begun.country
        || now.country_instance != begun.country_instance
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
    let mut received = [0; 3];
    for (index, out) in received.iter_mut().enumerate() {
        *out = value(
            memory,
            consumers
                .element(index as u32, crate::fuel_consumer_data::STRIDE)
                .ok_or("fuel award consumer address overflow")?,
            crate::fuel_consumer_data::RECEIVED,
        )?;
    }
    Ok(Settled {
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
    })
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
pub fn unit(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    kind: u32,
) -> Result<Recipient, &'static str> {
    let class = match kind {
        0 => 0,
        2 => 1,
        _ => return Err("fuel recipient kind unsupported"),
    };
    let mut registry = creference_registry::HeaderResolver::default();
    let (reference, actual_class) = cunit::counter_identity(memory, base, pointer, &mut registry)?;
    if actual_class != class {
        return Err("fuel recipient unit class mismatch");
    }
    Ok(Recipient {
        kind,
        reference,
        owner: value(memory, pointer, cunit::ROSTER_OWNER)?,
        host: 0,
    })
}
pub fn air(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    container: usize,
    consuming: u32,
) -> Result<Recipient, &'static str> {
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
    let country = countries.resolve(consuming)?;
    if countries.resolve(owner)?.pointer != country.pointer {
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
    Ok(Recipient {
        kind: 1,
        reference,
        owner,
        host: value(memory, pointer, cair_base::HOST_REFERENCE)?,
    })
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
