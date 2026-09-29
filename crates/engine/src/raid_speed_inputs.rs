//! Stored inputs of `CalculateRaidUnitSpeed@141453880`, which
//! `CRaidInstance::HourlyUpdate@140FDA5A0` adds to the travelled distance
//! (`+0x50`) every phase-4 hour and the raid arrow divides by. The function
//! is pure; its branches, as the game takes them:
//!
//! - transport planes (`EssentialNeedsTransportPlanes@140A90AA0`): the largest
//!   stat 0x3e (`CEquipmentVariant+0x2F0`) over the raid pool's entries with
//!   a nonzero amount whose variant has the `air_transport` category (bit 20
//!   of `+0x408`); without one, the same over the owner's stockpile
//!   (`CCountry+0xF68` -> `CProductionStatus+0x200`); without either, 100000;
//! - convoys (`EssentialNeedsConvoys@140A90A80`): the NNavy defines only;
//! - otherwise the wing (the unit's `+8` reference) through
//!   `CAirWing::GetAceModifiedMaxSpeed@140F4B5E0`, else the army (the `+0`
//!   reference) stat block's `+0x290`.
//!
//! The rest is content: `NMilitary.LAND_SPEED_MODIFIER`,
//! `NNavy.NAVAL_SPEED_MODIFIER`, `NNavy.NAVAL_TRANSFER_BASE_SPEED`,
//! `NRaids.RAID_UNIT_SPEED_MULTIPLIER`, `NAir.ACE_WING_SIZE(_MAX_BONUS)` and
//! the raid type's `speed_multiplier`. `(x * 0x29F16B11C6D1E109 >> 64) >> 14`
//! with the sign correction is the compiled `x / 100000`.
//!
//! The transport stats are read here although VariantStats records stat
//! 0x3e: that table is daily, the game reads the stat every phase-4 hour, and
//! the stat's writers (`BD0560`/`BD0770`) are not proven daily-only.
//!
//! The ace's two country terms, `ace_effectiveness_factor` on its owner and
//! `air_ace_bonuses_factor` on the wing's country, are country modifier totals
//! the launch walk records every hour for every country
//! (`raid_launch_inputs::CommandPowerRow`); a row names the two countries by
//! canonical slot instead of reading the values again.
//!
//! No engine function is called.

use crate::{
    Memory, cace, cair_wing, ccountry, ccountry_raid_status, cequipment_variant,
    cequipment_variant_pool, cgame_state,
    cpdx_array::Array,
    cproduction_status, craid_instance, craid_system, creference_registry, cunit,
    modifier_accumulator,
    modifier_definition::{DefinitionMemo, effective_memo},
    modifier_keys, read,
    walk::Outcome,
};

/// `CAirWing+0x580`, the wing's maximum speed `GetAceModifiedMaxSpeed` scales.
pub const WING_MAX_SPEED: usize = 0x580;
/// The army stat block's maximum speed (`CUnit+0x138` -> `+0x290`).
pub const ARMY_MAX_SPEED: usize = 0x290;
/// `CRaidInstance+0x1A0`, the hour `End@140FD7600` stores (`CGameState+0x468`),
/// valid while `+0x1B0` is set.
pub const END_HOUR: usize = 0x1a0;
pub const END_PRESENT: usize = 0x1b0;
/// `CRaidInstance+0xC8`: the raid unit's army then air wing references.
pub const ARMY_REFERENCE: usize = craid_instance::UNIT;
pub const WING_REFERENCE: usize = craid_instance::UNIT + 8;
/// The air transport category bit of `CEquipmentVariant+0x408`.
pub const AIR_TRANSPORT: u64 = 0x10_0000;
pub const TRANSPORT_SPEED_STAT: usize = cequipment_variant::MAXIMUM_SPEED_CACHE_CANDIDATE;
/// `CAce+0x170` -> `+0x38` CModifier, whose accumulator is at `+0x10`.
pub const ACE_TYPE_ACCUMULATOR: usize = 0x38 + 0x10;
/// `CAirWing+0x38` -> `+0x40` -> `+0x90`: the pool's country when `+0x9C4` is not positive.
pub const WING_POOL_OWNER: usize = 0x40;
pub const POOL_OWNER_COUNTRY: usize = 0x90;
/// The resolved reference of a CUnit or CAirWing points `0x10` into the object.
const REFERENCE_OFFSET: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub reference_type: u32,
    pub id: u32,
    pub phase: i32,
    pub outcome: i32,
    /// [`END_HOUR`] when [`END_PRESENT`] is set.
    pub end_hour: Option<i32>,
    /// [`WING_MAX_SPEED`] when the wing reference resolves.
    pub wing_max_speed: Option<i64>,
    /// [`ARMY_MAX_SPEED`] when the army reference resolves.
    pub army_max_speed: Option<i64>,
    /// Stat 0x3e of each qualifying pool entry, in pool order, in the
    /// caller's arena.
    pub transport_start: usize,
    pub transport_len: usize,
    /// The raid pool had no qualifying entry: the list is the owner's stockpile's.
    pub transport_from_country: u8,
    /// Neither pool had one: the game asserts and uses 100000.
    pub transport_fallback: u8,
    /// The wing's ace reference (`+0x2E8`) resolves, and `CAce+0x158`.
    pub ace_present: u8,
    pub ace_alive: u8,
    /// Only for a living ace: key 2 on its modifiers (`+0x30`) and on its
    /// type's; its assigned wing's plane count (`+0xE8` resolved `+0x6C`,
    /// `None` when it does not resolve).
    pub ace_speed: i64,
    pub ace_type_speed: i64,
    pub ace_wing_planes: Option<i32>,
    /// Only for a living ace: the canonical slot of its owner (`+0xE0`), whose
    /// key 0x25a scales the ace's speed, and of the wing's country
    /// (`CAirWing+0x9C4` when positive, else `+0x38` -> `+0x40` -> `+0x90`),
    /// whose key 0x72 is added.
    pub ace_owner: Option<u32>,
    pub wing_country: Option<u32>,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.id) << 32) | u64::from(self.reference_type)
    }
}

pub struct Walk {
    pub outcome: Outcome,
    pub transport_len: usize,
    /// Transport speeds every live raid needs, fitted or not.
    pub required_transport: usize,
}

/// Whether a raid's speed inputs are recorded: phase 4, or ended (not
/// cancelled) at most an hour before `hour`, while its arrow's unit may still
/// be flying (`CRaidArrow::UpdateForRaid@141CD75A0` shows it until the
/// displayed progress reaches 0.999, render state that is not stored).
pub fn live(phase: i32, outcome: i32, end_hour: Option<i32>, hour: i32) -> bool {
    phase == craid_instance::PHASE_IN_PROGRESS
        || (phase == craid_instance::PHASE_ENDED
            && outcome != craid_instance::OUTCOME_CANCELED
            && end_hour.is_some_and(|end| end >= hour.saturating_sub(1)))
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read::<T>(memory, at))
        .ok_or("raid speed input unreadable")
}

fn effective(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    let value = modifier_accumulator::lookup_at(memory, accumulator, key)?;
    if !value.present {
        return Ok(0);
    }
    effective_memo(memory, base, key, value.raw, memo)
}

/// A pool's qualifying transport speeds (`items` at `+0x20`, count `+0x2C`,
/// `{variant*, i64 amount}`), appended to `arena` from `len`.
fn transports(
    memory: &impl Memory,
    pool: usize,
    arena: &mut [i64],
    len: &mut usize,
    required: &mut usize,
) -> Result<usize, &'static str> {
    let items = pool
        .checked_add(cequipment_variant_pool::ITEMS)
        .and_then(|at| read::<[u8; 16]>(memory, at))
        .and_then(|header| Array::from_header(header, 16))
        .ok_or("raid pool header unreadable or invalid")?;
    let mut found = 0;
    for index in 0..items.count {
        let entry = items
            .element(index, 16)
            .and_then(|at| read::<[u8; 16]>(memory, at))
            .ok_or("raid pool entry unreadable")?;
        let variant = u64::from_le_bytes(entry[..8].try_into().unwrap()) as usize;
        let amount = i64::from_le_bytes(entry[8..].try_into().unwrap());
        if amount == 0 || variant == 0 {
            continue;
        }
        let flags: u64 = field(memory, variant, cequipment_variant::CLASSIFIER_FLAGS)?;
        if flags & AIR_TRANSPORT == 0 {
            continue;
        }
        let speed: i64 = field(memory, variant, TRANSPORT_SPEED_STAT)?;
        *required += 1;
        if let Some(slot) = arena.get_mut(*len) {
            *slot = speed;
            *len += 1;
        }
        found += 1;
    }
    Ok(found)
}

/// A raw country reference's CCountry, `None` for zero or an unlinked id.
fn country(
    countries: &ccountry::Reader<'_, impl Memory>,
    raw: u32,
) -> Result<Option<ccountry::Country>, &'static str> {
    if raw == 0 {
        return Ok(None);
    }
    let Some(slot) = countries.link(raw)? else {
        return Ok(None);
    };
    countries.at(slot)
}

#[allow(clippy::too_many_arguments)]
fn read_row(
    memory: &impl Memory,
    base: usize,
    raid: usize,
    identity: (u32, u32, i32, i32, Option<i32>),
    countries: &ccountry::Reader<'_, impl Memory>,
    memo: &DefinitionMemo,
    arena: &mut [i64],
    walk: &mut Walk,
) -> Result<Row, &'static str> {
    let (reference_type, id, phase, outcome, end_hour) = identity;
    let mut row = Row {
        reference_type,
        id,
        phase,
        outcome,
        end_hour,
        ..Row::default()
    };
    let wing = creference_registry::resolve(memory, base, field(memory, raid, WING_REFERENCE)?)?
        .and_then(|pointer| pointer.checked_sub(REFERENCE_OFFSET))
        .filter(|&wing| wing != 0);
    let army = creference_registry::resolve(memory, base, field(memory, raid, ARMY_REFERENCE)?)?
        .and_then(|pointer| pointer.checked_sub(REFERENCE_OFFSET))
        .filter(|&army| army != 0);
    if let Some(army) = army {
        let stats: u64 = field(memory, army, cunit::STAT_BLOCK)?;
        row.army_max_speed = Some(field(memory, stats as usize, ARMY_MAX_SPEED)?);
    }
    // The transport lists: the raid's pool, else the owner's stockpile.
    row.transport_start = walk.transport_len;
    let pool = raid
        .checked_add(craid_instance::EQUIPMENT)
        .ok_or("raid pool address overflow")?;
    let mut found = transports(
        memory,
        pool,
        arena,
        &mut walk.transport_len,
        &mut walk.required_transport,
    )?;
    if found == 0 {
        let owner: u64 = field(memory, raid, craid_instance::OWNER_OBJECT)?;
        let raw: u32 = field(memory, owner as usize, 8)?;
        if let Some(country) = country(countries, raw)? {
            let status: u64 = field(memory, country.pointer, ccountry::PRODUCTION_STATUS)?;
            let stockpile = (status as usize)
                .checked_add(cproduction_status::STOCKPILE_POOL)
                .ok_or("stockpile address overflow")?;
            found = transports(
                memory,
                stockpile,
                arena,
                &mut walk.transport_len,
                &mut walk.required_transport,
            )?;
            row.transport_from_country = 1;
        }
        row.transport_fallback = u8::from(found == 0);
    }
    row.transport_len = walk.transport_len - row.transport_start;
    let Some(wing) = wing else {
        return Ok(row);
    };
    row.wing_max_speed = Some(field(memory, wing, WING_MAX_SPEED)?);
    let Some(ace) =
        creference_registry::resolve(memory, base, field(memory, wing, cair_wing::ACE_REFERENCE)?)?
    else {
        return Ok(row);
    };
    row.ace_present = 1;
    row.ace_alive = field(memory, ace, cace::ALIVE)?;
    if row.ace_alive == 0 {
        return Ok(row);
    }
    row.ace_speed = effective(
        memory,
        base,
        ace + cace::MODIFIERS,
        modifier_keys::AIR_MAXIMUM_SPEED_FACTOR,
        memo,
    )?;
    let kind: u64 = field(memory, ace, crate::raid_launch_inputs::ACE_TYPE_DEFINITION)?;
    row.ace_type_speed = effective(
        memory,
        base,
        (kind as usize)
            .checked_add(ACE_TYPE_ACCUMULATOR)
            .ok_or("ace type address overflow")?,
        modifier_keys::AIR_MAXIMUM_SPEED_FACTOR,
        memo,
    )?;
    row.ace_wing_planes = creference_registry::resolve(
        memory,
        base,
        field(memory, ace, cace::ASSIGNED_WING_REFERENCE)?,
    )?
    .filter(|&pointer| pointer != REFERENCE_OFFSET)
    .map(|pointer| field::<i32>(memory, pointer, 0x6c))
    .transpose()?;
    let owner: u64 = field(memory, ace, cace::OWNER_COUNTRY)?;
    let owner = usize::try_from(owner).map_err(|_| "ace owner address overflow")?;
    row.ace_owner = Some(countries.of_pointer(owner)?.canonical_slot);
    let override_country: i32 = field(memory, wing, cair_wing::COUNTRY_OVERRIDE)?;
    let raw = if override_country > 0 {
        override_country as u32
    } else {
        let pool: u64 = field(memory, wing, cair_wing::POOL)?;
        let holder: u64 = field(memory, pool as usize, WING_POOL_OWNER)?;
        field(memory, holder as usize, POOL_OWNER_COUNTRY)?
    };
    let country = country(countries, raw)?.ok_or("raid wing country unresolved")?;
    row.wing_country = Some(country.canonical_slot);
    Ok(row)
}

/// One raid instance the raid system holds, identified and authenticated.
#[derive(Debug, Clone, Copy)]
pub struct Raid {
    pub pointer: usize,
    pub reference_type: u32,
    pub id: u32,
    pub phase: i32,
    pub outcome: i32,
    pub end_hour: Option<i32>,
}

/// Every raid instance of the raid system's country statuses, in their
/// order, with the current hour (`CGameState+0x468`). Parents are copied 64
/// per guarded call; each instance is authenticated by its vtable.
pub fn each_raid(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut(i32, Raid) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let hour: i32 = field(memory, game_state, cgame_state::CURRENT_HOUR)?;
    let root: u64 = field(memory, game_state, cgame_state::RAID_SYSTEM)?;
    let root = root as usize;
    if root == 0 {
        return Err("raid system unreadable or null");
    }
    let expected = base
        .checked_add(craid_system::VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, root) != Some(expected as u64) {
        return Err("raid system vtable mismatch");
    }
    let roster = root
        .checked_add(craid_system::COUNTRY_STATUSES)
        .and_then(|at| Array::read(memory, at, ccountry_raid_status::STRIDE))
        .ok_or("raid parent array unreadable or invalid")?;
    let instance_vtable = base
        .checked_add(craid_instance::VTABLE_RVA)
        .ok_or("module address overflow")?;
    const BATCH: usize = 64;
    let mut records = [0_u8; BATCH * ccountry_raid_status::PARENT_BYTES];
    for first in (0..roster.count).step_by(BATCH) {
        let count = (roster.count - first).min(BATCH as u32) as usize;
        ccountry_raid_status::copy_parents(memory, roster, first, count, &mut records)?;
        for record in records[..count * ccountry_raid_status::PARENT_BYTES]
            .chunks_exact(ccountry_raid_status::PARENT_BYTES)
        {
            let instances = Array::from_header(record[12..28].try_into().unwrap(), 8)
                .ok_or("raid instance array invalid")?;
            for index in 0..instances.count {
                let raid: u64 = field(
                    memory,
                    instances
                        .element(index, 8)
                        .ok_or("raid instance pointer overflow")?,
                    0,
                )?;
                let raid = raid as usize;
                if raid == 0 || read::<u64>(memory, raid) != Some(instance_vtable as u64) {
                    return Err("raid instance pointer null or not a raid");
                }
                let present: u8 = field(memory, raid, END_PRESENT)?;
                visit(
                    hour,
                    Raid {
                        pointer: raid,
                        reference_type: field(memory, raid, craid_instance::REFERENCE_TYPE)?,
                        id: field(memory, raid, craid_instance::ID)?,
                        phase: field(memory, raid, craid_instance::PHASE)?,
                        outcome: field(memory, raid, craid_instance::OUTCOME)?,
                        end_hour: (present != 0)
                            .then(|| field::<i32>(memory, raid, END_HOUR))
                            .transpose()?,
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// Every live raid's speed inputs ([`live`]), in raid-system order. A raid
/// whose transport speeds do not fit in `arena` has no row and is counted.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    memo: &DefinitionMemo,
    rows: &mut [Row],
    arena: &mut [i64],
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        transport_len: 0,
        required_transport: 0,
    };
    memo.clear();
    let (mut len, mut unread) = (0, 0_u32);
    let result = ccountry::Reader::new(memory, base, game_state).and_then(|countries| {
        each_raid(memory, base, game_state, |hour, raid| {
            if !live(raid.phase, raid.outcome, raid.end_hour, hour) {
                return Ok(());
            }
            let identity = (
                raid.reference_type,
                raid.id,
                raid.phase,
                raid.outcome,
                raid.end_hour,
            );
            let before = (walk.transport_len, walk.required_transport);
            let row = read_row(
                memory,
                base,
                raid.pointer,
                identity,
                &countries,
                memo,
                arena,
                &mut walk,
            )?;
            let fitted = walk.required_transport - before.1 == walk.transport_len - before.0;
            match rows.get_mut(len) {
                Some(slot) if fitted => {
                    *slot = row;
                    len += 1;
                }
                _ => {
                    walk.transport_len = before.0;
                    unread += 1;
                }
            }
            Ok(())
        })
    });
    walk.outcome = match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if unread == 0 => Outcome::Rows { len },
        Ok(()) => Outcome::Truncated {
            len,
            unread_rows: unread,
        },
    };
    walk
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{creference_registry as registry, modifier_definition, reference_bucket as bucket};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Image(BTreeMap<usize, u8>);

    impl Memory for Image {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    impl Image {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
        }
        fn u8(&mut self, at: usize, value: u8) {
            self.put(at, &[value]);
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        /// A sorted accumulator of `(key, raw)` entries.
        fn accumulator(&mut self, at: usize, data: usize, entries: &[(u32, i64)]) {
            self.array(at, data, entries.len() as u32);
            for (index, &(key, raw)) in entries.iter().enumerate() {
                self.u64(data + index * 16, u64::from(key));
                self.u64(data + index * 16 + 8, raw as u64);
            }
        }
        /// One registry table per reference type, each holding `handle`.
        fn register(&mut self, table: usize, handle: u64, object: usize) {
            let kind = handle as u32;
            let root = if kind < registry::LOW_ROOT_COUNT {
                registry::LOW_ROOTS_RVA + kind as usize * 8
            } else if kind < registry::MID_TYPE_LIMIT {
                registry::MID_ROOT_RVA
            } else {
                registry::HIGH_ROOT_RVA
            };
            let entries = table + 0x100;
            self.u64(root, table as u64);
            self.u64(table + registry::ENTRIES, entries as u64);
            self.u32(table + registry::COUNT, 1);
            self.u32(table + registry::MASK, 7);
            self.u8(table + registry::OVERFLOW_DISTANCE, 5);
            self.put(entries, &[0; 14 * bucket::STRIDE]);
            self.u8(entries + 13 * bucket::STRIDE + bucket::DISTANCE, 255);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            let at = entries + index * bucket::STRIDE;
            self.u8(at + bucket::DISTANCE, 1);
            self.u64(at + bucket::TYPE, handle);
            self.u64(at + bucket::OBJECT, object as u64);
        }
    }

    const GS: usize = 0x1000;
    const ROOT: usize = 0x2000;
    const PARENTS: usize = 0x3000;
    const INSTANCES: usize = 0x4000;
    const RAID: usize = 0x5000;
    const COUNTRIES: usize = 0x7000;
    const LINKS: usize = 0x7100;
    const COUNTRY: usize = 0x8000;
    const OWNER: usize = 0x8800;
    const WING: usize = 0x2_0000;
    const ARMY: usize = 0x3_0000;
    const STATS: usize = 0x3_8000;
    const ACE: usize = 0x4_0000;
    const ACE_TYPE: usize = 0x4_8000;
    const TRANSPORT: usize = 0x5_0000;
    const RIFLE: usize = 0x5_8000;
    const DEFINITIONS: usize = 0x10_0000;
    const ARMY_HANDLE: u64 = (1 << 32) | 4713;
    const WING_HANDLE: u64 = (2 << 32) | 69;
    const ACE_HANDLE: u64 = (3 << 32) | 70;

    /// Hour 100; one phase-4 raid of country 2 (slot 1) flown by a wing of
    /// that country with a living ace owned by country 3 (slot 3), its army
    /// resolved too, and a pool of one transport and one rifle entry.
    fn fixture() -> Image {
        let mut m = Image::default();
        m.u32(GS + cgame_state::CURRENT_HOUR, 100);
        m.u64(GS + cgame_state::RAID_SYSTEM, ROOT as u64);
        m.u64(ROOT, craid_system::VTABLE_RVA as u64);
        m.array(ROOT + craid_system::COUNTRY_STATUSES, PARENTS, 1);
        m.put(PARENTS, &[0; ccountry_raid_status::STRIDE]);
        m.array(PARENTS + ccountry_raid_status::INSTANCES, INSTANCES, 1);
        m.u64(INSTANCES, RAID as u64);
        m.put(RAID, &[0; 0x200]);
        m.u64(RAID, craid_instance::VTABLE_RVA as u64);
        m.u32(RAID + craid_instance::REFERENCE_TYPE, 84);
        m.u32(RAID + craid_instance::ID, 9);
        m.u32(RAID + craid_instance::PHASE, 4);
        m.u64(RAID + ARMY_REFERENCE, ARMY_HANDLE);
        m.u64(RAID + WING_REFERENCE, WING_HANDLE);
        m.u64(RAID + craid_instance::OWNER_OBJECT, 0x6000);
        m.u32(0x6000 + 8, 2);
        let pool = RAID + craid_instance::EQUIPMENT;
        m.array(pool + cequipment_variant_pool::ITEMS, 0x6100, 2);
        m.u64(0x6100, TRANSPORT as u64);
        m.u64(0x6108, 5);
        m.u64(0x6110, RIFLE as u64);
        m.u64(0x6118, 7);
        m.u64(
            TRANSPORT + cequipment_variant::CLASSIFIER_FLAGS,
            AIR_TRANSPORT | 1,
        );
        m.u64(TRANSPORT + TRANSPORT_SPEED_STAT, 300_000);
        m.u64(RIFLE + cequipment_variant::CLASSIFIER_FLAGS, 0x20);
        m.u64(RIFLE + TRANSPORT_SPEED_STAT, 1);
        // Countries: slot 1 is country id 2, slot 3 country id 3, slot 2 is
        // null. Their modifier totals are not read here.
        m.array(GS + cgame_state::COUNTRIES, COUNTRIES, 4);
        m.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 4);
        m.u64(COUNTRIES + 8, COUNTRY as u64);
        m.u64(COUNTRIES + 16, 0);
        m.u64(COUNTRIES + 24, OWNER as u64);
        m.u32(LINKS + 4, 1);
        m.u32(LINKS + 8, 1);
        m.u32(LINKS + 12, 3);
        m.u64(COUNTRY, ccountry::VTABLE_RVA as u64);
        m.u32(COUNTRY + ccountry::ID, 2);
        m.u64(OWNER, ccountry::VTABLE_RVA as u64);
        m.u32(OWNER + ccountry::ID, 3);
        // Modifier metadata: the speed key enabled, unclamped.
        m.u64(modifier_definition::TABLE_RVA, DEFINITIONS as u64);
        let row = DEFINITIONS + 2 * modifier_definition::STRIDE;
        m.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
        m.u32(row + modifier_definition::FLAGS, 0);
        // The wing, its ace and the army; references point 0x10 in.
        m.register(0x6_0000, WING_HANDLE, WING + 0x10);
        m.register(0x6_1000, ARMY_HANDLE, ARMY + 0x10);
        m.register(0x6_2000, ACE_HANDLE, ACE);
        m.u64(WING + WING_MAX_SPEED, 1_200_000);
        m.u64(WING + cair_wing::ACE_REFERENCE, ACE_HANDLE);
        m.u32(WING + cair_wing::COUNTRY_OVERRIDE, 2);
        m.u32(WING + 0x7c, 50);
        m.u64(ARMY + cunit::STAT_BLOCK, STATS as u64);
        m.u64(STATS + ARMY_MAX_SPEED, 400_000);
        m.u8(ACE + cace::ALIVE, 1);
        m.accumulator(ACE + cace::MODIFIERS, 0x9100, &[(2, 20_000)]);
        m.u64(ACE + cace::OWNER_COUNTRY, OWNER as u64);
        m.u64(ACE + cace::ASSIGNED_WING_REFERENCE, WING_HANDLE);
        m.u64(
            ACE + crate::raid_launch_inputs::ACE_TYPE_DEFINITION,
            ACE_TYPE as u64,
        );
        m.accumulator(ACE_TYPE + ACE_TYPE_ACCUMULATOR, 0x9200, &[(2, 10_000)]);
        m
    }

    fn run(memory: &Image) -> (Walk, Vec<Row>, Vec<i64>) {
        let mut rows = vec![Row::default(); 2];
        let mut arena = vec![0; 4];
        let walk = walk(
            memory,
            0,
            GS,
            &DefinitionMemo::default(),
            &mut rows,
            &mut arena,
        );
        (walk, rows, arena)
    }

    #[test]
    fn a_live_raid_records_every_branchs_stored_inputs() {
        let (walk, rows, arena) = run(&fixture());
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        let row = rows[0];
        assert_eq!((row.key(), row.phase), ((9 << 32) | 84, 4));
        assert_eq!(
            (row.wing_max_speed, row.army_max_speed),
            (Some(1_200_000), Some(400_000))
        );
        // Only the transport entry qualifies, from the raid's own pool.
        assert_eq!(
            &arena[row.transport_start..][..row.transport_len],
            &[300_000]
        );
        assert_eq!((row.transport_from_country, row.transport_fallback), (0, 0));
        assert_eq!(
            (
                row.ace_present,
                row.ace_alive,
                row.ace_speed,
                row.ace_type_speed,
                row.ace_wing_planes,
                row.ace_owner,
                row.wing_country
            ),
            (1, 1, 20_000, 10_000, Some(50), Some(3), Some(1))
        );
        // Without the override the wing's country is its pool holder's.
        let mut memory = fixture();
        memory.u32(WING + cair_wing::COUNTRY_OVERRIDE, 0);
        memory.u64(WING + cair_wing::POOL, 0xb000);
        memory.u64(0xb000 + WING_POOL_OWNER, 0xb100);
        memory.u32(0xb100 + POOL_OWNER_COUNTRY, 3);
        let (_, rows, _) = run(&memory);
        assert_eq!(
            (rows[0].ace_owner, rows[0].wing_country),
            (Some(3), Some(3))
        );
        // An ace owner that is not a canonical country object fails the table.
        memory.u64(ACE + cace::OWNER_COUNTRY, COUNTRY as u64 + 8);
        assert!(matches!(
            run(&memory).0.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn transports_fall_back_to_the_owner_stockpile_then_to_none() {
        let mut memory = fixture();
        // No qualifying raid entry: the owner's stockpile is read.
        memory.u64(0x6108, 0);
        let production = 0xa000;
        memory.u64(COUNTRY + ccountry::PRODUCTION_STATUS, production as u64);
        let stockpile = production + cproduction_status::STOCKPILE_POOL;
        memory.array(stockpile + cequipment_variant_pool::ITEMS, 0xa800, 1);
        memory.u64(0xa800, TRANSPORT as u64);
        memory.u64(0xa808, 3);
        let (_, rows, arena) = run(&memory);
        assert_eq!(
            &arena[rows[0].transport_start..][..rows[0].transport_len],
            &[300_000]
        );
        assert_eq!(
            (rows[0].transport_from_country, rows[0].transport_fallback),
            (1, 0)
        );
        memory.u64(0xa808, 0);
        let (_, rows, _) = run(&memory);
        assert_eq!((rows[0].transport_len, rows[0].transport_fallback), (0, 1));
    }

    #[test]
    fn only_phase_four_and_just_ended_raids_are_live() {
        assert!(live(4, 0, None, 100));
        assert!(!live(3, 0, None, 100));
        assert!(live(5, 3, Some(99), 100));
        assert!(!live(5, 3, Some(98), 100));
        assert!(!live(5, craid_instance::OUTCOME_CANCELED, Some(100), 100));
        assert!(!live(5, 3, None, 100));
        let mut memory = fixture();
        memory.u32(RAID + craid_instance::PHASE, 2);
        assert_eq!(run(&memory).0.outcome, Outcome::Rows { len: 0 });
        memory.u32(RAID + craid_instance::PHASE, 5);
        memory.u32(RAID + craid_instance::OUTCOME, 3);
        memory.u8(RAID + END_PRESENT, 1);
        memory.u32(RAID + END_HOUR, 99);
        let (walk, rows, _) = run(&memory);
        assert_eq!(
            (walk.outcome, rows[0].end_hour),
            (Outcome::Rows { len: 1 }, Some(99))
        );
    }

    /// RaidEquipment: the raid's pool entries by variant reference in pool
    /// order, its archetype totals by token in native order (read as stored,
    /// not rebuilt from the entries) and its nuke count, before launch and
    /// while flying only.
    #[test]
    fn a_raid_pool_is_recorded_before_launch_and_while_flying() {
        use crate::{allocation_equipment_row as allocation, cequipment_type, raid_equipment};
        const ALLOCATIONS: usize = 0x6200;
        const ARCHETYPES: [usize; 2] = [0x7_0000, 0x7_1000];
        /// `AddVariantQuantity@140FF7A30`'s clamp: a saturated total.
        const SATURATED: i64 = 0x53e2_d620_fb40;
        let mut memory = fixture();
        for (variant, id) in [(TRANSPORT, 41_u64), (RIFLE, 42)] {
            memory.u64(variant, cequipment_variant::VTABLE_RVA as u64);
            memory.u64(
                variant + 8,
                (id << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
            );
        }
        memory.u32(RAID + craid_instance::NUKES, 2);
        let pool = RAID + craid_instance::EQUIPMENT;
        memory.array(pool + cequipment_variant_pool::ALLOCATIONS, ALLOCATIONS, 2);
        for (index, (archetype, token, total)) in [
            (ARCHETYPES[0], 0x3309_u32, 5_i64),
            (ARCHETYPES[1], 0x330a, SATURATED),
        ]
        .into_iter()
        .enumerate()
        {
            let entry = ALLOCATIONS + index * allocation::STRIDE;
            memory.put(entry, &[0; allocation::STRIDE]);
            memory.u64(entry + allocation::ARCHETYPE, archetype as u64);
            memory.u64(entry + allocation::AMOUNT, total as u64);
            memory.u64(archetype, cequipment_type::VTABLE_RVA as u64);
            memory.u32(archetype + cequipment_type::TOKEN, token);
        }
        struct Run {
            outcome: Outcome,
            row: raid_equipment::Row,
            variants: Vec<u64>,
            amounts: Vec<i64>,
            tokens: Vec<u32>,
            totals: Vec<i64>,
            required: (usize, usize),
        }
        let run = |memory: &Image, capacity: usize, archetype_capacity: usize| {
            let mut rows = [raid_equipment::Row::default(); 1];
            let (mut variants, mut amounts) = (vec![0; capacity], vec![0; capacity]);
            let (mut tokens, mut totals) =
                (vec![0; archetype_capacity], vec![0; archetype_capacity]);
            let walk = raid_equipment::walk(
                memory,
                0,
                GS,
                &mut rows,
                raid_equipment::Arenas {
                    variants: &mut variants,
                    amounts: &mut amounts,
                    archetype_tokens: &mut tokens,
                    archetype_amounts: &mut totals,
                },
            );
            Run {
                outcome: walk.outcome,
                row: rows[0],
                variants,
                amounts,
                tokens,
                totals,
                required: (walk.required_items, walk.required_archetypes),
            }
        };
        let reference = |id: u64| (id << 32) | u64::from(cequipment_variant::REFERENCE_TYPE);
        let full = run(&memory, 2, 2);
        assert_eq!(full.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (
                full.row.key(),
                full.row.nukes,
                full.row.items_len,
                full.row.archetypes_len
            ),
            ((9 << 32) | 84, 2, 2, 2)
        );
        assert_eq!(full.variants, [reference(41), reference(42)]);
        assert_eq!(full.amounts, [5, 7]);
        // The totals are the stored ones: the saturated second is not the
        // rifle entry's 7.
        assert_eq!(full.tokens, [0x3309, 0x330a]);
        assert_eq!(full.totals, [5, SATURATED]);
        // A pool whose entries or totals do not fit is counted with its
        // demand.
        for (capacity, archetype_capacity) in [(1, 2), (2, 1)] {
            let short = run(&memory, capacity, archetype_capacity);
            assert_eq!(
                (short.outcome, short.required),
                (
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1
                    },
                    (2, 2)
                )
            );
        }
        // A total whose archetype is not a CEquipmentType fails the table.
        memory.u64(ARCHETYPES[1], 0);
        assert!(matches!(
            run(&memory, 2, 2).outcome,
            Outcome::Unavailable { .. }
        ));
        memory.u64(ARCHETYPES[1], cequipment_type::VTABLE_RVA as u64);
        // Prepared raids are recorded; cancelled ended ones are not.
        memory.u32(RAID + craid_instance::PHASE, 3);
        assert_eq!(run(&memory, 2, 2).outcome, Outcome::Rows { len: 1 });
        memory.u32(RAID + craid_instance::PHASE, 5);
        memory.u32(
            RAID + craid_instance::OUTCOME,
            craid_instance::OUTCOME_CANCELED as u32,
        );
        assert_eq!(run(&memory, 2, 2).outcome, Outcome::Rows { len: 0 });
    }

    #[test]
    fn a_dead_ace_and_an_unresolved_army_leave_their_terms_out() {
        let mut memory = fixture();
        memory.u8(ACE + cace::ALIVE, 0);
        memory.u64(RAID + ARMY_REFERENCE, 0);
        let (_, rows, _) = run(&memory);
        assert_eq!(
            (
                rows[0].army_max_speed,
                rows[0].ace_present,
                rows[0].ace_alive,
                rows[0].ace_speed,
                rows[0].ace_owner,
                rows[0].wing_country
            ),
            (None, 1, 0, 0, None, None)
        );
    }
}
