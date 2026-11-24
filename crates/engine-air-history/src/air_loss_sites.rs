
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_air::cstrategic_air;
use engine_native_parts::{cair_wing_pool, csub_unit_definition};
use engine_native::ccountry;
use crate::{cloop_history, loop_history_record_fields, loop_history_window};
use engine_sites::Rva;

pub const WRITER_RVA: Rva = Rva(0x00cb_96a0);
pub const FIELD_COUNT: usize = cloop_history::air_stats::FIELD_COUNT as usize;
pub use engine_executable::native_values::air_loss::FAMILY_BASES;

pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}

pub const SITES: [Site; 2] = [
    Site {
        name: "own_air_loss",
        rva: Rva(0x00cb_99b9),
    },
    Site {
        name: "credited_air_loss",
        rva: Rva(0x00cb_9a09),
    },
];

pub fn selected_family(
    memory: &impl Memory,
    base: usize,
    wing: usize,
    enemy: u8,
) -> Result<u32, &'static str> {
    if enemy > 1 {
        return Err("air loss writer enemy flag is not boolean");
    }
    authenticate(memory, base, wing, engine_native_parts::cair_wing_layout::VTABLE_RVA)?;
    let pool = pointer(memory, wing, engine_native_parts::cair_wing_layout::POOL)?;
    authenticate(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
    let definition = pointer(memory, pool, cair_wing_pool::DEFINITION)?;
    let vtable = read::<u64>(memory, definition).ok_or("air loss definition vtable unreadable")?;
    let forms = [
        csub_unit_definition::VTABLE_RVA,
        csub_unit_definition::NULL_VTABLE_RVA,
    ];
    if engine_sites::which(base, vtable as usize, &forms).is_none() {
        return Err("air loss definition form unsupported");
    }
    let flags = field::<u64>(memory, definition, csub_unit_definition::AIR_FLAGS)?;
    Ok(family_base(flags, enemy))
}

fn family_base(flags: u64, enemy: u8) -> u32 {
    if flags & 0x1_0000_0c00 != 0 {
        [3, 44][enemy as usize]
    } else if flags & 0x0020_b000 != 0 || flags & (1 << 17) != 0 {
        [10, 51][enemy as usize]
    } else if flags & (1 << 14) != 0 {
        [17, 58][enemy as usize]
    } else if flags & (1 << 20) != 0 {
        [24, 65][enemy as usize]
    } else if flags & (1 << 18) != 0 {
        [30, 71][enemy as usize]
    } else if flags & 0x1c_0001_0000 != 0 {
        [40, 81][enemy as usize]
    } else {
        0
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
    addresses: [usize; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub country: u32,
    pub region: u32,
    pub daily_slot: u32,
    pub enemy: u8,
    pub cause: i32,
    pub fields: [i64; 2],
}

#[expect(
    clippy::too_many_arguments,
    reason = "the receiver, region, enemy, cause and family base are separate facts of one air loss"
)]
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

pub fn after(
    memory: &impl Memory,
    base: usize,
    game: usize,
    before: &Snapshot,
) -> Result<Change, &'static str> {
    after_with_singleton(memory, base, None, game, before)
}

pub fn after_checked(
    memory: &impl Memory,
    base: usize,
    singleton: usize,
    game: usize,
    before: &Snapshot,
) -> Result<Change, &'static str> {
    after_with_singleton(memory, base, Some(singleton), game, before)
}

fn after_with_singleton(
    memory: &impl Memory,
    base: usize,
    singleton: Option<usize>,
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
        let mut slot = [0; 4];
        let mut record = [0; 8];
        let mut field0 = [0; 8];
        let mut field1 = [0; 8];
        const UNREADABLE: &str = "air loss writer field unreadable or address overflow";
        let at = |offset| before.window.checked_add(offset).ok_or(UNREADABLE);
        let slot_addr = at(loop_history_window::CURRENT_SLOT)?;
        let record_addr = at(loop_history_window::RECORD_FIELDS)?;
        if let Some(singleton) = singleton {
            let mut root = [0; 8];
            if memory.copy_scattered(
                [
                    singleton,
                    slot_addr,
                    record_addr,
                    before.addresses[0],
                    before.addresses[1],
                ],
                &mut [
                    &mut root[..],
                    &mut slot[..],
                    &mut record[..],
                    &mut field0[..],
                    &mut field1[..],
                ],
            ) != 5
            {
                if read::<u64>(memory, singleton) != Some(game as u64) {
                    return Err("air loss game root changed during native call");
                }
                return Err(UNREADABLE);
            }
            if u64::from_le_bytes(root) != game as u64 {
                return Err("air loss game root changed during native call");
            }
        } else if memory.copy_scattered(
            [slot_addr, record_addr, before.addresses[0], before.addresses[1]],
            &mut [&mut slot[..], &mut record[..], &mut field0[..], &mut field1[..]],
        ) != 4
        {
            return Err(UNREADABLE);
        }
        let slot = u32::from_le_bytes(slot);
        let record = u64::from_le_bytes(record) as usize;
        if record != before.record || slot != before.slot {
            return Err("air loss writer current record changed during call");
        }
        (
            slot,
            [i64::from_le_bytes(field0), i64::from_le_bytes(field1)],
        )
    } else {
        if let Some(singleton) = singleton
            && read::<u64>(memory, singleton) != Some(game as u64)
        {
            return Err("air loss game root changed during native call");
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gathered {
    pub region: u32,
    pub game: usize,
    pub hour: i32,
    pub family: u32,
    pub snapshot: Snapshot,
    pub opponent: Option<u32>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the singleton, receiver, wing, region, enemy, cause and tags are separate facts of one air loss"
)]
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
    let vtable = |key: Rva| engine_sites::at(base, key).map(|at| at as u64);
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let half = |bytes: &[u8]| u32::from_le_bytes(bytes[..4].try_into().unwrap());

    let (mut region_vtable, mut region_id, mut game) = ([0; 8], [0; 4], [0; 8]);
    let (mut wing_vtable, mut pool) = ([0; 8], [0; 8]);
    let (mut receiver_vtable, mut country, mut histories) = ([0; 8], [0; 4], [0; 16]);
    let mut tag = [0; 4];
    let copied = memory.copy_scattered(
        [
            region,
            region.checked_add(engine_native_parts::cstrategic_region::ID)?,
            singleton,
            wing,
            wing.checked_add(engine_native_parts::cair_wing_layout::POOL)?,
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
    if copied != 9 || word(&region_vtable) != vtable(engine_native_parts::cstrategic_region::VTABLE_RVA)? {
        return None;
    }
    let region_id = half(&region_id);
    let game = usize::try_from(word(&game))
        .ok()
        .filter(|&game| game != 0)?;
    let pool = usize::try_from(word(&pool)).ok()?;
    let histories = Array::from_header(histories, 8)?;
    if word(&wing_vtable) != vtable(engine_native_parts::cair_wing_layout::VTABLE_RVA)? || pool == 0 {
        return None;
    }

    let (mut hour, mut pool_vtable, mut definition, mut history) = ([0; 4], [0; 8], [0; 8], [0; 8]);
    let copied = memory.copy_scattered(
        [
            game.checked_add(engine_native::cgame_state::CURRENT_HOUR)?,
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
    let forms = [
        csub_unit_definition::VTABLE_RVA,
        csub_unit_definition::NULL_VTABLE_RVA,
    ];
    if copied != 6 || engine_sites::which(base, word(&definition_vtable) as usize, &forms).is_none()
    {
        return None;
    }
    let family = family_base(word(&flags), enemy);
    let indices = selected_indices(family, cause).ok()?;
    if word(&receiver_vtable) != vtable(cstrategic_air::VTABLE_RVA)? {
        return None;
    }
    let country = half(&country);
    engine_native::ccountry::Reader::new(memory, base, game)
        .and_then(|countries| countries.resolve(country))
        .ok()?;
    if word(&history_vtable) != vtable(cloop_history::VTABLE_RVA)?
        || usize::try_from(word(&own)).ok()? != history
        || half(&count) != FIELD_COUNT as u32
    {
        return None;
    }
    let window = usize::try_from(word(&window)).ok().filter(|&w| w != 0)?;

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

pub fn selected_indices(family: u32, cause: i32) -> Result<[u32; 2], &'static str> {
    let second = i64::from(family) + i64::from(cause);
    if family as usize >= FIELD_COUNT || !(0..FIELD_COUNT as i64).contains(&second) {
        return Err("air loss cause field is outside native record");
    }
    Ok([family, second as u32])
}

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
    vtable: Rva,
) -> Result<(), &'static str> {
    let expected =
        engine_sites::at(base, vtable).ok_or("air loss writer vtable address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air loss writer object vtable unreadable or mismatched");
    }
    Ok(())
}

fn field<T: engine_native_memory::memory::Raw>(
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

pub fn wing_count(memory: &impl Memory, base: usize, game: usize) -> Result<usize, &'static str> {
    let mut total = 0_usize;
    cstrategic_air::visit(memory, base, game, |root| {
        let pools = array(memory, root.address, cstrategic_air::POOLS)?;
        engine_native_memory::cpdx_array::pointers(
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
        .checked_add(engine_native::cgame_state::DAILY_COUNTRY_ROSTER)
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
