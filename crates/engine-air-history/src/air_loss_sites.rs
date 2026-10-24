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
pub const WRITER_RVA: usize = 0x00c5_c8c0;
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
        rva: 0x00c5_cbd9,
        sentinel: [
            0xe8, 0xe2, 0xfc, 0xff, 0xff, 0x8b, 0x06, 0x8b, 0x17, 0x3b, 0xc2, 0x74, 0x48, 0x85,
            0xc0, 0x74,
        ],
    },
    Site {
        name: "credited_air_loss",
        rva: 0x00c5_cc29,
        sentinel: [
            0xe8, 0x92, 0xfc, 0xff, 0xff, 0x48, 0x8b, 0xcf, 0xe8, 0x4a, 0x9d, 0x06, 0x00, 0x45,
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
    window: usize,
    record: usize,
    slot: u32,
    /// The two selected words of `record`.
    addresses: [usize; 2],
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
        window: 0,
        record: 0,
        slot: 0,
        addresses: [0; 2],
    };
    snapshot.country = owner(memory, base, game, receiver)?;
    snapshot.history = history(memory, receiver, snapshot.region)?;
    if snapshot.history != 0 {
        let current = current(memory, base, snapshot.history, family_base, cause)?;
        snapshot.window = current.window;
        snapshot.record = current.record;
        snapshot.slot = current.slot;
        snapshot.fields = current.fields;
        snapshot.addresses = current.addresses;
    }
    Ok(snapshot)
}

/// Called immediately after the same synchronous native writer returns. A
/// current-record change refuses the event instead of assigning an unrelated
/// record's values to the writer. This function allocates nothing.
///
/// With a history before the call, the writer performs clamped additions to only the two selected
/// words of its current record. History lookup returns the existing history unchanged;
/// region-combat lookup writes only the country air object's region-combat table, and the record
/// updater writes only its region-combat data. None touches the receiver, its history table or that
/// history. One guarded copy therefore takes the window's current slot and record again as the
/// witness that no daily advance ran, and a second takes the two words. Each copies exactly those
/// fields in their old order. A history the call created is authenticated in full, as `before`
/// would.
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
    let (slot, fields) = if before.history != 0 {
        // The slot and record pointer, in order, in one guarded copy of
        // exactly those spans.
        let mut slot = [0; 4];
        let mut record = [0; 8];
        const UNREADABLE: &str = "air loss writer field unreadable or address overflow";
        let at = |offset| before.window.checked_add(offset).ok_or(UNREADABLE);
        let addresses = [
            at(loop_history_window::CURRENT_SLOT)?,
            at(loop_history_window::RECORD_FIELDS)?,
        ];
        if memory.copy_scattered(addresses, &mut [&mut slot[..], &mut record[..]]) != 2 {
            return Err(UNREADABLE);
        }
        let slot = u32::from_le_bytes(slot);
        let record = u64::from_le_bytes(record) as usize;
        if record != before.record || slot != before.slot {
            return Err("air loss writer current record changed during call");
        }
        (slot, words(memory, before.addresses)?)
    } else {
        if owner(memory, base, game, before.receiver)? != before.country {
            return Err("air loss writer country changed during call");
        }
        let history = history(memory, before.receiver, before.region)?;
        if history == 0 {
            return Err("air loss writer history changed during call");
        }
        let current = current(memory, base, history, before.family_base, before.cause)?;
        (current.slot, current.fields)
    };
    change.daily_slot = slot;
    for ((delta, after), before) in change.fields.iter_mut().zip(fields).zip(before.fields) {
        *delta = after
            .checked_sub(before)
            .ok_or("air loss writer delta overflow")?;
    }
    Ok(change)
}

/// One healthy writer call's operands, as the hook's serial reads take them
/// ([`selected_family`], [`before`], [`opponent`] and the region and game
/// roots): the same values in six gathered guarded copies plus the country
/// resolution. Each object's fields are copied together, and fields a later
/// object needs are read ahead of the checks the serial reads make first;
/// every value is used only after the serial checks pass, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gathered {
    pub region: u32,
    pub game: usize,
    pub hour: i32,
    pub family: u32,
    pub snapshot: Snapshot,
    pub opponent: Option<u32>,
}

/// [`Gathered`] for one call: `region` is the region pointer argument,
/// `singleton` the game root's address. `None` when any copy fails or any
/// check the serial reads make refuses, a new history included: the caller
/// then takes the serial reads once, which meet a persistent fault where
/// they always did, so a fault never refuses more than they would.
#[allow(clippy::too_many_arguments)]
pub fn gather(
    memory: &impl Memory,
    base: usize,
    singleton: usize,
    receiver: usize,
    wing: usize,
    region: usize,
    enemy: u8,
    cause: i32,
    tags: [usize; 2],
) -> Option<Gathered> {
    if enemy > 1 || region == 0 || receiver == 0 || wing == 0 {
        return None;
    }
    let vtable = |rva: usize| base.checked_add(rva).map(|at| at as u64);
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let half = |bytes: &[u8]| u32::from_le_bytes(bytes[..4].try_into().unwrap());

    // The arguments' objects: region identity, game root, wing, receiver,
    // and the opponent tag word.
    let (mut region_vtable, mut region_id, mut game) = ([0; 8], [0; 4], [0; 8]);
    let (mut wing_vtable, mut pool) = ([0; 8], [0; 8]);
    let (mut receiver_vtable, mut country, mut histories) = ([0; 8], [0; 4], [0; 16]);
    let mut tag = [0; 4];
    let copied = memory.copy_scattered(
        [
            region,
            region.checked_add(crate::cstrategic_region::ID)?,
            singleton,
            wing,
            wing.checked_add(cair_wing::POOL)?,
            receiver,
            receiver.checked_add(cstrategic_air::COUNTRY)?,
            receiver.checked_add(cstrategic_air::HISTORY_TABLE)?,
            tags[usize::from(enemy == 0)],
        ],
        &mut [
            &mut region_vtable,
            &mut region_id,
            &mut game,
            &mut wing_vtable,
            &mut pool,
            &mut receiver_vtable,
            &mut country,
            &mut histories,
            &mut tag,
        ],
    );
    if copied != 9 || word(&region_vtable) != vtable(crate::cstrategic_region::VTABLE_RVA)? {
        return None;
    }
    let region_id = half(&region_id);
    let game = usize::try_from(word(&game))
        .ok()
        .filter(|&game| game != 0)?;
    let pool = usize::try_from(word(&pool)).ok()?;
    let histories = Array::from_header(histories, 8)?;
    if word(&wing_vtable) != vtable(cair_wing::VTABLE_RVA)? || pool == 0 {
        return None;
    }

    // The hour, the pool and the receiver's history slot.
    let (mut hour, mut pool_vtable, mut definition, mut history) = ([0; 4], [0; 8], [0; 8], [0; 8]);
    let copied = memory.copy_scattered(
        [
            game.checked_add(crate::cgame_state::CURRENT_HOUR)?,
            pool,
            pool.checked_add(cair_wing_pool::DEFINITION)?,
            histories.element(region_id, 8)?,
        ],
        &mut [&mut hour, &mut pool_vtable, &mut definition, &mut history],
    );
    if copied != 4 || word(&pool_vtable) != vtable(cair_wing_pool::VTABLE_RVA)? {
        return None;
    }
    let definition = usize::try_from(word(&definition)).ok()?;
    let history = usize::try_from(word(&history)).ok().filter(|&h| h != 0)?;

    // The definition and the history.
    let (mut definition_vtable, mut flags) = ([0; 8], [0; 8]);
    let (mut history_vtable, mut own, mut count, mut window) = ([0; 8], [0; 8], [0; 4], [0; 8]);
    let copied = memory.copy_scattered(
        [
            definition,
            definition.checked_add(csub_unit_definition::AIR_FLAGS)?,
            history,
            history.checked_add(cloop_history::SELF_REFERENCE)?,
            history.checked_add(cloop_history::FIELD_COUNT)?,
            history.checked_add(cloop_history::DAILY_WINDOW)?,
        ],
        &mut [
            &mut definition_vtable,
            &mut flags,
            &mut history_vtable,
            &mut own,
            &mut count,
            &mut window,
        ],
    );
    if copied != 6
        || ![
            csub_unit_definition::VTABLE_RVA,
            csub_unit_definition::NULL_VTABLE_RVA,
        ]
        .into_iter()
        .any(|rva| vtable(rva) == Some(word(&definition_vtable)))
    {
        return None;
    }
    let family = family_base(word(&flags), enemy);
    let indices = selected_indices(family, cause).ok()?;
    // The serial owner: authentication, then the country's resolution.
    if word(&receiver_vtable) != vtable(cstrategic_air::VTABLE_RVA)? {
        return None;
    }
    let country = half(&country);
    crate::ccountry::Reader::new(memory, base, game)
        .and_then(|countries| countries.resolve(country))
        .ok()?;
    if word(&history_vtable) != vtable(cloop_history::VTABLE_RVA)?
        || usize::try_from(word(&own)).ok()? != history
        || half(&count) != FIELD_COUNT as u32
    {
        return None;
    }
    let window = usize::try_from(word(&window)).ok().filter(|&w| w != 0)?;

    // The daily window.
    let (mut window_vtable, mut records, mut capacity, mut slot) =
        ([0; 8], [0; 16], [0; 4], [0; 4]);
    let (mut wrapped, mut owner, mut record) = ([0; 1], [0; 8], [0; 8]);
    let copied = memory.copy_scattered(
        [
            window,
            window.checked_add(loop_history_window::RECORDS)?,
            window.checked_add(loop_history_window::CAPACITY)?,
            window.checked_add(loop_history_window::CURRENT_SLOT)?,
            window.checked_add(loop_history_window::WRAPPED)?,
            window.checked_add(loop_history_window::HISTORY)?,
            window.checked_add(loop_history_window::RECORD_FIELDS)?,
        ],
        &mut [
            &mut window_vtable,
            &mut records,
            &mut capacity,
            &mut slot,
            &mut wrapped,
            &mut owner,
            &mut record,
        ],
    );
    let records = Array::from_header(records, 8)?;
    let (capacity, slot) = (half(&capacity), half(&slot));
    if copied != 7
        || word(&window_vtable) != vtable(loop_history_window::VTABLE_RVA)?
        || capacity != cloop_history::DAILY_WINDOW_CAPACITY
        || records.count != capacity
        || slot >= capacity
        || wrapped[0] > 1
        || usize::try_from(word(&owner)).ok()? != history
    {
        return None;
    }
    let record = usize::try_from(word(&record)).ok().filter(|&r| r != 0)?;

    // The current slot's record pointer and the record.
    let (mut current, mut record_history, mut values) = ([0; 8], [0; 8], [0; 16]);
    let copied = memory.copy_scattered(
        [
            records.element(slot, 8)?,
            record.checked_add(loop_history_record_fields::HISTORY)?,
            record.checked_add(loop_history_record_fields::VALUES)?,
        ],
        &mut [&mut current, &mut record_history, &mut values],
    );
    let values = Array::from_header(values, 8)?;
    if copied != 3
        || usize::try_from(word(&current)).ok()? != record
        || usize::try_from(word(&record_history)).ok()? != history
        || values.count != FIELD_COUNT as u32
    {
        return None;
    }
    let addresses = indices.map(|index| values.element(index, 8));
    let addresses = [addresses[0]?, addresses[1]?];
    let fields = words(memory, addresses).ok()?;
    let tag = half(&tag) as i32;
    Some(Gathered {
        region: region_id,
        game,
        hour: i32::from_le_bytes(hour),
        family,
        snapshot: Snapshot {
            country,
            region: region_id,
            enemy,
            cause,
            family_base: family,
            fields,
            receiver,
            history,
            window,
            record,
            slot,
            addresses,
        },
        opponent: (tag > 0).then_some(tag as u32),
    })
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
    window: usize,
    record: usize,
    slot: u32,
    fields: [i64; 2],
    addresses: [usize; 2],
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
    let mut addresses = [0; 2];
    for (address, index) in addresses.iter_mut().zip(indices) {
        *address = values
            .element(index, 8)
            .ok_or("air loss selected field outside record")?;
    }
    Ok(Current {
        window,
        record,
        slot,
        fields: words(memory, addresses)?,
        addresses,
    })
}

/// The two selected words, in order, in one guarded copy of exactly those
/// spans: the first unreadable word refuses, as two reads would.
fn words(memory: &impl Memory, addresses: [usize; 2]) -> Result<[i64; 2], &'static str> {
    let mut words = [[0; 8]; 2];
    let [first, second] = &mut words;
    if memory.copy_scattered(addresses, &mut [&mut first[..], &mut second[..]]) != 2 {
        return Err("air loss selected field unreadable");
    }
    Ok(words.map(i64::from_le_bytes))
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
