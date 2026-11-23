
use super::{
    COUNTRY_MODIFIERS, EXTRA_SHARED_SLOTS, ID, OWNER, VTABLE_RVA,
    country_modifier_accumulator_with_canonical,
};
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        RecordField,
        RecordLayout,
        RecordSources,
        packed_fields,
    },
    read,
    read_slice,
    walk::{Buffer, Outcome},
};
use crate::{ccountry, cgame_state, cproduction_status};
use engine_native_parts::{modifier_accumulator, modifier_definition};

pub const NAMES: [&[u8]; 8] = [
    b"local_building_slots",
    b"local_building_slots_factor",
    b"global_building_slots",
    b"global_building_slots_factor",
    b"local_factories",
    b"local_factory_energy_consumption",
    b"local_factory_energy_consumption_per_infrastructure",
    b"factory_energy_consumption",
];
const OWNER_STATE: [usize; 2] = [0, 1];
const OWNER_COUNTRY: [usize; 2] = [2, 3];
const CONTROLLER_STATE: [usize; 3] = [4, 5, 6];
const CONTROLLER_COUNTRY: usize = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub extra_shared_slots: i32,
    pub owner_state: Option<[i64; 2]>,
    pub owner_country: Option<[i64; 2]>,
    pub controller_state: Option<[i64; 3]>,
    pub controller_country: Option<i64>,
    pub energy_factor: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Country {
    slots: Option<[i64; 2]>,
    energy: Option<i64>,
    factor: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Keys {
    held: Option<(Array, [Option<u32>; 8])>,
    countries: Vec<Option<Country>>,
}

impl Keys {
    pub fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * size_of::<Option<Country>>()
    }

    fn resolve(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<[Option<u32>; 8], &'static str> {
        let table = modifier_definition::table(memory, base)?;
        if let Some((held, keys)) = self.held
            && held == table
        {
            return Ok(keys);
        }
        self.held = None;
        let (keys, table) = modifier_definition::keys_by_names_at(memory, base, &NAMES)?;
        self.held = Some((table, keys));
        Ok(keys)
    }
}

const FIELDS: [RecordField; 5] = packed_fields([
    (0, 8),
    (ID, 4),
    (OWNER, 8),
    (EXTRA_SHARED_SLOTS, 4),
    (COUNTRY_MODIFIERS, 17),
]);
const RECORD_BYTES: usize = 41;
const LAYOUT: RecordLayout<'static> = match RecordLayout::new(&FIELDS, RECORD_BYTES) {
    Some(layout) => layout,
    None => panic!("valid state building input layout"),
};
const BATCH: usize = 32;

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    held: &mut Keys,
    output: &mut Vec<Row>,
) -> Outcome {
    let keys = match held.resolve(memory, base) {
        Ok(names) => names.map(|key| {
            key.and_then(|key| Some((key, modifier_definition::operands(memory, base, key).ok()?)))
        }),
        Err(reason) => return Outcome::Unavailable { reason },
    };
    walk_keys(
        memory,
        base,
        game_state,
        &keys,
        &mut held.countries,
        Buffer::Growing(output),
    )
    .unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

type Key = Option<(u32, modifier_definition::Operands)>;

fn walk_keys(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    keys: &[Key; 8],
    cache: &mut Vec<Option<Country>>,
    mut output: Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    cache.clear();
    engine_allocation::resize_default(cache, countries.count() as usize)
        .map_err(|_| "state building input allocation failed")?;
    let states = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("state roster unreadable")?;
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let status_vtable = engine_sites::at(base, cproduction_status::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut pointers = [0_u8; BATCH * 8];
    let mut records = [0_u8; BATCH * RECORD_BYTES];
    let mut first = 0;
    while first < states.count {
        let count = (states.count - first).min(BATCH as u32) as usize;
        let at = states
            .element(first, 8)
            .ok_or("state roster address overflow")?;
        if !read_slice(memory, at, &mut pointers[..count * 8]) {
            return Err("state roster unreadable");
        }
        let mut sources = [0_usize; BATCH];
        let mut slots = [0_u32; BATCH];
        let mut live = 0;
        for index in 0..count {
            let pointer =
                u64::from_le_bytes(pointers[index * 8..index * 8 + 8].try_into().unwrap());
            if pointer != 0 {
                sources[live] = pointer as usize;
                slots[live] = first + index as u32;
                live += 1;
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&sources[..live]),
            &LAYOUT,
            &mut records[..live * RECORD_BYTES],
        );
        if copied != live * FIELDS.len() {
            return Err("state building input fields unreadable");
        }
        for index in 0..live {
            let record = &records[index * RECORD_BYTES..(index + 1) * RECORD_BYTES];
            let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable
                || word(8) != slots[index]
            {
                return Err("state building input identity invalid");
            }
            if !output.ensure(len + 1)? {
                unread_rows = unread_rows.saturating_add(1);
                continue;
            }
            let row = state_row(
                memory,
                status_vtable,
                &countries,
                keys,
                cache,
                sources[index],
                record,
            );
            output[len] = row;
            len += 1;
        }
        first += count as u32;
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn state_row<M: Memory>(
    memory: &M,
    status_vtable: u64,
    countries: &ccountry::Reader<'_, M>,
    keys: &[Key; 8],
    cache: &mut [Option<Country>],
    state: usize,
    record: &[u8],
) -> Row {
    let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
    let (owner, controller) = (word(12), word(16));
    let header: [u8; 17] = record[24..41].try_into().unwrap();
    let canonical = |raw: u32| -> Result<u32, &'static str> {
        if raw == 0 {
            return Ok(0);
        }
        countries
            .link(raw)?
            .ok_or("country link unreadable or outside country table")
    };
    let accumulator = |raw: u32| {
        country_modifier_accumulator_with_canonical(memory, state, raw, header, canonical)
    };
    let owner_accumulator = accumulator(owner);
    let controller_accumulator = if controller == owner {
        owner_accumulator
    } else {
        accumulator(controller)
    };
    let owner_state = owner_accumulator
        .ok()
        .and_then(|address| values(memory, address, keys, OWNER_STATE));
    let controller_state = controller_accumulator
        .ok()
        .and_then(|address| values(memory, address, keys, CONTROLLER_STATE));
    let owner_country = country(memory, status_vtable, countries, keys, cache, owner);
    let controller_country = country(memory, status_vtable, countries, keys, cache, controller);
    Row {
        state: word(8),
        owner,
        controller,
        extra_shared_slots: i32::from_le_bytes(record[20..24].try_into().unwrap()),
        owner_state,
        owner_country: owner_country.and_then(|country| country.slots),
        controller_state,
        controller_country: controller_country.and_then(|country| country.energy),
        energy_factor: controller_country.and_then(|country| country.factor),
    }
}

fn values<const N: usize>(
    memory: &impl Memory,
    address: usize,
    keys: &[Key; 8],
    wanted: [usize; N],
) -> Option<[i64; N]> {
    let mut resolved = [(0_u32, modifier_definition::Operands::default(), 0_usize); N];
    for (out, (slot, at)) in resolved.iter_mut().zip(wanted.into_iter().enumerate()) {
        let (key, operands) = keys[at]?;
        *out = (key, operands, slot);
    }
    resolved.sort_unstable_by_key(|entry| entry.0);
    if resolved.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    let sorted = resolved.map(|entry| entry.0);
    let mut found = [engine_native_memory::modifier_cache::Value::default(); N];
    modifier_accumulator::lookup_many_at(memory, address, &sorted, &mut found).ok()?;
    let mut result = [0_i64; N];
    for ((_, operands, slot), value) in resolved.into_iter().zip(found) {
        result[slot] = if value.present {
            operands.effective(value.raw)
        } else {
            0
        };
    }
    Some(result)
}

fn country<M: Memory>(
    memory: &M,
    status_vtable: u64,
    countries: &ccountry::Reader<'_, M>,
    keys: &[Key; 8],
    cache: &mut [Option<Country>],
    raw: u32,
) -> Option<Country> {
    let country = countries.resolve(raw).ok()?;
    let slot = cache.get_mut(country.canonical_slot as usize)?;
    if let Some(held) = *slot {
        return Some(held);
    }
    let accumulator = country
        .pointer
        .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?;
    let slots = values(memory, accumulator, keys, OWNER_COUNTRY);
    let energy = values(memory, accumulator, keys, [CONTROLLER_COUNTRY]).map(|[value]| value);
    let factor = (|| {
        let status = read::<u64>(
            memory,
            country.pointer.checked_add(ccountry::PRODUCTION_STATUS)?,
        )?;
        if status == 0 || read::<u64>(memory, status as usize)? != status_vtable {
            return None;
        }
        read::<i64>(
            memory,
            (status as usize).checked_add(cproduction_status::FACTORY_ENERGY_FACTOR)?,
        )
    })();
    let read = Country {
        slots,
        energy,
        factor,
    };
    *slot = Some(read);
    Some(read)
}

#[cfg(test)]
mod tests;
