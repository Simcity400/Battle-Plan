//! Aircraft-loss bookkeeping retaining each event's hour, region, family, cause
//! and opponent. Midnight differences can recover totals only when both advances
//! are witnessed; overlapping field ordinals prevent recovery of event context.
//! The daily pass advances only its retained country roster. A normal midnight
//! advance finalizes the preceding daily bucket after hourly writes. Advances
//! wrap at capacity and zero the new current record. The initial incomplete slot,
//! attach/load and partial-day roster membership prevent assigning dates to every
//! previous slot. Roll-ups modify higher windows, independently of completed
//! daily records.
//!
//! Each event writes the signed fixed5 count to its selected family total and
//! the total-plus-cause field. Unknown families select base zero. The reader
//! retains both fields with family, cause and own/credited context because field
//! indices overlap across families and other channels. Credited accident losses
//! perform no write; own losses still include them. Causes retain their raw
//! integers: aircraft combat, anti-air or raid damage, accident, airbase bombing,
//! train anti-air and naval anti-air against kamikaze.
//!
//! The count is bookkeeping rather than an independent inventory-removal witness.
//! Removal can exhaust a variant bank before fulfilling the request, yet loss
//! bookkeeping receives the original count. A saturated counter cannot certify
//! the missing amount. Airbase bombing can remove aircraft without loss
//! bookkeeping when no attacking wing exists. The collector refuses increments
//! that disagree with the two expected signed writes, including positive counts
//! at the clamp, and validates field indices independently. Synthetic coverage
//! establishes layouts and arithmetic; live hook behavior, fresh-save agreement
//! and gameplay cost are unverified.

use crate::{
    Memory, cair_wing, cair_wing_pool, ccountry, cloop_history, cpdx_array::Array, cstrategic_air,
    csub_unit_definition, loop_history_record_fields, loop_history_window, read,
};

/// Seven-argument writer: manager entry, wing, count, enemy, region, selection, cause.
pub const WRITER_RVA: usize = 0x00c4_d180;
/// Field count used by air-history construction.
pub const FIELD_COUNT: usize = cloop_history::air_stats::FIELD_COUNT as usize;
/// Own/enemy family bases; unknown families select zero.
pub const FAMILY_BASES: [u32; 13] = [0, 3, 10, 17, 24, 30, 40, 44, 51, 58, 65, 71, 81];

pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Both direct callsites in the loss dispatcher. Tapping these calls
/// preserves the writer prologue and system unwind metadata.
pub const SITES: [Site; 2] = [
    Site {
        name: "own_air_loss",
        rva: 0x00c4_d499,
        sentinel: [
            0xe8, 0xe2, 0xfc, 0xff, 0xff, 0x8b, 0x06, 0x8b, 0x17, 0x3b, 0xc2, 0x74, 0x48, 0x85,
            0xc0, 0x74,
        ],
    },
    Site {
        name: "credited_air_loss",
        rva: 0x00c4_d4e9,
        sentinel: [
            0xe8, 0x92, 0xfc, 0xff, 0xff, 0x48, 0x8b, 0xcf, 0xe8, 0x3a, 0x9d, 0x06, 0x00, 0x45,
            0x85, 0xff,
        ],
    },
];

/// Read family-base selection in its original predicate order. Masks describe
/// content-independent predicates. A fully readable unknown family returns
/// zero, as bookkeeping does when its assertion does not terminate execution.
pub fn selected_family(
    memory: &impl Memory,
    base: usize,
    wing: usize,
    enemy: u8,
) -> Result<u32, &'static str> {
    if enemy > 1 {
        return Err("air loss writer enemy flag is not boolean");
    }
    authenticate(memory, base, wing, cair_wing::VTABLE_RVA)?;
    let pool = pointer(memory, wing, cair_wing::POOL)?;
    authenticate(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
    let definition = pointer(memory, pool, cair_wing_pool::DEFINITION)?;
    let vtable = read::<u64>(memory, definition).ok_or("air loss definition vtable unreadable")?;
    if ![
        csub_unit_definition::VTABLE_RVA,
        csub_unit_definition::NULL_VTABLE_RVA,
    ]
    .into_iter()
    .any(|rva| base.checked_add(rva).map(|address| address as u64) == Some(vtable))
    {
        return Err("air loss definition form unsupported");
    }
    let flags = field::<u64>(memory, definition, csub_unit_definition::AIR_FLAGS)?;
    Ok(family_base(flags, enemy))
}

fn family_base(flags: u64, enemy: u8) -> u32 {
    if flags & 0x1_0000_0c00 != 0 {
        // Fighter-family predicate.
        [3, 44][enemy as usize]
    } else if flags & 0x0020_b000 != 0 || flags & (1 << 17) != 0 {
        // Bomber-family predicates.
        [10, 51][enemy as usize]
    } else if flags & (1 << 14) != 0 {
        // Close-air-support predicate.
        [17, 58][enemy as usize]
    } else if flags & (1 << 20) != 0 {
        // Transport predicate.
        [24, 65][enemy as usize]
    } else if flags & (1 << 18) != 0 {
        // Scout predicate.
        [30, 71][enemy as usize]
    } else if flags & 0x1c_0001_0000 != 0 {
        // Missile-family predicate.
        [40, 81][enemy as usize]
    } else {
        0 // Unknown families retain the initial zero base.
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub country: u32,
    pub region: u32,
    pub enemy: u8,
    pub cause: i32,
    pub family_base: u32,
    pub fields: [i64; 2],
    receiver: usize,
    history: usize,
    record: usize,
    slot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub country: u32,
    pub region: u32,
    /// Authenticated post-call daily cursor, including a history newly allocated
    /// by this call.
    pub daily_slot: u32,
    pub enemy: u8,
    pub cause: i32,
    /// Actual after-minus-before native fields, including native clamp effects.
    pub fields: [i64; 2],
}

/// Called immediately before the writer. A missing history is a known zero
/// only here: construction zeroes its current record before the first add.
/// A null region or credited accident performs no field write.
#[allow(clippy::too_many_arguments)]
pub fn before(
    memory: &impl Memory,
    base: usize,
    game: usize,
    receiver: usize,
    region: u32,
    enemy: u8,
    cause: i32,
    family_base: u32,
) -> Result<Snapshot, &'static str> {
    if enemy > 1 {
        return Err("air loss writer enemy flag is not boolean");
    }
    selected_indices(family_base, cause)?;
    let mut snapshot = Snapshot {
        country: 0,
        region,
        enemy,
        cause,
        family_base,
        fields: [0; 2],
        receiver,
        history: 0,
        record: 0,
        slot: 0,
    };
    snapshot.country = owner(memory, base, game, receiver)?;
    snapshot.history = history(memory, receiver, snapshot.region)?;
    if snapshot.history != 0 {
        let current = current(memory, base, snapshot.history, family_base, cause)?;
        snapshot.record = current.record;
        snapshot.slot = current.slot;
        snapshot.fields = current.fields;
    }
    Ok(snapshot)
}

/// Called immediately after the same synchronous native writer returns.
/// An identity or current-record change refuses the event instead of assigning
/// an unrelated record's values to the writer. This function allocates nothing.
pub fn after(
    memory: &impl Memory,
    base: usize,
    game: usize,
    before: &Snapshot,
) -> Result<Change, &'static str> {
    let mut change = Change {
        country: before.country,
        region: before.region,
        daily_slot: 0,
        enemy: before.enemy,
        cause: before.cause,
        fields: [0; 2],
    };
    if owner(memory, base, game, before.receiver)? != before.country {
        return Err("air loss writer country changed during call");
    }
    let history = history(memory, before.receiver, before.region)?;
    if history == 0 || (before.history != 0 && history != before.history) {
        return Err("air loss writer history changed during call");
    }
    let current = current(memory, base, history, before.family_base, before.cause)?;
    if before.history != 0 && (before.record != current.record || before.slot != current.slot) {
        return Err("air loss writer current record changed during call");
    }
    change.daily_slot = current.slot;
    for ((delta, after), before) in change
        .fields
        .iter_mut()
        .zip(current.fields)
        .zip(before.fields)
    {
        *delta = after
            .checked_sub(before)
            .ok_or("air loss writer delta overflow")?;
    }
    Ok(change)
}

/// The writer adds to family and family-plus-cause, twice for cause zero.
pub fn selected_indices(family: u32, cause: i32) -> Result<[u32; 2], &'static str> {
    let second = i64::from(family) + i64::from(cause);
    if family as usize >= FIELD_COUNT || !(0..FIELD_COUNT as i64).contains(&second) {
        return Err("air loss cause field is outside native record");
    }
    Ok([family, second as u32])
}

/// The credited tag pointer is retained in RDI and the effective tag pointer
/// in RSI. Neither changes before either tap. Follow only the opposite tag
/// word, using the signed-positive gate.
pub fn opponent(
    memory: &impl Memory,
    tags: [usize; 2],
    enemy: u8,
) -> Result<Option<u32>, &'static str> {
    let pointer = tags[usize::from(enemy == 0)];
    let tag = read::<i32>(memory, pointer).ok_or("air loss opponent tag unreadable")?;
    Ok((tag > 0).then_some(tag as u32))
}

fn owner(
    memory: &impl Memory,
    base: usize,
    game: usize,
    receiver: usize,
) -> Result<u32, &'static str> {
    authenticate(memory, base, receiver, cstrategic_air::VTABLE_RVA)?;
    let country = field(memory, receiver, cstrategic_air::COUNTRY)?;
    ccountry::Reader::new(memory, base, game)?.resolve(country)?;
    Ok(country)
}

fn history(memory: &impl Memory, receiver: usize, region: u32) -> Result<usize, &'static str> {
    let histories = array(memory, receiver, cstrategic_air::HISTORY_TABLE)?;
    let slot = histories
        .element(region, 8)
        .ok_or("air loss writer region outside history table")?;
    pointer(memory, slot, 0)
}

struct Current {
    record: usize,
    slot: u32,
    fields: [i64; 2],
}

fn current(
    memory: &impl Memory,
    base: usize,
    history: usize,
    family: u32,
    cause: i32,
) -> Result<Current, &'static str> {
    authenticate(memory, base, history, cloop_history::VTABLE_RVA)?;
    if pointer(memory, history, cloop_history::SELF_REFERENCE)? != history {
        return Err("air loss writer history self reference mismatch");
    }
    if field::<u32>(memory, history, cloop_history::FIELD_COUNT)? != FIELD_COUNT as u32 {
        return Err("air loss writer history field count mismatch");
    }
    let window = pointer(memory, history, cloop_history::DAILY_WINDOW)?;
    authenticate(memory, base, window, loop_history_window::VTABLE_RVA)?;
    let records = array(memory, window, loop_history_window::RECORDS)?;
    let capacity = field::<u32>(memory, window, loop_history_window::CAPACITY)?;
    let slot = field::<u32>(memory, window, loop_history_window::CURRENT_SLOT)?;
    let wrapped = field::<u8>(memory, window, loop_history_window::WRAPPED)?;
    if capacity != cloop_history::DAILY_WINDOW_CAPACITY
        || records.count != capacity
        || slot >= capacity
        || wrapped > 1
    {
        return Err("air loss writer daily window is invalid");
    }
    if pointer(memory, window, loop_history_window::HISTORY)? != history {
        return Err("air loss writer daily window owner mismatch");
    }
    let record = pointer(memory, window, loop_history_window::RECORD_FIELDS)?;
    let slot_address = records
        .element(slot, 8)
        .ok_or("air loss writer current slot address overflow")?;
    if record == 0 || pointer(memory, slot_address, 0)? != record {
        return Err("air loss writer current record disagrees with slot");
    }
    if pointer(memory, record, loop_history_record_fields::HISTORY)? != history {
        return Err("air loss writer record owner mismatch");
    }
    let values = array(memory, record, loop_history_record_fields::VALUES)?;
    if values.count != FIELD_COUNT as u32 {
        return Err("air loss writer record field count mismatch");
    }
    let indices = selected_indices(family, cause)?;
    let mut fields = [0; 2];
    for (value, index) in fields.iter_mut().zip(indices) {
        let address = values
            .element(index, 8)
            .ok_or("air loss selected field outside record")?;
        *value = read(memory, address).ok_or("air loss selected field unreadable")?;
    }
    Ok(Current {
        record,
        slot,
        fields,
    })
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rva: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(rva)
        .ok_or("air loss writer vtable address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air loss writer object vtable unreadable or mismatched");
    }
    Ok(())
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("air loss writer field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "air loss writer pointer outside address space")
}

fn array(memory: &impl Memory, object: usize, offset: usize) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("air loss writer array unreadable or invalid")
}

/// Queue sizing only. Read every country's pool and wing-vector headers,
/// including countries omitted from the daily pass. A malformed header
/// refuses sizing; no wing is followed and no player/AI filter applies.
/// This does not establish complete event-count demand.
pub fn wing_count(memory: &impl Memory, base: usize, game: usize) -> Result<usize, &'static str> {
    let mut total = 0_usize;
    cstrategic_air::visit(memory, base, game, |root| {
        let pools = array(memory, root.address, cstrategic_air::POOLS)?;
        crate::cpdx_array::pointers(
            memory,
            pools,
            "air loss wing pool pointer unreadable",
            |_, pool| {
                if pool == 0 {
                    return Err("air loss wing pool is null");
                }
                authenticate(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
                let wings = array(memory, pool, cair_wing_pool::WINGS)?;
                total = total
                    .checked_add(wings.count as usize)
                    .ok_or("air loss wing count overflow")?;
                Ok(())
            },
        )
    })?;
    Ok(total)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RosterRow {
    pub country: u32,
    pub eligible: bool,
}

/// Read the retained signed country-slot roster used by the daily air pass,
/// without recomputing its owned-state or diplomacy predicate after other
/// daily work could change it. Every canonical country receives a true/false
/// membership witness.
pub fn roster(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut Vec<RosterRow>,
) -> Result<(), &'static str> {
    output.clear();
    let countries = ccountry::Reader::new(memory, base, game)?;
    let mut selected = crate::allocation::filled(countries.count() as usize, false)
        .map_err(|_| "air daily roster allocation failed")?;
    let slots = countries.arrays()[0];
    let active = game
        .checked_add(crate::cgame_state::DAILY_COUNTRY_ROSTER)
        .and_then(|a| Array::read(memory, a, 4))
        .ok_or("air daily roster header invalid")?;
    for ordinal in 0..active.count {
        let address = active
            .element(ordinal, 4)
            .ok_or("air daily roster address overflow")?;
        let slot = read::<i32>(memory, address)
            .filter(|slot| *slot > 0)
            .ok_or("air daily roster country slot invalid")?;
        let address = slots
            .element(slot as u32, 8)
            .ok_or("air daily roster country slot outside country table")?;
        let country = countries.of_pointer(pointer(memory, address, 0)?)?;
        selected[country.canonical_slot as usize] = true;
    }
    crate::allocation::reserve_exact(output, countries.count() as usize)
        .map_err(|_| "air daily roster allocation failed")?;
    for slot in 1..countries.count() {
        if let Some(country) = countries.at(slot)? {
            output.push(RosterRow {
                country: country.id,
                eligible: selected[slot as usize],
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
