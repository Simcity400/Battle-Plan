
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native_parts::{ccombat_tactic, cnull_combat_tactic};
use engine_native::{ccountry, cgame_state};
use engine_sites::Rva;

pub const DOCTRINE_SYSTEM: usize = 0x400;
pub const DOCTRINE_ROWS: usize = 8;
pub const DOCTRINE_STRIDE: usize = 0xa0;
pub const ROW_TAG: usize = 8;
pub const TACTICS: usize = 0x28;
const ROW_SPAN: usize = TACTICS + 0x10 - ROW_TAG;
const POINTER: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub unlock_start: usize,
    pub unlock_count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistryEntry {
    pub pointer: usize,
    pub id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Roots {
    pub database: usize,
    pub entries: Array,
    pub system: usize,
    pub doctrine: Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub outcome: Outcome,
    pub status_data: usize,
    pub status_count: u32,
    pub required_rows: usize,
    pub required_unlocks: usize,
    pub required_registry: usize,
    pub required_list: usize,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("country tactic unlock input unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "country tactic unlock pointer overflow")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("country tactic unlock array invalid")
}

fn list_bytes(count: u32) -> Result<usize, &'static str> {
    (count as usize)
        .checked_mul(POINTER)
        .ok_or("country tactic unlock list size overflows")
}

fn decode(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    bytes
        .chunks_exact(POINTER)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("eight bytes")) as usize)
}

fn prove_ends(memory: &impl Memory, entries: Array) -> Result<(), &'static str> {
    if entries.count == 0 {
        return Ok(());
    }
    let last = entries
        .element(entries.count - 1, POINTER)
        .ok_or("country tactic unlock list end unreadable")?;
    let (mut first, mut end) = ([0_u8; POINTER], [0_u8; POINTER]);
    if memory.copy_scattered_prefetched([entries.data, last], &mut [&mut first, &mut end]) != 2 {
        return Err("country tactic unlock list end unreadable");
    }
    Ok(())
}

fn registry_slots(count: u32) -> usize {
    (count as usize)
        .saturating_mul(2)
        .max(1)
        .next_power_of_two()
}

fn home(pointer: usize, mask: usize) -> usize {
    ((pointer as u64 >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize & mask
}

fn lookup(table: &[RegistryEntry], pointer: usize) -> Option<u32> {
    if pointer == 0 {
        return None;
    }
    let mask = table.len().checked_sub(1)?;
    let mut index = home(pointer, mask);
    for _ in 0..table.len() {
        let entry = table[index];
        if entry.pointer == pointer {
            return Some(entry.id);
        }
        if entry.pointer == 0 {
            return None;
        }
        index = (index + 1) & mask;
    }
    None
}

fn roots(memory: &impl Memory, base: usize, game: usize) -> Result<Roots, &'static str> {
    let global = engine_sites::at(base, ccombat_tactic::DATABASE_RVA)
        .ok_or("country tactic unlock input unreadable")?;
    let database = pointer(memory, global, 0)?;
    if database == 0 {
        return Err("tactic database is not initialized");
    }
    let entries = array(memory, database, ccombat_tactic::DATABASE_ENTRIES, POINTER)?;
    if entries.count == 0 {
        return Err("tactic database missing null entry");
    }
    let system = pointer(memory, game, DOCTRINE_SYSTEM)?;
    if system == 0 {
        return Err("country tactic unlock doctrine system missing");
    }
    let doctrine = array(memory, system, DOCTRINE_ROWS, DOCTRINE_STRIDE)?;
    Ok(Roots {
        database,
        entries,
        system,
        doctrine,
    })
}

fn registry(
    memory: &impl Memory,
    base: usize,
    entries: Array,
    registry: &mut [RegistryEntry],
    list: &mut [u8],
) -> Result<bool, &'static str> {
    let slots = registry_slots(entries.count);
    let bytes = list_bytes(entries.count)?;
    if slots > registry.len() || bytes > list.len() {
        return Ok(false);
    }
    let table = &mut registry[..slots];
    table.fill(RegistryEntry::default());
    if !read_slice(memory, entries.data, &mut list[..bytes]) {
        return Err("tactic database entries unreadable");
    }
    let vtable = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("country tactic unlock module address overflow")
    };
    let (null, tactic) = (
        vtable(cnull_combat_tactic::VTABLE_RVA)?,
        vtable(ccombat_tactic::VTABLE_RVA)?,
    );
    for (index, object) in decode(&list[..bytes]).enumerate() {
        let expected = if index == 0 { null } else { tactic };
        if object == 0
            || field::<u64>(memory, object, 0)? != expected
            || field::<i32>(memory, object, ccombat_tactic::ID)? != index as i32
        {
            return Err("tactic database entry identity invalid");
        }
        let mask = slots - 1;
        let mut slot = home(object, mask);
        while table[slot].pointer != 0 {
            slot = (slot + 1) & mask;
        }
        table[slot] = RegistryEntry {
            pointer: object,
            id: index as u32,
        };
    }
    Ok(true)
}

pub fn count(memory: &impl Memory, base: usize, game: usize) -> Capture {
    capture_buffer(
        memory,
        base,
        game,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
    )
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut Vec<Row>,
    unlocks: &mut Vec<u32>,
    registry: &mut Vec<RegistryEntry>,
    list: &mut Vec<u8>,
) -> Capture {
    capture_buffer(
        memory,
        base,
        game,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(unlocks),
        engine_native_memory::walk::Buffer::Growing(registry),
        engine_native_memory::walk::Buffer::Growing(list),
    )
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut unlocks: engine_native_memory::walk::Buffer<'_, u32>,
    mut table: engine_native_memory::walk::Buffer<'_, RegistryEntry>,
    mut list: engine_native_memory::walk::Buffer<'_, u8>,
) -> Capture {
    let mut capture = Capture {
        status_data: 0,
        status_count: 0,
        outcome: Outcome::Rows { len: 0 },
        required_rows: 0,
        required_unlocks: 0,
        required_registry: 0,
        required_list: 0,
    };
    let mut unstored = false;
    let result = (|| {
        let observed = roots(memory, base, game)?;
        capture.status_data = observed.doctrine.data;
        capture.status_count = observed.doctrine.count;
        prove_ends(memory, observed.entries)?;
        capture.required_registry = registry_slots(observed.entries.count);
        capture.required_list = list_bytes(observed.entries.count)?;
        table.ensure(capture.required_registry)?;
        list.ensure(capture.required_list)?;
        let database = registry(memory, base, observed.entries, &mut table, &mut list)?
            .then(|| &table[..capture.required_registry]);
        let countries = ccountry::Reader::new(memory, base, game)?;
        output.ensure(countries.count().saturating_sub(1) as usize)?;
        let country_roots = countries.arrays();
        let mut span = [0u8; ROW_SPAN];
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let start = capture.required_unlocks;
            let count = {
                let native = observed
                    .doctrine
                    .element(country.canonical_slot, DOCTRINE_STRIDE)
                    .ok_or("country tactic unlock country outside doctrine table")?
                    .checked_add(ROW_TAG)
                    .ok_or("country tactic unlock row address overflow")?;
                if !read_slice(memory, native, &mut span) {
                    return Err("country tactic unlock row unreadable");
                }
                let tag = u32::from_le_bytes(span[..4].try_into().expect("four bytes"));
                if tag == 0 || countries.link(tag)? != Some(country.canonical_slot) {
                    return Err("country tactic unlock row names another country");
                }
                let header = span[TACTICS - ROW_TAG..].try_into().expect("sixteen bytes");
                let entries = Array::from_header(header, POINTER)
                    .ok_or("country tactic unlock array invalid")?;
                let bytes = list_bytes(entries.count)?;
                let end = start
                    .checked_add(entries.count as usize)
                    .ok_or("country tactic unlock count overflows")?;
                let fits = database.is_some() && bytes <= list.len();
                if !fits || end > unlocks.len() {
                    let full = database.is_some() && (fits || list.is_growing());
                    prove_ends(memory, entries).map_err(|reason| {
                        if full {
                            "country tactic unlock list unreadable"
                        } else {
                            reason
                        }
                    })?;
                }
                list.ensure(bytes)?;
                unlocks.ensure(end)?;
                capture.required_list = capture.required_list.max(bytes);
                if let (Some(database), Some(staged)) = (database, list.get_mut(..bytes)) {
                    if !read_slice(memory, entries.data, staged) {
                        return Err("country tactic unlock list unreadable");
                    }
                    let end = start.saturating_add(entries.count as usize);
                    let destination = unlocks.get_mut(start..end);
                    let mut ids = decode(staged).map(|tactic| {
                        lookup(database, tactic)
                            .ok_or("country tactic unlock names no loaded tactic")
                    });
                    match destination {
                        Some(destination) => {
                            for (slot, id) in destination.iter_mut().zip(&mut ids) {
                                *slot = id?;
                            }
                        }
                        None => {
                            for id in ids {
                                id?;
                            }
                        }
                    }
                } else {
                    unstored = true;
                }
                entries.count as usize
            };
            capture.required_unlocks = start
                .checked_add(count)
                .ok_or("country tactic unlock count overflows")?;
            if let Some(destination) = output.get_mut(capture.required_rows) {
                *destination = Row {
                    country: country.id,
                    unlock_start: start,
                    unlock_count: count,
                };
            }
            capture.required_rows += 1;
        }
        if roots(memory, base, game)? != observed
            || array(memory, game, cgame_state::COUNTRIES, 8)? != country_roots[0]
            || array(memory, game, cgame_state::COUNTRY_LINKS, 4)? != country_roots[1]
        {
            return Err("country tactic unlock census changed during capture");
        }
        Ok(())
    })();
    capture.outcome = match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(())
            if unstored
                || capture.required_rows > output.len()
                || capture.required_unlocks > unlocks.len() =>
        {
            Outcome::Truncated {
                len: 0,
                unread_rows: capture.required_rows as u32,
            }
        }
        Ok(()) => Outcome::Rows {
            len: capture.required_rows,
        },
    };
    capture
}

#[cfg(test)]
mod tests;
