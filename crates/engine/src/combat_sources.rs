//! Per-pass sharing of the land combat division walk's per-source reads.
//!
//! The hourly capture holds the simulation thread for its whole pass, so a
//! source read earlier in the pass holds the value a later division would read
//! again. The division walk (`carmy_combat_inputs`, `land_reinforcement_inputs`)
//! shares whole per-source results by the source's address: a country's
//! diplomacy walks, a state's modifier source, a province's weather, a
//! template's terms, the module's defines. Each is computed at its first use,
//! in the walk's own order, and kept with its failure. A result read in stages
//! keeps each stage's failure apart, so every division meets the value or the
//! reason main's reader meets, at the point where main meets it.
//!
//! A combat's divisions' own army and stat block fields are gathered in two
//! guarded calls ([`Images`]) and served to each division's readers through
//! its [`DivisionImage`].
//!
//! [`Sources::clear`] starts each pass; nothing crosses hours. Storage is kept
//! between passes: the maps and the flat arenas below grow on the acquisition
//! worker to the pass's exact demand and are reused. The equivalence tests run
//! a test-only copy of main's readers (`main_reinforcement`, `main_combat`) as
//! the baseline and compare every output and every failure.

use crate::{
    Memory,
    memory::{Raw, RecordField, RecordLayout, RecordSources},
    read,
};
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hash, Hasher},
};

#[cfg(test)]
mod main_combat;
#[cfg(test)]
mod main_reinforcement;

/// Multiplicative hashing of the few integer keys used here; the keys are
/// addresses and ids, not attacker-chosen input.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

pub(crate) type Map<K, V> = HashMap<K, V, BuildHasherDefault<KeyHasher>>;
pub(crate) type Shared<K, V> = Map<K, Result<V, &'static str>>;
type LeaderChain = Result<Option<usize>, &'static str>;

/// A run of one of the pass's flat arenas.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    pub start: usize,
    pub len: usize,
}

impl Run {
    pub fn of<T>(self, arena: &[T]) -> &[T] {
        &arena[self.start..self.start + self.len]
    }
}

/// One pass's shared sources. Construct with [`Sources::new`], call
/// [`Sources::clear`] before each pass, and keep the value between passes so
/// its storage is reused.
#[derive(Default)]
pub struct Sources {
    /// Scalars of shared objects (module defines, provinces, states), by
    /// address and width.
    fields: Map<(usize, usize), Option<[u8; 8]>>,
    pub(crate) definitions: Shared<usize, usize>,
    pub(crate) weather: Shared<usize, (usize, usize)>,
    pub(crate) state_sources: Shared<(usize, u32), usize>,
    pub(crate) controllers: Shared<(u32, u32), u8>,
    pub(crate) core_supply: Shared<(u32, usize), bool>,
    pub(crate) infrastructure: Shared<usize, crate::land_reinforcement_inputs::Infrastructure>,
    pub(crate) air_owners: Shared<usize, bool>,
    pub(crate) air_entries: Shared<(usize, usize), (u32, [i64; 3])>,
    pub(crate) supply_terms: Shared<usize, crate::land_reinforcement_inputs::SupplyEntries>,
    pub(crate) supply_arena: Vec<crate::land_reinforcement_inputs::SupplyTerm>,
    pub(crate) priorities: Shared<usize, crate::land_reinforcement_inputs::Priority>,
    pub(crate) category_tokens: Vec<u32>,
    pub(crate) metadata: Shared<u32, (u32, u32)>,
    /// Reinforcement accumulator censuses, read as main's direct reader reads
    /// them, and their sorted entries.
    pub(crate) censuses: Shared<usize, Run>,
    pub(crate) census_entries: Vec<(u32, i64)>,
    pub(crate) transport_owners: Shared<usize, crate::land_reinforcement_inputs::TransportOwner>,
    pub(crate) transport_enemies: Vec<crate::land_reinforcement_inputs::TransportEnemy>,
    pub(crate) regional: Shared<(usize, u32), i64>,
    pub(crate) widths: Shared<usize, crate::carmy_combat_inputs::TemplateWidth>,
    pub(crate) width_arena: Vec<crate::carmy_combat_inputs::WidthTerm>,
    pub(crate) context_metadata:
        Option<Result<crate::carmy_combat_inputs::ContextMetadata, &'static str>>,
    pub(crate) context_modifiers: Shared<usize, [crate::carmy_combat_inputs::ContextModifier; 3]>,
    /// The division `capture_stats` authenticated last: its pointer, handle
    /// and stat block. The same division's later readers skip those reads.
    pub(crate) verified: Option<(usize, u64, usize)>,
    /// The last division's leader chain (`carmy_leader::movement_accumulator`)
    /// by pointer and handle: the context reader reads it, and the same
    /// division's reinforcement reader reuses it.
    leader: Option<(usize, u64, LeaderChain)>,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a pass: forgets every value, keeps the storage.
    pub fn clear(&mut self) {
        self.fields.clear();
        self.definitions.clear();
        self.weather.clear();
        self.state_sources.clear();
        self.controllers.clear();
        self.core_supply.clear();
        self.infrastructure.clear();
        self.air_owners.clear();
        self.air_entries.clear();
        self.supply_terms.clear();
        self.supply_arena.clear();
        self.priorities.clear();
        self.category_tokens.clear();
        self.metadata.clear();
        self.censuses.clear();
        self.census_entries.clear();
        self.transport_owners.clear();
        self.transport_enemies.clear();
        self.regional.clear();
        self.widths.clear();
        self.width_arena.clear();
        self.context_metadata = None;
        self.context_modifiers.clear();
        self.verified = None;
        self.leader = None;
    }

    /// A shared object's scalar; `reason` is the unshared read's failure.
    pub(crate) fn field<T: Raw>(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        offset: usize,
        reason: &'static str,
    ) -> Result<T, &'static str> {
        const { assert!(size_of::<T>() <= 8) };
        let address = pointer.checked_add(offset).ok_or(reason)?;
        let bytes = *self
            .fields
            .entry((address, size_of::<T>()))
            .or_insert_with(|| {
                let mut bytes = [0; 8];
                crate::read_slice(memory, address, &mut bytes[..size_of::<T>()]).then_some(bytes)
            });
        let bytes = bytes.ok_or(reason)?;
        read(
            &crate::memory::SliceMemory {
                base: 1,
                bytes: &bytes,
            },
            1,
        )
        .ok_or(reason)
    }

    /// `carmy_leader::movement_accumulator` for this division, read once: the
    /// pass holds the simulation thread, so the same division's second call
    /// would read the same chain.
    pub(crate) fn leader_chain(
        &mut self,
        memory: &impl Memory,
        base: usize,
        army: usize,
        handle: u64,
    ) -> Result<Option<usize>, &'static str> {
        if let Some((pointer, verified, result)) = self.leader
            && pointer == army
            && verified == handle
        {
            return result;
        }
        let result = crate::carmy_leader::movement_accumulator(memory, base, army);
        self.leader = Some((army, handle, result));
        result
    }
}

/// The value `compute` gives for `key`, computed once per pass.
pub(crate) fn share<K: Hash + Eq, V>(
    map: &mut Shared<K, V>,
    key: K,
    compute: impl FnOnce() -> Result<V, &'static str>,
) -> Result<&V, &'static str> {
    match map.entry(key).or_insert_with(compute) {
        Ok(value) => Ok(value),
        Err(reason) => Err(*reason),
    }
}

const fn span(start: usize, end: usize, destination: usize) -> RecordField {
    RecordField {
        source_offset: start,
        destination_offset: destination,
        len: end - start,
    }
}

/// The CArmy spans one division's readers read (stats, context,
/// reinforcement, the modifier member and the leader chain), packed in order:
/// identity/class/handle; formation and supply bytes; the orders group; the
/// stat block; modifier data and attack accumulator; owners, location and
/// route; retreat flags; the transfer membership; the name member; the
/// template; the strength factor; the direction groups; supply days;
/// acclimatization; supply and fuel. Small gaps inside a span are the same object's bytes.
const ARMY_FIELDS: [RecordField; 16] = [
    span(0x000, 0x020, 0x000),
    span(0x028, 0x060, 0x020),
    span(0x0c0, 0x0c8, 0x058),
    span(0x138, 0x140, 0x060),
    span(0x170, 0x1a8, 0x068),
    span(0x1d8, 0x210, 0x0a0),
    span(0x24c, 0x24e, 0x0d8),
    span(0x2ae, 0x2af, 0x0da),
    span(0x2f8, 0x300, 0x0e0),
    span(0x340, 0x348, 0x0e8),
    span(0x3b8, 0x3c0, 0x0f0),
    span(0x468, 0x470, 0x0f8),
    span(0x498, 0x49c, 0x100),
    // The direction group (`carmy_combat_inputs::DIRECTION_GROUP`), which the
    // stat reader's naming scatter reads.
    span(0x4a8, 0x4ac, 0x104),
    span(0x4c0, 0x4f8, 0x108),
    span(0x600, 0x628, 0x140),
];
const ARMY_STRIDE: usize = 0x168;
/// The stat block spans the readers read: identity; defence to initiative;
/// fuel capacity; anti-air; base speed, armour and piercing.
const STAT_FIELDS: [RecordField; 6] = [
    span(0x000, 0x008, 0x000),
    span(0x0a8, 0x0e8, 0x008),
    span(0x120, 0x128, 0x048),
    span(0x220, 0x228, 0x050),
    span(0x290, 0x2a8, 0x058),
    // The equipment category mask (`csub_unit_definition::AIR_FLAGS`).
    span(0x5a8, 0x5b0, 0x070),
];
const STAT_STRIDE: usize = 0x78;
const ARMY_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ARMY_FIELDS, ARMY_STRIDE).unwrap();
const STAT_LAYOUT: RecordLayout<'static> = RecordLayout::new(&STAT_FIELDS, STAT_STRIDE).unwrap();
/// Where the army record keeps the stat block pointer (`cunit::STAT_BLOCK`).
const STAT_POINTER: usize = 0x060;
const _: () = assert!(
    ARMY_FIELDS[3].source_offset == crate::cunit::STAT_BLOCK
        && ARMY_FIELDS[3].destination_offset == STAT_POINTER
);

/// Many divisions' army and stat block fields, gathered with one guarded call
/// for the armies and one for their stat blocks (a failing record costs one
/// more call, which resumes after it). The pass holds the simulation thread,
/// so each field is the value its reader would read; a division whose record
/// did not copy completely is served nothing, and its reads go to the source
/// one by one, failing where they fail. Storage grows on the acquisition
/// worker and is reused.
#[derive(Default)]
pub struct Images {
    armies: Vec<usize>,
    stats: Vec<usize>,
    army_bytes: Vec<u8>,
    stat_bytes: Vec<u8>,
    army_complete: Vec<bool>,
    stat_complete: Vec<bool>,
}

impl Images {
    /// Holds no division: every read goes to the source.
    pub fn clear(&mut self) {
        self.armies.clear();
    }

    /// Gathers the images of `armies` (the census's authenticated pointers).
    pub fn capture(&mut self, memory: &impl Memory, armies: &[usize]) {
        let count = armies.len();
        self.armies.clear();
        self.armies.extend_from_slice(armies);
        self.army_bytes.resize(count * ARMY_STRIDE, 0);
        self.army_complete.resize(count, false);
        gather(
            memory,
            &self.armies,
            &ARMY_LAYOUT,
            &mut self.army_bytes,
            &mut self.army_complete,
        );
        self.stats.clear();
        for row in 0..count {
            let at = row * ARMY_STRIDE + STAT_POINTER;
            self.stats.push(if self.army_complete[row] {
                u64::from_le_bytes(self.army_bytes[at..at + 8].try_into().unwrap()) as usize
            } else {
                0
            });
        }
        self.stat_bytes.resize(count * STAT_STRIDE, 0);
        self.stat_complete.resize(count, false);
        gather(
            memory,
            &self.stats,
            &STAT_LAYOUT,
            &mut self.stat_bytes,
            &mut self.stat_complete,
        );
    }

    /// The memory `army`'s readers read through: its image when `row` holds
    /// that division, else the source alone.
    pub fn division<'a, M: Memory>(
        &'a self,
        inner: &'a M,
        row: usize,
        army: usize,
    ) -> DivisionImage<'a, M> {
        let held = self.armies.get(row) == Some(&army) && self.army_complete[row];
        let stat = held && self.stat_complete[row];
        DivisionImage {
            inner,
            army,
            army_row: held.then(|| &self.army_bytes[row * ARMY_STRIDE..][..ARMY_STRIDE]),
            stat: if stat { self.stats[row] } else { 0 },
            stat_row: stat.then(|| &self.stat_bytes[row * STAT_STRIDE..][..STAT_STRIDE]),
        }
    }
}

/// Copies every row of `pointers` it can, marking each complete one. A null
/// pointer is skipped; a failing record ends its call and the next resumes
/// after it, so nothing is read twice.
fn gather(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    bytes: &mut [u8],
    complete: &mut [bool],
) {
    let (fields, stride) = (layout.field_count(), layout.stride());
    complete.fill(false);
    let mut row = 0;
    while row < pointers.len() {
        if pointers[row] == 0 {
            row += 1;
            continue;
        }
        let end = pointers[row..]
            .iter()
            .position(|&pointer| pointer == 0)
            .map_or(pointers.len(), |run| row + run);
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[row..end]),
            layout,
            &mut bytes[row * stride..end * stride],
        ) / fields;
        complete[row..row + copied].fill(true);
        // The failed record, if any, is skipped.
        row += copied + usize::from(row + copied < end);
    }
}

/// One division's army and stat block fields from [`Images`], serving every
/// read that falls inside one of their spans; everything else goes to the
/// source.
pub struct DivisionImage<'a, M> {
    inner: &'a M,
    army: usize,
    army_row: Option<&'a [u8]>,
    stat: usize,
    stat_row: Option<&'a [u8]>,
}

impl<'a, M: Memory> DivisionImage<'a, M> {
    /// The image bytes for `[address, address + len)`, when one span holds
    /// them all.
    fn served(&self, address: usize, len: usize) -> Option<&'a [u8]> {
        let within = |base: usize, row: Option<&'a [u8]>, fields: &[RecordField]| {
            let row = row?;
            let offset = address.checked_sub(base)?;
            let end = offset.checked_add(len)?;
            fields
                .iter()
                .find(|field| {
                    field.len != 0
                        && field.source_offset <= offset
                        && end <= field.source_offset + field.len
                })
                .map(|field| {
                    let start = field.destination_offset + offset - field.source_offset;
                    &row[start..start + len]
                })
        };
        if address == 0 {
            return None;
        }
        within(self.army, self.army_row, &ARMY_FIELDS)
            .or_else(|| within(self.stat, self.stat_row, &STAT_FIELDS))
    }
}

impl<M: Memory> Memory for DivisionImage<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        match self.served(address, destination.len()) {
            Some(bytes) => {
                destination.copy_from_slice(bytes);
                true
            }
            None => self.inner.copy(address, destination),
        }
    }
    fn prefetch(&self, address: usize, len: usize) {
        if self.served(address, len).is_none() {
            self.inner.prefetch(address, len);
        }
    }
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.inner.authenticated_countries(base, game_state)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.inner.modifier_accumulator_value(address, key)
    }
    fn core_supply_access(
        &self,
        base: usize,
        game_state: usize,
        owner_raw: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        self.inner
            .core_supply_access(base, game_state, owner_raw, area)
    }
    fn movement_primary_root(
        &self,
        base: usize,
        member: usize,
    ) -> Option<Result<usize, &'static str>> {
        self.inner.movement_primary_root(base, member)
    }
    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        self.inner
            .movement_air_defense_value(defense, denominator, reduction)
    }
    /// All lanes from the image, or the whole scatter from the source: one
    /// guarded call either way, and the same bytes.
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if addresses
            .iter()
            .zip(destinations.iter())
            .all(|(&address, destination)| self.served(address, destination.len()).is_some())
        {
            for (&address, destination) in addresses.iter().zip(destinations.iter_mut()) {
                let bytes = self.served(address, destination.len()).unwrap();
                destination.copy_from_slice(bytes);
            }
            return N;
        }
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if addresses
            .iter()
            .zip(destinations.iter())
            .all(|(&address, destination)| self.served(address, destination.len()).is_some())
        {
            return self.copy_scattered(addresses, destinations);
        }
        self.inner
            .copy_scattered_prefetched(addresses, destinations)
    }
    /// One guarded gather at the source; a gather never touches the image.
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        carmy, carmy_combat_inputs as inputs, cbuilding_database, ccountry, cgame_state,
        corders_group,
        cpdx_array::Array,
        cprovince, creferenced_division_template as template, cstate, cstrategic_region,
        csub_unit_definition, cunit, cunit_leader, cweather_manager,
        land_reinforcement_inputs as reinforcement,
        memory::{RecordLayout, RecordSources},
        modifier_definition, province_definition, sweather_per_region,
    };
    use std::cell::{Cell, RefCell};

    const PAGE: usize = 4096;
    const GAME: usize = 0x4000_0000;
    /// Addresses below this fault, as a process's first pages do; a null
    /// object's field (address 8, say) fails as it does live.
    const UNMAPPED: usize = 0x1_0000;

    /// Sparse memory: every mapped address reads, as zero unless written.
    #[derive(Default)]
    struct World {
        pages: HashMap<usize, Box<[u8; PAGE]>>,
        next: usize,
        /// Guarded calls; a scatter or a record gather is one, as live.
        copies: Cell<usize>,
        roster: Vec<Option<ccountry::Country>>,
        arrays: Option<[Array; 2]>,
        /// Whether country lookups use the authenticated roster, as the
        /// Ordinary job's memory does.
        authenticated: bool,
        /// Byte ranges whose reads fail.
        poisoned: RefCell<Vec<(usize, usize)>>,
        /// Every guarded span, when logging.
        log: RefCell<Option<Vec<(usize, usize)>>>,
    }

    impl World {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                let address = at + index;
                self.pages
                    .entry(address / PAGE)
                    .or_insert_with(|| Box::new([0; PAGE]))[address % PAGE] = *byte;
            }
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn alloc(&mut self, size: usize) -> usize {
            if self.next == 0 {
                self.next = 0x1_0000_0000;
            }
            let at = self.next;
            self.next += size.next_multiple_of(64);
            at
        }
        /// A CPdxArray header: data, capacity, count.
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        fn object(&mut self, size: usize, vtable: usize) -> usize {
            let at = self.alloc(size);
            self.u64(at, vtable as u64);
            at
        }
        /// A sorted modifier accumulator census.
        fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
            let data = self.alloc(entries.len().max(1) * 16);
            for (index, (key, value)) in entries.iter().enumerate() {
                self.u32(data + index * 16, *key);
                self.u64(data + index * 16 + 8, *value as u64);
            }
            self.array(at, data, entries.len() as u32);
        }
        fn get_u64(&self, at: usize) -> u64 {
            let mut bytes = [0; 8];
            assert!(self.fill(at, &mut bytes));
            u64::from_le_bytes(bytes)
        }
        fn fill(&self, address: usize, output: &mut [u8]) -> bool {
            if address < UNMAPPED
                || address.checked_add(output.len()).is_none()
                || self
                    .poisoned
                    .borrow()
                    .iter()
                    .any(|&(at, len)| address < at + len && at < address + output.len())
            {
                return false;
            }
            for (index, byte) in output.iter_mut().enumerate() {
                let at = address + index;
                *byte = self
                    .pages
                    .get(&(at / PAGE))
                    .map_or(0, |page| page[at % PAGE]);
            }
            true
        }
        fn span(&self, address: usize, output: &mut [u8]) -> bool {
            if let Some(log) = self.log.borrow_mut().as_mut() {
                log.push((address, output.len()));
            }
            self.fill(address, output)
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.span(address, output)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.copies.set(self.copies.get() + 1);
            for (ordinal, (address, destination)) in
                addresses.into_iter().zip(destinations).enumerate()
            {
                if !destination.is_empty() && !self.span(address, destination) {
                    return ordinal;
                }
            }
            N
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.copies.set(self.copies.get() + 1);
            let mut completed = 0;
            for row in 0..sources.len() {
                for field in layout.fields() {
                    if field.len != 0 {
                        let Some(address) = sources
                            .pointer(row)
                            .filter(|&pointer| pointer != 0)
                            .and_then(|pointer| pointer.checked_add(field.source_offset))
                        else {
                            return completed;
                        };
                        let start = row * layout.stride() + field.destination_offset;
                        if !self.span(address, &mut destination[start..start + field.len]) {
                            return completed;
                        }
                    }
                    completed += 1;
                }
            }
            completed
        }
        fn authenticated_countries(
            &self,
            _base: usize,
            game: usize,
        ) -> Option<ccountry::AuthenticatedRoster<'_>> {
            (self.authenticated && game == GAME)
                .then(|| {
                    self.arrays
                        .map(|arrays| ccountry::AuthenticatedRoster::new(&self.roster, arrays))
                })
                .flatten()
        }
        /// The core-supply predicate's own tests cover its native walk; here
        /// it is one counted call whose answer depends only on its inputs, so
        /// the test sees whether the walk asks it once per (owner, area).
        fn core_supply_access(
            &self,
            _base: usize,
            _game: usize,
            owner_raw: u32,
            area: usize,
        ) -> Option<Result<bool, &'static str>> {
            self.copies.set(self.copies.get() + 1);
            Some(Ok(!(owner_raw as usize + area / 64).is_multiple_of(3)))
        }
    }

    struct Shape {
        countries: u32,
        divisions: usize,
        owners: usize,
        templates: usize,
        states: usize,
        provinces: usize,
        /// Owned relations each owner's diplomacy walks, none a war.
        relations: usize,
        transports: usize,
        /// Army leaders; every fourth division, from the second, has one.
        leaders: usize,
    }

    /// Addresses the targeted fault cases break.
    struct Built {
        world: World,
        /// Divisions in combat: (army pointer, handle).
        divisions: Vec<(usize, u64)>,
        /// Per owner: its intel table and its navy's regional operands.
        intel: Vec<usize>,
        regional: Vec<usize>,
        /// Enemy owner indices, per owner.
        enemies: Vec<Vec<usize>>,
        subunits: Vec<usize>,
        /// The modifier definition table.
        table: usize,
    }

    /// The priority sub-units' token: a key every owner's country census has
    /// and no earlier reader of the division looks up.
    const TOKEN: u32 = 0x1a0;

    fn world(shape: &Shape) -> Built {
        let mut w = World::default();
        let slots = shape.countries as usize + 1;
        // Module data: the definition table, defines and singletons.
        let table = w.alloc(0x78 * 0x300);
        w.u64(modifier_definition::TABLE_RVA, table as u64);
        for key in [0x2b_usize, 0x5e, 0x17f, 0x1e9, 0xfb, 0x10f, TOKEN as usize] {
            w.u32(
                table + key * modifier_definition::STRIDE + modifier_definition::FLAGS,
                3,
            );
        }
        let database = w.alloc(0x400);
        w.u64(cbuilding_database::SINGLETON_RVA, database as u64);
        let definition = w.alloc(0x400);
        w.u64(database + 0x3b0, definition as u64);
        // Countries, links and an authenticated roster.
        let countries = w.alloc(slots * 8);
        let links = w.alloc(slots * 4);
        w.array(GAME + cgame_state::COUNTRIES, countries, slots as u32);
        w.array(GAME + cgame_state::COUNTRY_LINKS, links, slots as u32);
        let relation = w.alloc(0x40);
        w.u32(relation + 8, 0x3801);
        let air_manager = w.alloc(0x100);
        w.u64(
            GAME + cgame_state::STRATEGIC_AIR_MANAGER,
            air_manager as u64,
        );
        let air_countries = w.alloc(slots * 8);
        w.array(air_manager + 0x30, air_countries, slots as u32);
        w.roster = vec![None; slots];
        for slot in 1..slots {
            w.u32(links + slot * 4, slot as u32);
        }
        let mut owners = Vec::new();
        let mut diplomacies = Vec::new();
        let mut objects = Vec::new();
        let mut regional_data = Vec::new();
        w.u32(modifier_definition::TRADE_FEATURE_FLAGS_RVA, 1 << 4);
        let definitions = w.alloc(0x100);
        w.u64(0x0331_7cc8, definitions as u64);
        let static_modifier = w.object(0x100, crate::cstatic_modifier::VTABLE_RVA);
        w.accumulator(
            static_modifier + crate::cstatic_modifier::ACCUMULATOR,
            &[(0x29, 6)],
        );
        w.u64(definitions + 0xf8, static_modifier as u64);
        w.u32(
            table + 0x29 * modifier_definition::STRIDE + modifier_definition::CATEGORY_MASK,
            5,
        );
        let navy_manager = w.object(0x100, crate::cstrategic_navy_manager::VTABLE_RVA);
        w.u64(
            GAME + cgame_state::STRATEGIC_NAVY_MANAGER,
            navy_manager as u64,
        );
        let navies = w.alloc(slots * 8);
        w.array(
            navy_manager + crate::cstrategic_navy_manager::NAVIES,
            navies,
            slots as u32,
        );
        for owner in 0..shape.owners {
            let slot = 1 + owner * (slots - 1) / shape.owners.max(1);
            let country = w.object(0x1000, ccountry::VTABLE_RVA);
            w.u32(country + ccountry::ID, slot as u32);
            w.u64(countries + slot * 8, country as u64);
            w.accumulator(
                country + ccountry::MODIFIER_ACCUMULATOR,
                &[
                    (0x2b, 5_000),
                    (0x5e, -2_000),
                    (0xfb, 7),
                    (0x17f, 11),
                    (TOKEN, 17),
                    (0x1e9, 13),
                ],
            );
            let diplomacy = w.object(0x300, crate::cdiplomacy_status::VTABLE_RVA);
            w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            w.u64(
                diplomacy + crate::cdiplomacy_status::COUNTRY,
                country as u64,
            );
            diplomacies.push(diplomacy);
            objects.push(country);
            // A fog network, so the country can be another's transport enemy.
            w.u32(
                country + ccountry::FOG_NETWORK_TABLE_COUNT,
                u32::from(owner % 4 != 3),
            );
            let navy = w.object(0x100, crate::cstrategic_navy::VTABLE_RVA);
            w.u64(
                navy + 8,
                crate::cstrategic_navy::SECONDARY_VTABLE_RVA as u64,
            );
            w.u32(navy + crate::cstrategic_navy::OWNER, slot as u32);
            let regional = w.alloc(16);
            w.u64(regional, 700 + owner as u64);
            w.u64(regional + 8, 800 + owner as u64);
            w.array(
                navy + crate::cstrategic_navy::REGIONAL_OPERANDS,
                regional,
                2,
            );
            regional_data.push(regional);
            w.u64(navies + slot * 8, navy as u64);
            let list = w.alloc(shape.relations.max(1) * 8);
            for index in 0..shape.relations {
                w.u64(list + index * 8, relation as u64);
            }
            w.array(
                diplomacy + crate::cdiplomacy_status::OWNED_RELATIONS,
                list,
                shape.relations as u32,
            );
            let air = w.alloc(0x100);
            w.u64(air_countries + slot * 8, air as u64);
            let regions = w.alloc(0xa0 * 4);
            w.array(air + 0xe0, regions, 4);
            for index in 0..4 {
                w.u64(regions + index * 0xa0, 100 + index as u64);
                w.u64(regions + index * 0xa0 + 8, index as u64);
                w.u64(regions + index * 0xa0 + 0x18, 900 + index as u64);
            }
            w.roster[slot] = Some(ccountry::Country {
                pointer: country,
                canonical_slot: slot as u32,
                id: slot as u32,
            });
            owners.push(slot as u32);
        }
        // Intel tables and enemy lists, now every owner's slot is known.
        let mut intel_tables = Vec::new();
        let mut enemy_lists = Vec::new();
        for (index, (&country, &diplomacy)) in objects.iter().zip(&diplomacies).enumerate() {
            let intel = w.object(0x100, crate::ccountry_intel::VTABLE_RVA);
            w.u32(intel + crate::ccountry_intel::OWNER, owners[index]);
            let rows = w.alloc(slots * crate::country_intel_row::STRIDE);
            for (other, &slot) in owners.iter().enumerate() {
                w.u64(
                    rows + slot as usize * crate::country_intel_row::STRIDE
                        + crate::country_intel_row::NAVY,
                    (index * 100 + other) as u64,
                );
            }
            w.array(intel + crate::ccountry_intel::DATA, rows, slots as u32);
            w.u64(country + ccountry::INTEL_TABLE, intel as u64);
            intel_tables.push(intel);
            // CDiplomacyStatus+0xA4, the at-war test in `air()`, is this
            // list's count, so an owner with enemies never walks its
            // relations. Every third owner is at war with the next three (one
            // of which has capitulated); the rest have no enemies and walk
            // their relations.
            let enemies: Vec<usize> = if index % 3 == 0 {
                (1..=3).map(|step| (index + step) % owners.len()).collect()
            } else {
                Vec::new()
            };
            let list = w.alloc(enemies.len().max(1) * 4);
            for (at, enemy) in enemies.iter().enumerate() {
                w.u32(list + at * 4, owners[*enemy]);
            }
            w.array(
                diplomacy + crate::cdiplomacy_status::ENEMY_COUNTRIES,
                list,
                enemies.len() as u32,
            );
            enemy_lists.push(enemies);
            w.put(
                diplomacy + crate::cdiplomacy_status::CAPITULATED,
                &[u8::from(index % 5 == 4)],
            );
        }
        // Each owner's relation matrix row: a tuple per other owner, none at
        // war, every other pair friendly through +2A0.
        for (from, &diplomacy) in diplomacies.iter().enumerate() {
            let row = w.alloc(slots * 8);
            w.array(
                diplomacy + crate::cdiplomacy_status::RELATIONS,
                row,
                slots as u32,
            );
            for (to, &target) in owners.iter().enumerate() {
                if to == from {
                    continue;
                }
                let tuple = w.object(0x300, crate::crelation_status::VTABLE_RVA);
                w.u64(
                    tuple + crate::crelation_status::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                w.u32(tuple + crate::crelation_status::TARGET_COUNTRY, target);
                w.u64(tuple + 0x2a0, u64::from((from + to) % 2 == 0));
                w.u64(row + target as usize * 8, tuple as u64);
            }
        }
        w.arrays = Some([
            Array::read(&w, GAME + cgame_state::COUNTRIES, 8).unwrap(),
            Array::read(&w, GAME + cgame_state::COUNTRY_LINKS, 4).unwrap(),
        ]);
        // Weather: no province records; two strategic regions with records.
        let weather = w.object(0x100, cweather_manager::VTABLE_RVA);
        w.u64(GAME + cgame_state::WEATHER_MANAGER, weather as u64);
        let records = w.alloc(sweather_per_region::STRIDE * 2);
        w.array(weather + cweather_manager::REGIONS, records, 2);
        let mut regions = Vec::new();
        for id in 0..2_u32 {
            let record = records + id as usize * sweather_per_region::STRIDE;
            w.u64(record, sweather_per_region::VTABLE_RVA as u64);
            w.u32(record + sweather_per_region::ID, id);
            w.accumulator(record + sweather_per_region::MODIFIERS, &[(0x29, 3)]);
            let region = w.object(0x100, cstrategic_region::VTABLE_RVA);
            w.u32(region + cstrategic_region::ID, id);
            w.u32(region + 0x60, id + 1);
            regions.push(region);
        }
        // Sub-units, half with a priority token and categories, and templates.
        let subunits: Vec<usize> = (0..6)
            .map(|index| {
                let unit = w.object(0x600, csub_unit_definition::VTABLE_RVA);
                w.u32(unit + 8, 40 + index);
                w.u64(unit + 0x58, 200_000 + u64::from(index) * 50_000);
                if index % 2 == 0 {
                    w.u32(unit + 0x5d8, TOKEN);
                    let categories = w.alloc(16);
                    for (at, token) in [0x17f_u32, 0].into_iter().enumerate() {
                        let category = w.alloc(0x80);
                        w.u32(category + 0x60, token);
                        w.u64(categories + at * 8, category as u64);
                    }
                    w.array(unit + 0x508, categories, 1 + (index / 2) % 2);
                }
                unit
            })
            .collect();
        let templates: Vec<usize> = (0..shape.templates)
            .map(|index| {
                let t = w.object(0x600, template::VTABLE_RVA);
                w.u64(
                    t + 8,
                    ((index as u64 + 1) << 32) | u64::from(template::REFERENCE_TYPE),
                );
                let terms = w.alloc(16 * 4);
                let count = 2 + index % 3;
                for term in 0..count {
                    w.u64(terms + term * 16, subunits[(index + term) % 6] as u64);
                    w.u32(terms + term * 16 + 8, 1 + term as u32);
                }
                w.array(t + 0xc0, terms, count as u32);
                w.u64(t + template::PRIORITY_UNIT, subunits[index % 6] as u64);
                t
            })
            .collect();
        // States and provinces, registered by id.
        let states: Vec<usize> = (0..shape.states)
            .map(|index| {
                let state = w.object(0x1000, cstate::VTABLE_RVA);
                w.u32(state + cstate::ID, 500 + index as u32);
                w.u32(state + cstate::CONTROLLER, owners[index % owners.len()]);
                let buckets = w.alloc(0x40);
                w.u64(state + cstate::COUNTRY_MODIFIERS, buckets as u64);
                w.accumulator(
                    state + cstate::RESOURCE_MODIFIER + 0x10,
                    &[(0x2b, 4), (0x1e7, 9)],
                );
                state
            })
            .collect();
        let first_province = 1000;
        let registered = w.alloc((first_province + shape.provinces) * 8);
        w.array(
            GAME + cgame_state::PROVINCES,
            registered,
            (first_province + shape.provinces) as u32,
        );
        let provinces: Vec<usize> = (0..shape.provinces)
            .map(|index| {
                let id = first_province + index;
                let province = w.object(0x200, cprovince::VTABLE_RVA);
                let definition = w.alloc(0x100);
                w.u32(province + cprovince::ID, id as u32);
                w.u32(definition + province_definition::ID, id as u32);
                w.u64(province + cprovince::DEFINITION, definition as u64);
                w.u64(
                    province + cprovince::STATE,
                    states[index % states.len()] as u64,
                );
                w.u32(
                    province + cprovince::CONTROLLER,
                    owners[index % owners.len()],
                );
                w.u64(
                    province + cprovince::CONTROLLER_AREA,
                    (0x9000 + (index % 5) * 64) as u64,
                );
                if index % 4 != 3 {
                    w.u64(
                        province + cprovince::STRATEGIC_REGION,
                        regions[index % 2] as u64,
                    );
                }
                w.u64(registered + id * 8, province as u64);
                province
            })
            .collect();
        // Army leaders, each in its own orders group.
        let groups: Vec<usize> = (0..shape.leaders)
            .map(|index| {
                let leader = w.object(0x1000, crate::carmy_leader::VTABLE_RVA);
                w.u64(
                    leader + cunit_leader::TYPE,
                    ((index as u64 + 1) << 32) | u64::from(cunit_leader::REFERENCE_TYPE),
                );
                w.accumulator(
                    leader + crate::carmy_leader::MOVEMENT_MODIFIER + 0x10,
                    &[(0x2b, 21), (0x5e, -22), (0x10f, 23), (0x17f, 24)],
                );
                let group = w.object(0x200, corders_group::VTABLE_RVA);
                w.u64(
                    group + corders_group::HANDLE,
                    ((index as u64 + 1) << 32) | u64::from(corders_group::REFERENCE_TYPE),
                );
                w.u64(group + corders_group::LEADER, leader as u64);
                group
            })
            .collect();
        // Acclimatization definitions.
        let climates: Vec<usize> = (0..2)
            .map(|index| {
                let climate = w.alloc(0x40);
                w.u64(climate + 0x30, 300 + index as u64);
                climate
            })
            .collect();
        let mut divisions = Vec::new();
        for index in 0..shape.divisions {
            let army = w.object(0x800, carmy::VTABLE_RVA);
            let handle = ((index as u64 + 1) << 32) | 3;
            w.u64(army + cunit::HANDLE, handle);
            let stats = w.object(0x600, csub_unit_definition::VTABLE_RVA);
            w.u64(army + cunit::STAT_BLOCK, stats as u64);
            let province = provinces[(index * 7) % provinces.len()];
            w.u64(army + cunit::LOCATION, province as u64);
            w.u64(
                army + carmy::STANDARD_TEMPLATE,
                templates[(index * 5) % templates.len()] as u64,
            );
            let owner = owners[index % owners.len()];
            w.u32(army + cunit::ROSTER_OWNER, owner);
            w.u32(army + cunit::LOGICAL_OWNER, owner);
            // A generated name (programmatic override flag set), of a
            // nonzero type every other division; neither reads further.
            let name = w.alloc(0x100);
            w.u64(army + inputs::NAME_GROUP_MEMBER, name as u64);
            w.u64(name, crate::cname_group_member::VTABLE_RVA as u64);
            w.u32(name + crate::cname_group_member::TYPE, (index % 2) as u32);
            w.u32(name + crate::cname_group_member::ORDER, index as u32);
            w.put(name + crate::cname_group_member::FLAG_B, &[1]);
            // Every other division bypasses core supply (+0x2E); the rest ask
            // the predicate, which half of them fail.
            w.put(army + 0x2e, &[u8::from(index % 2 == 0)]);
            if !groups.is_empty() && index % 4 == 1 {
                w.u64(
                    army + carmy::ORDERS_GROUP,
                    groups[(index / 4) % groups.len()] as u64,
                );
            }
            if index % 3 == 0 {
                let object = army + carmy::ACCLIMATIZATION;
                w.u64(object + 0x20, climates[index % 2] as u64);
                let entries = w.alloc(32);
                for (at, &climate) in climates.iter().enumerate() {
                    w.u64(entries + at * 16, climate as u64);
                    w.u64(entries + at * 16 + 8, (index + at) as u64);
                }
                w.array(object + 8, entries, 2);
                w.u64(object + 0x38, 40);
            }
            if index % 5 == 2 {
                // A route: the next province and the destination.
                let route = w.alloc(8);
                w.u32(
                    route,
                    (first_province + (index + 1) % shape.provinces) as u32,
                );
                w.u32(
                    route + 4,
                    (first_province + (index + 3) % shape.provinces) as u32,
                );
                w.array(army + cunit::ORDINARY_ROUTE_DATA, route, 2);
            }
            if index < shape.transports {
                w.u64(army + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
                w.u64(army + cunit::LOCATION, provinces[0] as u64);
            }
            divisions.push((army, handle));
        }
        Built {
            world: w,
            divisions,
            intel: intel_tables,
            regional: regional_data,
            enemies: enemy_lists,
            subunits,
            table,
        }
    }

    #[derive(Debug, PartialEq)]
    struct Captured {
        stats: inputs::Row,
        context: inputs::ContextRow,
        widths: Vec<inputs::WidthTerm>,
        reinforcement: reinforcement::ContextRow,
        modifiers: Vec<reinforcement::ModifierTerm>,
        supply: Vec<reinforcement::SupplyTerm>,
        acclimatization: Vec<reinforcement::AcclimatizationTerm>,
        transport: Vec<reinforcement::TransportTerm>,
    }

    type Walked = Vec<Result<Captured, &'static str>>;

    /// Divisions per image gather in the test walk: a combat's.
    const IMAGE_BATCH: usize = 20;

    /// Per-division scratch, as the collector's.
    struct Scratch {
        widths: [inputs::WidthTerm; inputs::WIDTH_TERM_LIMIT],
        modifiers: [reinforcement::ModifierTerm; reinforcement::MODIFIER_LIMIT],
        supply: [reinforcement::SupplyTerm; reinforcement::SUPPLY_TERM_LIMIT],
        acclimatization: [reinforcement::AcclimatizationTerm; reinforcement::ACCLIMATIZATION_LIMIT],
        transport: Vec<reinforcement::TransportTerm>,
        groups: crate::cname_group_member::GroupKeys,
        names: Vec<u8>,
    }

    impl Scratch {
        fn new() -> Self {
            Self {
                widths: [Default::default(); inputs::WIDTH_TERM_LIMIT],
                modifiers: [Default::default(); reinforcement::MODIFIER_LIMIT],
                supply: [Default::default(); reinforcement::SUPPLY_TERM_LIMIT],
                acclimatization: [Default::default(); reinforcement::ACCLIMATIZATION_LIMIT],
                transport: Vec::new(),
                groups: crate::cname_group_member::GroupKeys::with_capacity(64, 64),
                names: vec![0; 4096],
            }
        }
        fn captured(
            &self,
            stats: inputs::Row,
            context: inputs::ContextRow,
            row: reinforcement::ContextRow,
        ) -> Captured {
            Captured {
                stats,
                widths: self.widths[..context.width_term_count].to_vec(),
                context,
                modifiers: self.modifiers[..row.modifier_count].to_vec(),
                supply: self.supply[..row.supply_term_count].to_vec(),
                acclimatization: self.acclimatization[..row.acclimatization_count].to_vec(),
                transport: self.transport[..row.transport_term_count].to_vec(),
                reinforcement: row,
            }
        }
    }

    /// The unshared per-division chain, as the collector ran it before
    /// sharing: those readers, one country reader for the combat and the
    /// per-pass context memo, every division read in full.
    fn walk_main(memory: &World, divisions: &[(usize, u64)]) -> Walked {
        use super::{main_combat as combat, main_reinforcement as reinforcement};
        let mut memo = combat::ContextModifiers::default();
        let mut s = Scratch::new();
        let countries = crate::ccountry::Reader::new(memory, 0, GAME);
        divisions
            .iter()
            .map(|&(army, handle)| {
                let stats = combat::capture_stats(
                    memory,
                    0,
                    army,
                    handle,
                    &mut s.groups,
                    &mut crate::cname_group_member::Names {
                        bytes: &mut s.names,
                        len: 0,
                        required: 0,
                    },
                )?;
                let countries = countries.as_ref().map_err(|reason| *reason)?;
                let context = combat::capture_context(
                    memory,
                    0,
                    countries,
                    army,
                    handle,
                    &mut s.widths,
                    &mut memo,
                )?;
                let row = reinforcement::capture_context(
                    memory,
                    0,
                    GAME,
                    army,
                    handle,
                    &mut s.modifiers,
                    &mut s.supply,
                    &mut s.acclimatization,
                    &mut s.transport,
                    0,
                )?;
                Ok(s.captured(stats, context, row))
            })
            .collect()
    }

    /// The collector's per-division chain over every division, in one pass,
    /// through each division's image.
    fn walk_shared(memory: &World, sources: &mut Sources, divisions: &[(usize, u64)]) -> Walked {
        sources.clear();
        let mut s = Scratch::new();
        let countries = crate::ccountry::Reader::new(memory, 0, GAME);
        let mut images = Images::default();
        divisions
            .iter()
            .enumerate()
            .map(|(index, &(army, handle))| {
                // Images gathered twenty divisions at a time, as a combat's.
                let row = index % IMAGE_BATCH;
                if row == 0 {
                    let armies: Vec<usize> = divisions[index..]
                        .iter()
                        .take(IMAGE_BATCH)
                        .map(|division| division.0)
                        .collect();
                    images.capture(memory, &armies);
                }
                let image = images.division(memory, row, army);
                let memory = &image;
                let stats = inputs::capture_stats(
                    memory,
                    0,
                    sources,
                    army,
                    handle,
                    &mut s.groups,
                    &mut crate::cname_group_member::Names {
                        bytes: &mut s.names,
                        len: 0,
                        required: 0,
                    },
                )?;
                let countries = countries.as_ref().map_err(|reason| *reason)?;
                let context = inputs::capture_context(
                    memory,
                    0,
                    countries,
                    sources,
                    army,
                    handle,
                    &mut s.widths,
                )?;
                let row = reinforcement::capture_context(
                    memory,
                    0,
                    GAME,
                    sources,
                    army,
                    handle,
                    &mut s.modifiers,
                    &mut s.supply,
                    &mut s.acclimatization,
                    &mut s.transport,
                    0,
                )?;
                Ok(s.captured(stats, context, row))
            })
            .collect()
    }

    type Row = Result<reinforcement::ContextRow, &'static str>;

    /// The reinforcement reader alone, with nothing authenticated before it:
    /// main's, then the shared one over one pass.
    fn reinforcement_alone(memory: &World, divisions: &[(usize, u64)]) -> [Vec<Row>; 2] {
        let mut s = Scratch::new();
        let main = divisions
            .iter()
            .map(|&(army, handle)| {
                super::main_reinforcement::capture_context(
                    memory,
                    0,
                    GAME,
                    army,
                    handle,
                    &mut s.modifiers,
                    &mut s.supply,
                    &mut s.acclimatization,
                    &mut s.transport,
                    0,
                )
            })
            .collect();
        let mut sources = Sources::new();
        let shared = divisions
            .iter()
            .map(|&(army, handle)| {
                reinforcement::capture_context(
                    memory,
                    0,
                    GAME,
                    &mut sources,
                    army,
                    handle,
                    &mut s.modifiers,
                    &mut s.supply,
                    &mut s.acclimatization,
                    &mut s.transport,
                    0,
                )
            })
            .collect();
        [main, shared]
    }

    /// Main's walk and the shared walk over the same memory, with the calls
    /// each made.
    fn both(built: &Built) -> ((Walked, usize), (Walked, usize)) {
        let w = &built.world;
        w.copies.set(0);
        let main = walk_main(w, &built.divisions);
        let main_calls = w.copies.replace(0);
        let shared = walk_shared(w, &mut Sources::new(), &built.divisions);
        ((main, main_calls), (shared, w.copies.replace(0)))
    }

    fn small() -> Shape {
        Shape {
            countries: 60,
            divisions: 240,
            owners: 9,
            templates: 17,
            states: 11,
            provinces: 23,
            relations: 40,
            transports: 5,
            leaders: 4,
        }
    }

    /// Sharing changes no output: every division's stats, naming, context,
    /// width, reinforcement, modifier, supply, acclimatization and transport
    /// terms equal the unshared walk's, over divisions sharing owners, templates,
    /// states, provinces, regions and leaders, with routes, air entries,
    /// relation walks, weather, core supply and transport divisions; with and
    /// without the authenticated roster. And it reads far less.
    #[test]
    fn shared_sources_change_no_output() {
        for authenticated in [true, false] {
            let mut built = world(&small());
            built.world.authenticated = authenticated;
            let ((main, main_calls), (shared, shared_calls)) = both(&built);
            assert!(
                main.iter().all(Result::is_ok),
                "main's walk failed: {:?}",
                main.iter().find(|r| r.is_err())
            );
            assert_eq!(main, shared);
            let rows: Vec<_> = main.iter().map(|r| r.as_ref().unwrap()).collect();
            let r = |row: &&Captured| row.reinforcement;
            // The fixture reaches each shared branch.
            assert!(
                rows.iter()
                    .map(r)
                    .any(|r| r.flags[7] == 1 && r.scalar_values[15] != 0)
            );
            assert!(rows.iter().map(r).any(|r| r.flags[11] == 2));
            assert!(rows.iter().any(|row| !row.transport.is_empty()));
            assert!(rows.iter().map(r).any(|r| r.flags[8] == 1));
            assert!(
                rows.iter()
                    .map(r)
                    .any(|r| r.flags[3] == 0 && r.flags[8] == 0)
            );
            assert!(rows.iter().map(r).any(|r| r.flags[6] == 1 && r.ids[2] != 0));
            assert!(rows.iter().map(r).any(|r| r.ids[1] != 0));
            assert!(rows.iter().any(|row| row.context.leader_modifier_kind == 1));
            assert!(rows.iter().any(|row| {
                row.modifiers
                    .iter()
                    .any(|term| term.scope == 5 && term.present == 1)
            }));
            assert!(rows.iter().any(|row| {
                row.modifiers
                    .iter()
                    .any(|term| term.scope == 6 && term.present == 1)
            }));
            assert!(rows.iter().any(|row| !row.acclimatization.is_empty()));
            assert!(rows.iter().any(|row| row.stats.naming.unwrap().kind == 1));
            assert!(rows.iter().all(|row| !row.widths.is_empty()));
            assert!(rows.iter().any(|row| !row.supply.is_empty()));
            assert!(
                shared_calls * 3 < main_calls,
                "{shared_calls} of {main_calls}"
            );
        }
    }

    /// Every failure is main's: a read that fails at a chosen ordinal of
    /// main's walk fails every division as main's walk fails it, with main's
    /// reason, and the divisions around it read the same rows. Both the
    /// chain and the reinforcement reader alone, with and without the
    /// authenticated roster (whose absence makes country lookups read).
    #[test]
    fn shared_sources_fail_as_main_does() {
        let shape = Shape {
            divisions: 40,
            ..small()
        };
        for authenticated in [true, false] {
            let mut built = world(&shape);
            built.world.authenticated = authenticated;
            *built.world.log.borrow_mut() = Some(Vec::new());
            walk_main(&built.world, &built.divisions);
            let _ = reinforcement_alone(&built.world, &built.divisions);
            let log = built.world.log.borrow_mut().take().unwrap();
            // Each distinct span once, in main's first-read order.
            let mut seen = std::collections::HashSet::new();
            let spans: Vec<_> = log.into_iter().filter(|span| seen.insert(*span)).collect();
            let step = (spans.len() / 200).max(1);
            let mut failures = 0;
            for &span in spans.iter().step_by(step) {
                *built.world.poisoned.borrow_mut() = vec![span];
                let ((main, _), (shared, _)) = both(&built);
                assert_eq!(main, shared, "{span:x?}, roster {authenticated}");
                let [main, shared] = reinforcement_alone(&built.world, &built.divisions);
                assert_eq!(main, shared, "{span:x?} alone, roster {authenticated}");
                failures += usize::from(main.iter().any(Result::is_err));
            }
            assert!(failures > 0);
        }
    }

    /// Failures the shared walk would reorder if it read a source's stages
    /// together: each keeps main's order.
    #[test]
    fn staged_sources_keep_main_failure_order() {
        type Faults = fn(&Built) -> Vec<(usize, usize)>;
        let cases: [(&str, Faults); 3] = [
            // An owner's first enemy's regional operand and the next enemy's
            // intel: main meets the regional read first.
            ("regional before later intel", |built| {
                let owner = (0..built.enemies.len())
                    .find(|&owner| built.enemies[owner].len() >= 2)
                    .unwrap();
                let [first, second] = [built.enemies[owner][0], built.enemies[owner][1]];
                vec![
                    (built.regional[first], 16),
                    (built.intel[second] + crate::ccountry_intel::OWNER, 4),
                ]
            }),
            // One enemy's intel lane and its own regional operand: main reads
            // the regional operand first.
            ("regional before own intel", |built| {
                let owner = (0..built.enemies.len())
                    .find(|&owner| !built.enemies[owner].is_empty())
                    .unwrap();
                let enemy = built.enemies[owner][0];
                vec![
                    (built.regional[enemy], 16),
                    (built.intel[enemy] + crate::ccountry_intel::OWNER, 4),
                ]
            }),
            // The priority token's definition and the category header: main
            // appends the token's modifier first.
            ("token before categories", |built| {
                vec![
                    (
                        built.table
                            + TOKEN as usize * modifier_definition::STRIDE
                            + modifier_definition::FLAGS,
                        12,
                    ),
                    (built.subunits[0] + 0x508, 16),
                ]
            }),
        ];
        for (label, faults) in cases {
            let built = world(&small());
            *built.world.poisoned.borrow_mut() = faults(&built);
            let ((main, _), (shared, _)) = both(&built);
            assert!(main.iter().any(Result::is_err), "{label}");
            assert_eq!(main, shared, "{label}");
            let [main, shared] = reinforcement_alone(&built.world, &built.divisions);
            assert_eq!(main, shared, "{label} alone");
        }
        // With both reads failing, the reasons differ: the category header
        // is invalid rather than unreadable.
        let mut built = world(&small());
        let unit = built.subunits[0];
        let data = built.world.get_u64(unit + 0x508);
        built.world.u64(unit + 0x508, data);
        built.world.u32(unit + 0x508 + 8, 1);
        built.world.u32(unit + 0x508 + 12, 2);
        let metadata =
            built.table + TOKEN as usize * modifier_definition::STRIDE + modifier_definition::FLAGS;
        let first = |walked: &Walked| walked.iter().find_map(|r| r.as_ref().err().copied());
        let ((header, _), (shared, _)) = both(&built);
        assert_eq!(header, shared);
        assert_eq!(first(&header), Some("reinforcement array invalid"));
        *built.world.poisoned.borrow_mut() = vec![(metadata, 12)];
        let ((both_fail, _), (shared, _)) = both(&built);
        assert_eq!(both_fail, shared);
        assert_eq!(first(&both_fail), Some("reinforcement input unreadable"));
    }

    /// The image layouts hold every army and stat block field the division
    /// readers read on this fixture (leaders, routes, acclimatization,
    /// transports, manpower): every read of a division's army or stat block
    /// lies inside a gathered span. (A scatter that also reads a module
    /// define goes to the source whole, as one call either way.)
    #[test]
    fn images_serve_every_division_field() {
        let mut built = world(&small());
        built.world.authenticated = true;
        let armies: Vec<usize> = built.divisions.iter().map(|division| division.0).collect();
        let stats: Vec<usize> = armies
            .iter()
            .map(|&army| built.world.get_u64(army + cunit::STAT_BLOCK) as usize)
            .collect();
        *built.world.log.borrow_mut() = Some(Vec::new());
        let walked = walk_shared(&built.world, &mut Sources::new(), &built.divisions);
        assert!(walked.iter().all(Result::is_ok));
        let log = built.world.log.borrow_mut().take().unwrap();
        let gathered = |base: usize, fields: &[RecordField], address: usize, len: usize| {
            fields.iter().any(|field| {
                field.len != 0
                    && base + field.source_offset <= address
                    && address + len <= base + field.source_offset + field.len
            })
        };
        let misses: Vec<_> = log
            .iter()
            .filter_map(|&(address, len)| {
                let army = armies
                    .iter()
                    .find(|&&army| (army..army + 0x800).contains(&address));
                let stat = stats
                    .iter()
                    .find(|&&stat| (stat..stat + 0x600).contains(&address));
                match (army, stat) {
                    (Some(&army), _) if !gathered(army, &ARMY_FIELDS, address, len) => {
                        Some(("army", address - army, len))
                    }
                    (_, Some(&stat)) if !gathered(stat, &STAT_FIELDS, address, len) => {
                        Some(("stat", address - stat, len))
                    }
                    _ => None,
                }
            })
            .collect();
        assert!(misses.is_empty(), "{misses:x?}");
    }

    /// A record that does not copy completely is served nothing: the gather
    /// resumes after it (one more call per failed record, nothing reread),
    /// and that division's reads go to the source, as do the stat block reads
    /// of a division whose army record failed.
    #[test]
    fn images_resume_after_a_failed_record() {
        let built = world(&small());
        let w = &built.world;
        let armies: Vec<usize> = built.divisions.iter().take(6).map(|d| d.0).collect();
        let stats: Vec<usize> = armies
            .iter()
            .map(|&army| w.get_u64(army + cunit::STAT_BLOCK) as usize)
            .collect();
        // Division 2's army and division 4's stat block fail inside a span.
        w.poisoned.borrow_mut().extend([
            (armies[2] + carmy::STRENGTH_FACTOR, 8),
            (stats[4] + 0x298, 8),
        ]);
        let mut images = Images::default();
        w.copies.set(0);
        *w.log.borrow_mut() = Some(Vec::new());
        images.capture(w, &armies);
        // Armies: rows 0-1, then 3-5. Stat blocks: rows 0-1, then 3, then 5.
        assert_eq!(w.copies.get(), 5);
        let log = w.log.borrow_mut().take().unwrap();
        let mut seen = std::collections::HashSet::new();
        assert!(log.iter().all(|span| seen.insert(*span)), "a span reread");
        let read = |row: usize, address: usize| {
            let image = images.division(w, row, armies[row]);
            let before = w.copies.get();
            let mut bytes = [0; 8];
            let copied = image.copy(address, &mut bytes);
            (copied, w.copies.get() - before)
        };
        for row in 0..6 {
            let army = read(row, armies[row] + cunit::HANDLE);
            let stat = read(row, stats[row] + 0x290);
            assert_eq!(army, (true, usize::from(row == 2)), "row {row}");
            assert_eq!(stat, (true, usize::from(row == 2 || row == 4)), "row {row}");
        }
        // A read outside every span, and a division the row does not hold,
        // go to the source.
        assert_eq!(read(0, armies[0] + 0x700), (true, 1));
        let other = images.division(w, 1, armies[0]);
        let before = w.copies.get();
        assert!(other.copy(armies[0] + cunit::HANDLE, &mut [0; 8]));
        assert_eq!(w.copies.get() - before, 1);
    }

    /// The batched owned-relation walk answers as the one-by-one walk does,
    /// with its failure: a war at any position, none, a null relation, an
    /// unreadable pointer and an unreadable type. No span is read twice.
    #[test]
    fn batched_relation_walk_matches_the_sequential_walk() {
        #[derive(Clone, Copy, Debug)]
        enum Fault {
            Null,
            Pointer,
            Kind,
        }
        let sequential = |w: &World, relations: Array| -> Result<bool, &'static str> {
            for i in 0..relations.count {
                let at = relations.element(i, 8).unwrap();
                let relation = read::<u64>(w, at).ok_or("reinforcement input unreadable")? as usize;
                let kind = relation
                    .checked_add(8)
                    .and_then(|at| read::<u32>(w, at))
                    .ok_or("reinforcement input unreadable")?;
                if kind == 0x380a {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let mut cases = vec![
            (0, None, None),
            (1, Some(0), None),
            (63, None, None),
            (64, Some(63), None),
            (65, Some(64), None),
            (200, Some(130), None),
        ];
        for fault in [Fault::Null, Fault::Pointer, Fault::Kind] {
            for (war, at) in [(None, 77), (Some(90), 150), (Some(150), 90), (Some(64), 64)] {
                cases.push((200, war, Some((fault, at))));
            }
            cases.push((64, None, Some((fault, 63))));
            cases.push((65, Some(64), Some((fault, 0))));
        }
        for (count, war, fault) in cases {
            let mut w = World::default();
            let peace = w.alloc(0x20);
            w.u32(peace + 8, 0x3801);
            let at_war = w.alloc(0x20);
            w.u32(at_war + 8, 0x380a);
            let list = w.alloc(count.max(1) * 8);
            let mut relations = Vec::new();
            for index in 0..count {
                let relation = match (war, fault) {
                    (_, Some((Fault::Null, n))) if n == index => 0,
                    (Some(n), _) if n == index => at_war,
                    _ => {
                        let own = w.alloc(0x20);
                        w.u32(own + 8, 0x3801);
                        own
                    }
                };
                relations.push(relation);
                w.u64(list + index * 8, relation as u64);
            }
            match fault {
                Some((Fault::Pointer, n)) => w.poisoned.borrow_mut().push((list + n * 8, 8)),
                Some((Fault::Kind, n)) => w.poisoned.borrow_mut().push((relations[n] + 8, 4)),
                _ => {}
            }
            let header = w.alloc(16);
            w.array(header, list, count as u32);
            let relations = Array::read(&w, header, 8).unwrap();
            let expected = sequential(&w, relations);
            *w.log.borrow_mut() = Some(Vec::new());
            let batched = reinforcement::owned_war_relation(&w, relations);
            assert_eq!(
                batched, expected,
                "{count} relations, war {war:?}, fault {fault:?}"
            );
            if fault.is_some() && war.is_none() {
                assert!(expected.is_err());
            }
            let log = w.log.borrow_mut().take().unwrap();
            let mut seen = std::collections::HashSet::new();
            assert!(log.iter().all(|span| seen.insert(*span)), "a span reread");
        }
    }

    /// Guarded calls and CPU time per pass, main's walk and the shared walk,
    /// at 153, 1,000, 3,000 and 5,000 divisions, then the world-war relation
    /// walk at 1,500 and 5,000 countries. The time is this fixture's
    /// in-process copy, not the game's guarded read. Run in release with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn shared_sources_cost_at_scale() {
        let case = |label: &str, shape: Shape| {
            let mut built = world(&shape);
            built.world.authenticated = true;
            let started = std::time::Instant::now();
            let ((main, main_reads), _) = both(&built);
            let main_ms = started.elapsed().as_secs_f64() * 1e3;
            let mut sources = Sources::new();
            // A first pass warms storage, as a running recorder's has been.
            let _ = walk_shared(&built.world, &mut sources, &built.divisions);
            built.world.copies.set(0);
            let started = std::time::Instant::now();
            let shared = walk_shared(&built.world, &mut sources, &built.divisions);
            let shared_ms = started.elapsed().as_secs_f64() * 1e3;
            let shared_reads = built.world.copies.get();
            assert!(
                main.iter().all(Result::is_ok),
                "{label}: {:?}",
                main.iter().find(|r| r.is_err())
            );
            assert_eq!(main, shared);
            println!(
                "{label}: {} divisions, {} countries, {} relations each | main {main_reads} reads ({:.0}/division) {main_ms:.2} ms | shared {shared_reads} reads ({:.0}/division) {shared_ms:.2} ms",
                shape.divisions,
                shape.countries,
                shape.relations,
                main_reads as f64 / shape.divisions as f64,
                shared_reads as f64 / shape.divisions as f64,
            );
        };
        for divisions in [153, 1_000, 3_000, 5_000] {
            case(
                "scale",
                Shape {
                    countries: 1_500,
                    divisions,
                    owners: (divisions / 25).clamp(6, 150),
                    templates: (divisions / 6).max(20),
                    states: (divisions / 8).max(10),
                    provinces: (divisions / 5).max(20),
                    relations: 20,
                    transports: divisions / 50,
                    leaders: (divisions / 20).clamp(1, 48),
                },
            );
        }
        for countries in [1_500, 5_000] {
            case(
                "world war",
                Shape {
                    countries,
                    divisions: 3_000,
                    owners: 120,
                    templates: 500,
                    states: 375,
                    provinces: 600,
                    relations: countries as usize,
                    transports: 60,
                    leaders: 48,
                },
            );
        }
    }
}
