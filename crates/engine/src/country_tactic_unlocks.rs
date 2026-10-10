//! Country-wide simulation doctrine unlocks for possible-tactic presentation.
//! The doctrine array uses canonical country slots with 0xA0-byte rows. Its
//! ordered tactic-pointer vector appends an unlock only without a matching
//! unlock token. Serialization writes database IDs in vector order and omits
//! empty lists. Missing authored names select the authenticated null tactic
//! at database entry 0, recorded as ID 0.
//!
//! Pointers authenticate against the loaded tactic database read once into an
//! address-keyed table. Each country costs one 0x30-byte row copy covering its
//! identity and vector header, then a bulk copy for a nonempty list. The
//! callback supplies the authenticated roster; content and the lexical
//! dictionary supply the authored-name token.
//!
//! Every hour reads the full census. Unlock effects, reward application and
//! revocation, save loading and construction/move/destruction write lists.
//! Reward paths reach rows through back-pointers in other objects, so observing
//! only the row owner does not justify skipping an hour.
//! Capture each country once, including explicit empty lists. No viewing
//! country, combat side, locked background or trigger result is read.

use crate::{
    Memory, ccombat_tactic, ccountry, cgame_state, cnull_combat_tactic, cpdx_array::Array, read,
    read_slice, walk::Outcome,
};

/// Game-state doctrine-system pointer.
pub const DOCTRINE_SYSTEM: usize = 0x400;
/// Doctrine-system country-status array, serialized as the country-status table.
pub const DOCTRINE_ROWS: usize = 8;
pub const DOCTRINE_STRIDE: usize = 0xa0;
/// Country-status raw country reference initialized on emplacement and serialized.
pub const ROW_TAG: usize = 8;
/// CCountryDoctrineStatus tactic-pointer vector.
pub const TACTICS: usize = 0x28;
/// One copy spans the row tag through the vector header (+8..+0x38).
const ROW_SPAN: usize = TACTICS + 0x10 - ROW_TAG;
const POINTER: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw CCountryTag identity, not the canonical array index.
    pub country: u32,
    pub unlock_start: usize,
    pub unlock_count: usize,
}

/// One loaded tactic: its object address and its +98 database ID (0 for the
/// null tactic at index 0), a slot of an open-addressed table keyed by the
/// address (pointer 0 is an empty slot). The address is never recorded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistryEntry {
    pub pointer: usize,
    pub id: u32,
}

/// The headers a capture was bounded by; they must be unchanged when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Roots {
    pub database: usize,
    pub entries: Array,
    pub system: usize,
    pub doctrine: Array,
}

/// Caller-owned buffers. Every length is a capacity; demand beyond it is
/// reported in [`Capture`] and never becomes a partial table.
pub struct Buffers<'a> {
    pub rows: &'a mut [Row],
    pub unlocks: &'a mut [u32],
    /// The database table.
    pub registry: &'a mut [RegistryEntry],
    /// Byte staging for the database entries or one country's pointer list.
    pub list: &'a mut [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub outcome: Outcome,
    pub required_rows: usize,
    pub required_unlocks: usize,
    /// Database table slots (`registry_slots` of the entries including the
    /// null entry at index 0).
    pub required_registry: usize,
    /// Bytes of the database entries or the longest country list.
    pub required_list: usize,
}

fn field<T: crate::memory::Raw>(
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

/// Slots of the database table for `count` entries: at least twice as many,
/// so every probe sequence ends at an empty slot within a step or two.
/// A native pointer list's count becomes growth demand only once its first
/// and last entries are proven readable, so a torn header cannot ask the spool
/// for more staging than the game has mapped at that address. A list that
/// fits the current staging needs no probe: reading it whole proves it.
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

/// Database ID of a loaded tactic. Null pointer names no tactic; the null
/// tactic is an authenticated nonzero singleton, separate from an empty slot.
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

/// The database and doctrine-system headers every capture is bounded by.
fn roots(memory: &impl Memory, base: usize, game: usize) -> Result<Roots, &'static str> {
    let database = pointer(memory, base, ccombat_tactic::DATABASE_RVA)?;
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

/// Read the database into an open-addressed table in `registry`,
/// authenticating entry 0 as the null tactic (ID 0) and every other entry as
/// the `CCombatTactic` carrying its own index. `false` when the buffers are
/// too small; the demand is [`registry_slots`] entries and `entries.count * 8`
/// staging bytes.
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
    let vtable = |rva: usize| {
        base.checked_add(rva)
            .map(|address| address as u64)
            .ok_or("country tactic unlock module address overflow")
    };
    let (null, tactic) = (
        vtable(cnull_combat_tactic::VTABLE_RVA)?,
        vtable(ccombat_tactic::VTABLE_RVA)?,
    );
    for (index, object) in decode(&list[..bytes]).enumerate() {
        // Database ordinal is the assigned ID; mismatch refuses identity.
        let expected = if index == 0 { null } else { tactic };
        if object == 0
            || field::<u64>(memory, object, 0)? != expected
            || field::<i32>(memory, object, ccombat_tactic::ID)? != index as i32
        {
            return Err("tactic database entry identity invalid");
        }
        // Each object carries its own index, so no object appears twice.
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

/// Full census sized from the live country, database and list counts.
/// Pressure retains the exact demand of every buffer but publishes no
/// partial table. Lists that fit the staging buffer are still authenticated
/// under other pressure, so a fault is never hidden as pressure.
pub fn capture(memory: &impl Memory, base: usize, game: usize, buffers: Buffers<'_>) -> Capture {
    capture_buffer(
        memory,
        base,
        game,
        crate::walk::Buffer::Fixed(buffers.rows),
        crate::walk::Buffer::Fixed(buffers.unlocks),
        crate::walk::Buffer::Fixed(buffers.registry),
        crate::walk::Buffer::Fixed(buffers.list),
    )
}

/// Grow on the worker from the original database, country and unlock headers.
#[allow(clippy::too_many_arguments)]
pub fn capture_growing(
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(unlocks),
        crate::walk::Buffer::Growing(registry),
        crate::walk::Buffer::Growing(list),
    )
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut unlocks: crate::walk::Buffer<'_, u32>,
    mut table: crate::walk::Buffer<'_, RegistryEntry>,
    mut list: crate::walk::Buffer<'_, u8>,
) -> Capture {
    let mut capture = Capture {
        outcome: Outcome::Rows { len: 0 },
        required_rows: 0,
        required_unlocks: 0,
        required_registry: 0,
        required_list: 0,
    };
    // A list that was read but could not be stored makes the capture pressure.
    let mut unstored = false;
    let result = (|| {
        let observed = roots(memory, base, game)?;
        prove_ends(memory, observed.entries)?;
        capture.required_registry = registry_slots(observed.entries.count);
        capture.required_list = list_bytes(observed.entries.count)?;
        // The live database and acquired list extents authorize complete storage
        // before interpreting captured lists.
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
                // The row's own CCountryTag must name this country.
                let tag = u32::from_le_bytes(span[..4].try_into().expect("four bytes"));
                if tag == 0 || countries.link(tag)? != Some(country.canonical_slot) {
                    return Err("country tactic unlock row names another country");
                }
                let header = span[TACTICS - ROW_TAG..].try_into().expect("sixteen bytes");
                let entries = Array::from_header(header, POINTER)
                    .ok_or("country tactic unlock array invalid")?;
                let bytes = list_bytes(entries.count)?;
                list.ensure(bytes)?;
                let end = start
                    .checked_add(entries.count as usize)
                    .ok_or("country tactic unlock count overflows")?;
                unlocks.ensure(end)?;
                let fits = database.is_some() && bytes <= list.len();
                if !fits {
                    prove_ends(memory, entries)?;
                }
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
