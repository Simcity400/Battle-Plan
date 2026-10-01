//! Country-wide simulation doctrine unlocks for possible-tactic presentation.
//!
//! 140D6A7B0 indexes game+400's doctrine array at +8 by canonical country
//! slot (stride A0), each row an `NDoctrines::CCountryDoctrineStatus`. Its
//! +28 vector holds combat-tactic pointers: 1413BA2E0
//! (CUnlockCombatTacticEffect) appends a tactic unless one with the same +94
//! unlock token is present, and 1413B8CF0 tests membership by +94. The row
//! writer 1413BA360 saves the list under 0x3357 (`enable_tactic`) as each
//! pointer's +98 database ID, in vector order, and omits an empty list.
//!
//! An `unlock_tactic` name absent from the database appends the
//! `CNullCombatTactic` singleton: FindCombatTacticByNameOrNullTactic
//! (1406B11F0) falls back to 1406B3BD0, which 1406B3690 also inserts as
//! database entry 0 with +98 ID 0. The save writes it as `enable_tactic={ 0 }`,
//! so entry 0 is authenticated as the null tactic and recorded as ID 0.
//!
//! The recording keeps that saved identity. Every pointer is authenticated
//! against the loaded tactic database (1406B0FC0) rather than dereferenced:
//! the database is read once into an address-keyed table. A country costs one
//! 0x30-byte copy of its row (+8 CCountryTag through the +28 vector header,
//! which authenticates the row's own country) and one bulk copy of a nonempty
//! list; the country itself comes from the callback's authenticated roster.
//! The +94 token is a property of the tactic's authored name, which the viewer
//! derives from content and the lexical dictionary.
//!
//! Every hour reads the whole census (0.31 ms at 5,000 countries x 32 unlocks,
//! synthetic, release). No list is reused on a hook's word, because the
//! vector's writer set is not proven. Known writers of +0x28/+0x30/+0x34:
//! - 1413BA2E0, the unlock effect's appender (only caller 14051D2E0);
//! - 1413B8130, the doctrine reward applier, an inlined PushBack through the
//!   row's +0x38 allocator, from CTrackStatus::AddMastery (14146A7F0,
//!   140FB3500) and SetGrandDoctrine (140FB1770, 14146A590);
//! - 1413B9CF0, the reward revoker, which erases entries (from 140FB1770,
//!   14146A590, 14146A2F0, 14146A340, 140FB3A30);
//! - 1413B4200, the save-load append (from vf4 1413B9C60);
//! - the row's constructors, mover and destructor (140D66F10, 140D66CC0,
//!   140D67C70, 140D68EA0).
//!
//! The reward paths reach the row through back-pointers other objects store
//! (folder +0x30 and track +0x50, set by 140FB32E0), which a call-graph walk
//! from the row's owners does not find, so an instruction-level proof over
//! every holder of those pointers would be needed before any hour could be
//! skipped.
//!
//! Capture every country once, including explicit empty lists. No local
//! viewing country, combat side, locked background or trigger result is read.

use crate::{
    Memory, ccombat_tactic, ccountry, cgame_state, cnull_combat_tactic, cpdx_array::Array, read,
    read_slice, walk::Outcome,
};

/// CGameState doctrine-system pointer (140D6A7B0's first argument).
pub const DOCTRINE_SYSTEM: usize = 0x400;
/// CDoctrineSystem country-status array; vf2 (140D6C880) saves it as 0x2D49.
pub const DOCTRINE_ROWS: usize = 8;
pub const DOCTRINE_STRIDE: usize = 0xa0;
/// CCountryDoctrineStatus CCountryTag, set by the emplacing 140D66F10 /
/// 140D66CC0 and saved by 1413BA360 under 0x289A.
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

/// The database ID of a loaded tactic object. A null pointer names none: the
/// native null tactic is an authenticated nonzero singleton (1406B3BD0), and
/// a zero pointer would otherwise match an empty slot.
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
        // The database's own ordinal is the ID 1406B3690 assigned; a
        // mismatch is not the database the save and content describe.
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
    let Buffers {
        rows: output,
        unlocks,
        registry: table,
        list,
    } = buffers;
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
        let database = registry(memory, base, observed.entries, table, list)?
            .then(|| &table[..capture.required_registry]);
        let countries = ccountry::Reader::new(memory, base, game)?;
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
mod tests {
    use super::*;
    use crate::{country_memory::Cache, memory::SliceMemory};
    use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

    const GAME: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const COUNTRY_A: usize = 0x3000;
    const COUNTRY_B: usize = 0x4000;
    const SYSTEM: usize = 0x5000;
    const SYSTEM_MOVED: usize = 0x5800;
    const DOCTRINE: usize = 0x6000;
    const LIST: usize = 0x7000;
    const TACTIC_A: usize = 0x8000;
    const TACTIC_B: usize = 0x9000;
    const DATABASE: usize = 0xa000;
    const DATABASE_MOVED: usize = 0xa800;
    const ENTRIES: usize = 0xb000;
    const FOREIGN: usize = 0xc000;
    const NULL_TACTIC: usize = 0xd000;

    #[derive(Default)]
    struct Fixture {
        bytes: BTreeMap<usize, u8>,
        reads: RefCell<Vec<(usize, usize)>>,
        fail: Option<usize>,
        change: Option<(usize, usize, u64)>,
    }

    impl Fixture {
        fn put<const N: usize>(&mut self, at: usize, bytes: [u8; N]) {
            self.bytes
                .extend(bytes.into_iter().enumerate().map(|(i, b)| (at + i, b)));
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.put(at, (data as u64).to_le_bytes());
            self.put(at + 8, count.to_le_bytes());
            self.put(at + 12, count.to_le_bytes());
        }
        fn row(slot: usize) -> usize {
            DOCTRINE + slot * DOCTRINE_STRIDE
        }
        fn new() -> Self {
            let mut f = Self::default();
            f.array(GAME + cgame_state::COUNTRIES, COUNTRIES, 4);
            f.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
            for (index, pointer) in [0, COUNTRY_A, COUNTRY_A, COUNTRY_B].into_iter().enumerate() {
                f.put(COUNTRIES + index * 8, (pointer as u64).to_le_bytes());
            }
            for (index, slot) in [0u32, 1, 1, 3].into_iter().enumerate() {
                f.put(LINKS + index * 4, slot.to_le_bytes());
            }
            for (pointer, id) in [(COUNTRY_A, 2u32), (COUNTRY_B, 3)] {
                f.put(pointer, (ccountry::VTABLE_RVA as u64).to_le_bytes());
                f.put(pointer + ccountry::ID, id.to_le_bytes());
            }
            f.put(GAME + DOCTRINE_SYSTEM, (SYSTEM as u64).to_le_bytes());
            f.array(SYSTEM + DOCTRINE_ROWS, DOCTRINE, 4);
            f.array(SYSTEM_MOVED + DOCTRINE_ROWS, DOCTRINE, 4);
            // Every doctrine row spans its tag through its vector header.
            for slot in 0..4 {
                f.put(Self::row(slot) + ROW_TAG, [0; ROW_SPAN]);
            }
            // Row 1 carries country A's raw id 2 (canonical slot 1); row 3 B's.
            f.put(Self::row(1) + ROW_TAG, 2u32.to_le_bytes());
            f.put(Self::row(3) + ROW_TAG, 3u32.to_le_bytes());
            f.array(Self::row(1) + TACTICS, LIST, 3);
            for (index, tactic) in [TACTIC_B, TACTIC_A, TACTIC_B].into_iter().enumerate() {
                f.put(LIST + index * 8, (tactic as u64).to_le_bytes());
            }
            // Database: the null tactic, then IDs 1 and 2 in ID order.
            f.put(
                ccombat_tactic::DATABASE_RVA,
                (DATABASE as u64).to_le_bytes(),
            );
            f.array(DATABASE + ccombat_tactic::DATABASE_ENTRIES, ENTRIES, 3);
            f.array(
                DATABASE_MOVED + ccombat_tactic::DATABASE_ENTRIES,
                ENTRIES,
                3,
            );
            for (index, tactic) in [NULL_TACTIC, TACTIC_A, TACTIC_B].into_iter().enumerate() {
                f.put(ENTRIES + index * 8, (tactic as u64).to_le_bytes());
            }
            f.put(
                NULL_TACTIC,
                (cnull_combat_tactic::VTABLE_RVA as u64).to_le_bytes(),
            );
            f.put(NULL_TACTIC + ccombat_tactic::ID, 0i32.to_le_bytes());
            for (pointer, id) in [(TACTIC_A, 1i32), (TACTIC_B, 2), (FOREIGN, 3)] {
                f.put(pointer, (ccombat_tactic::VTABLE_RVA as u64).to_le_bytes());
                f.put(pointer + ccombat_tactic::ID, id.to_le_bytes());
            }
            f
        }
    }

    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut reads = self.reads.borrow_mut();
            let index = reads.len();
            let occurrence = reads.iter().filter(|(at, _)| *at == address).count();
            reads.push((address, output.len()));
            if self.fail == Some(index) {
                if let Some(first) = output.first_mut() {
                    *first = 0xa5;
                }
                return false;
            }
            if let Some((at, count, replacement)) = self.change
                && address == at
                && occurrence == count
                && output.len() == 8
            {
                output.copy_from_slice(&replacement.to_le_bytes());
                return true;
            }
            for (i, byte) in output.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(address + i)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    struct Owned {
        rows: Vec<Row>,
        unlocks: Vec<u32>,
        registry: Vec<RegistryEntry>,
        list: Vec<u8>,
    }

    impl Owned {
        fn new(rows: usize, unlocks: usize, registry: usize, list: usize) -> Self {
            Self {
                rows: vec![Row::default(); rows],
                unlocks: vec![0; unlocks],
                registry: vec![RegistryEntry::default(); registry],
                list: vec![0; list],
            }
        }
        fn fits() -> Self {
            Self::new(2, 3, 8, 24)
        }
        fn capture(&mut self, memory: &impl Memory, base: usize, game: usize) -> Capture {
            capture(
                memory,
                base,
                game,
                Buffers {
                    rows: &mut self.rows,
                    unlocks: &mut self.unlocks,
                    registry: &mut self.registry,
                    list: &mut self.list,
                },
            )
        }
        fn fixture(&mut self, memory: &Fixture) -> Capture {
            self.capture(memory, 0, GAME)
        }
    }

    #[test]
    fn census_records_saved_database_ids_in_native_order_with_one_copy_per_list() {
        let memory = Fixture::new();
        let mut owned = Owned::fits();
        let result = owned.fixture(&memory);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            owned.rows,
            [
                Row {
                    country: 2,
                    unlock_start: 0,
                    unlock_count: 3
                },
                Row {
                    country: 3,
                    unlock_start: 3,
                    unlock_count: 0
                },
            ]
        );
        // Repeated native entries remain independently observed occurrences.
        assert_eq!(owned.unlocks, [2, 1, 2]);
        let reads = memory.reads.borrow();
        assert_eq!(
            reads
                .iter()
                .filter(|(at, _)| *at == LIST)
                .collect::<Vec<_>>(),
            [&(LIST, 24)]
        );
        // One copy spans each row's tag and vector header.
        assert_eq!(
            reads
                .iter()
                .filter(|(at, _)| *at == Fixture::row(1) + ROW_TAG)
                .collect::<Vec<_>>(),
            [&(Fixture::row(1) + ROW_TAG, ROW_SPAN)]
        );
        // Country lists resolve through the database, never by dereference.
        assert!(
            !reads
                .iter()
                .any(|(at, _)| (LIST + 8..LIST + 24).contains(at))
        );
        assert_eq!(reads.iter().filter(|(at, _)| *at == TACTIC_B).count(), 1);
    }

    /// An unknown `unlock_tactic` name appends the null tactic, which the
    /// save writes as ID 0; it is a recorded unlock, not a fault.
    #[test]
    fn the_null_tactic_is_database_entry_zero_and_records_id_zero() {
        let mut memory = Fixture::new();
        memory.put(LIST + 8, (NULL_TACTIC as u64).to_le_bytes());
        let mut owned = Owned::fits();
        assert_eq!(owned.fixture(&memory).outcome, Outcome::Rows { len: 2 });
        assert_eq!(owned.unlocks, [2, 0, 2]);
        let invalid = Outcome::Unavailable {
            reason: "tactic database entry identity invalid",
        };
        // Entry 0 must be the null tactic with ID 0, nothing else.
        let mut memory = Fixture::new();
        memory.put(
            NULL_TACTIC,
            (ccombat_tactic::VTABLE_RVA as u64).to_le_bytes(),
        );
        assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
        let mut memory = Fixture::new();
        memory.put(NULL_TACTIC + ccombat_tactic::ID, 5i32.to_le_bytes());
        assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
    }

    #[test]
    fn pressure_preserves_exact_demand_and_cannot_hide_an_unknown_tactic() {
        let memory = Fixture::new();
        let result = Owned::new(0, 0, 0, 0).fixture(&memory);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(
            (
                result.required_rows,
                result.required_unlocks,
                result.required_registry,
                result.required_list
            ),
            (2, 3, 8, 24)
        );
        // Only the unlock arena is short: every list is still authenticated.
        let mut memory = Fixture::new();
        memory.put(LIST + 8, (FOREIGN as u64).to_le_bytes());
        assert_eq!(
            Owned::new(2, 0, 8, 24).fixture(&memory).outcome,
            Outcome::Unavailable {
                reason: "country tactic unlock names no loaded tactic"
            }
        );
        // A null pointer is corrupt input, never the null tactic's ID 0 (an
        // authenticated nonzero singleton, 1406B3BD0), whether or not the
        // unlock arena fits.
        let mut memory = Fixture::new();
        memory.put(LIST + 8, 0u64.to_le_bytes());
        for owned in [Owned::fits(), Owned::new(2, 0, 8, 24)] {
            let mut owned = owned;
            assert_eq!(
                owned.fixture(&memory).outcome,
                Outcome::Unavailable {
                    reason: "country tactic unlock names no loaded tactic"
                }
            );
        }
    }

    /// A list that does not fit the staging counts as demand only once its
    /// first and last entries read: a torn header that claims a huge list at
    /// an unmapped address fails the table instead of asking the spool for
    /// gigabytes, while a readable one still reports its exact demand.
    #[test]
    fn an_unfit_list_is_demand_only_once_its_ends_read() {
        let mut memory = Fixture::new();
        memory.array(Fixture::row(1) + TACTICS, 0x7fff_0000, i32::MAX as u32);
        let result = Owned::new(0, 0, 0, 0).fixture(&memory);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "country tactic unlock list end unreadable"
            }
        );
        assert!(result.required_list < 1 << 20);
        // The same readable list reports its exact demand, as before.
        let memory = Fixture::new();
        assert_eq!(Owned::new(0, 0, 0, 0).fixture(&memory).required_list, 24);
    }

    #[test]
    fn every_short_read_is_unavailable_and_stops_at_the_first_fault() {
        let memory = Fixture::new();
        let _ = Owned::fits().fixture(&memory);
        let count = memory.reads.borrow().len();
        for fault in 0..count {
            let mut memory = Fixture::new();
            memory.fail = Some(fault);
            let result = Owned::fits().fixture(&memory);
            assert!(
                matches!(result.outcome, Outcome::Unavailable { .. }),
                "fault {fault}"
            );
            assert_eq!(memory.reads.borrow().len(), fault + 1, "fault {fault}");
        }
    }

    #[test]
    fn malformed_database_rows_and_changed_roots_never_become_empty_lists() {
        let invalid = Outcome::Unavailable {
            reason: "tactic database entry identity invalid",
        };
        let mut memory = Fixture::new();
        memory.put(TACTIC_A, 0u64.to_le_bytes());
        assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
        let mut memory = Fixture::new();
        memory.put(TACTIC_B + ccombat_tactic::ID, 7i32.to_le_bytes());
        assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
        // The same object at a second index cannot carry both IDs.
        let mut memory = Fixture::new();
        memory.put(ENTRIES + 16, (TACTIC_A as u64).to_le_bytes());
        assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
        // A row whose own tag resolves to another country is not this one's.
        let mut memory = Fixture::new();
        memory.put(Fixture::row(1) + ROW_TAG, 3u32.to_le_bytes());
        assert_eq!(
            Owned::fits().fixture(&memory).outcome,
            Outcome::Unavailable {
                reason: "country tactic unlock row names another country"
            }
        );
        let changed = Outcome::Unavailable {
            reason: "country tactic unlock census changed during capture",
        };
        for (address, moved) in [
            (GAME + DOCTRINE_SYSTEM, SYSTEM_MOVED),
            (ccombat_tactic::DATABASE_RVA, DATABASE_MOVED),
        ] {
            let mut memory = Fixture::new();
            memory.change = Some((address, 1, moved as u64));
            assert_eq!(Owned::fits().fixture(&memory).outcome, changed);
        }
    }

    /// Above the database RVA, so a module base below the world maps the
    /// singleton into it.
    const WORLD: usize = 0x1000_0000;

    /// A flat address space of `countries` live countries, each unlocking
    /// `unlocks` tactics of a `tactics`-entry database (plus the null entry).
    /// Returns the bytes, the module base and the game state.
    fn world(countries: usize, tactics: usize, unlocks: usize) -> (Vec<u8>, usize, usize) {
        let slots = countries + 1;
        let mut end = WORLD;
        let mut region = |size: usize| {
            let start = end;
            end += size.next_multiple_of(0x10);
            start
        };
        let game = region(0x1000);
        let table = region(slots * 8);
        let links = region(slots * 4);
        let objects = region(slots * 0x10);
        let system = region(0x20);
        let doctrine = region(slots * DOCTRINE_STRIDE);
        let database = region(0x20);
        let entries = region((tactics + 1) * 8);
        let objects_t = region((tactics + 1) * 0x100);
        let lists = region(countries * unlocks * 8);
        let base = region(8) - ccombat_tactic::DATABASE_RVA;
        let mut bytes = vec![0u8; end - WORLD];
        let mut put = |address: usize, value: &[u8]| {
            bytes[address - WORLD..address - WORLD + value.len()].copy_from_slice(value);
        };
        let header = |data: usize, count: usize| {
            let mut out = [0u8; 16];
            out[..8].copy_from_slice(&(data as u64).to_le_bytes());
            out[8..12].copy_from_slice(&(count as u32).to_le_bytes());
            out[12..].copy_from_slice(&(count as u32).to_le_bytes());
            out
        };
        put(game + cgame_state::COUNTRIES, &header(table, slots));
        put(game + cgame_state::COUNTRY_LINKS, &header(links, slots));
        put(game + DOCTRINE_SYSTEM, &(system as u64).to_le_bytes());
        put(system + DOCTRINE_ROWS, &header(doctrine, slots));
        for slot in 1..slots {
            let object = objects + slot * 0x10;
            put(table + slot * 8, &(object as u64).to_le_bytes());
            put(links + slot * 4, &(slot as u32).to_le_bytes());
            put(
                object,
                &((base + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(object + ccountry::ID, &(slot as u32).to_le_bytes());
            let row = doctrine + slot * DOCTRINE_STRIDE;
            put(row + ROW_TAG, &(slot as u32).to_le_bytes());
            let list = lists + (slot - 1) * unlocks * 8;
            put(row + TACTICS, &header(list, unlocks));
            for ordinal in 0..unlocks {
                let index = 1 + (slot + ordinal) % tactics;
                let tactic = objects_t + index * 0x100;
                put(list + ordinal * 8, &(tactic as u64).to_le_bytes());
            }
        }
        put(
            base + ccombat_tactic::DATABASE_RVA,
            &(database as u64).to_le_bytes(),
        );
        put(
            database + ccombat_tactic::DATABASE_ENTRIES,
            &header(entries, tactics + 1),
        );
        for index in 0..=tactics {
            let tactic = objects_t + index * 0x100;
            let vtable = match index {
                0 => cnull_combat_tactic::VTABLE_RVA,
                _ => ccombat_tactic::VTABLE_RVA,
            };
            put(entries + index * 8, &(tactic as u64).to_le_bytes());
            put(tactic, &((base + vtable) as u64).to_le_bytes());
            put(tactic + ccombat_tactic::ID, &(index as i32).to_le_bytes());
        }
        (bytes, base, game)
    }

    struct Counted<'a, M> {
        memory: M,
        copies: std::cell::Cell<usize>,
        bytes: std::cell::Cell<usize>,
        _world: std::marker::PhantomData<&'a ()>,
    }

    impl<M: Memory> Memory for Counted<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.bytes.set(self.bytes.get() + output.len());
            self.memory.copy(address, output)
        }
        fn authenticated_countries(
            &self,
            base: usize,
            game_state: usize,
        ) -> Option<ccountry::AuthenticatedRoster<'_>> {
            self.memory.authenticated_countries(base, game_state)
        }
    }

    /// Five thousand countries with 32 unlocks each from a 96-tactic
    /// database, through the callback's authenticated country roster as the
    /// collector's job reads them. Prints copies, bytes and the best of five
    /// wall times; run with `--release -- --nocapture` for representative
    /// timing.
    #[test]
    fn five_thousand_countries_cost_two_copies_each() {
        const COUNTRIES: usize = 5_000;
        const TACTICS: usize = 96;
        const UNLOCKS: usize = 32;
        let (bytes, base, game) = world(COUNTRIES, TACTICS, UNLOCKS);
        let source = SliceMemory {
            base: WORLD,
            bytes: &bytes,
        };
        let mut storage = Arc::new(Cache::with_capacity(Cache::cells_for_countries(
            COUNTRIES + 1,
            8192,
        )));
        let snapshot = Cache::prepare_shared(&mut storage, &source, base, game).unwrap();
        let memory = Counted {
            memory: snapshot.roster_view(&source),
            copies: 0.into(),
            bytes: 0.into(),
            _world: std::marker::PhantomData,
        };
        let mut owned = Owned::new(
            COUNTRIES,
            COUNTRIES * UNLOCKS,
            registry_slots(TACTICS as u32 + 1),
            (TACTICS + 1).max(UNLOCKS) * 8,
        );
        let measure = |owned: &mut Owned, reuse: &dyn Fn(&mut Owned) -> Capture| {
            let mut best = std::time::Duration::MAX;
            let mut result = None;
            for _ in 0..5 {
                memory.copies.set(0);
                memory.bytes.set(0);
                let begin = std::time::Instant::now();
                let capture = reuse(owned);
                best = best.min(begin.elapsed());
                result = Some(capture);
            }
            (
                result.unwrap(),
                memory.copies.get(),
                memory.bytes.get(),
                best,
            )
        };
        let (fresh, copies, read, best) =
            measure(&mut owned, &|owned| owned.capture(&memory, base, game));
        assert_eq!(fresh.outcome, Outcome::Rows { len: COUNTRIES });
        assert_eq!(fresh.required_unlocks, COUNTRIES * UNLOCKS);
        assert_eq!(owned.unlocks[..3], [2, 3, 4]);
        println!(
            "country tactic unlocks, {COUNTRIES} countries x {UNLOCKS} unlocks, {TACTICS} tactics, \
             fresh: {copies} copies, {read} bytes, best of 5 {best:?}"
        );
        // One row copy and one list copy per country, two per loaded tactic
        // and the fixed roots.
        assert!(copies <= 2 * COUNTRIES + 2 * (TACTICS + 1) + 16, "{copies}");
    }
}
