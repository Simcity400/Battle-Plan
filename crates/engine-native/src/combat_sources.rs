
use crate::{
    Memory,
    memory::{Raw, RecordField, RecordLayout, RecordSources},
    read,
};
use std::{
    hash::Hash,
    io,
};

#[cfg(test)]
mod main_combat;
#[cfg(test)]
mod main_reinforcement;

pub use engine_native_parts::keyed_map::{KeyHasher, Map};
pub(crate) type Shared<K, V> = Map<K, Result<V, &'static str>>;
type LeaderChain = Result<Option<usize>, &'static str>;

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

#[derive(Default)]
pub struct Sources {
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
    pub(crate) censuses: Shared<usize, Run>,
    pub(crate) census_entries: Vec<(u32, i64)>,
    pub(crate) transport_owners: Shared<usize, crate::land_reinforcement_inputs::TransportOwner>,
    pub(crate) transport_enemies: Vec<crate::land_reinforcement_inputs::TransportEnemy>,
    pub(crate) regional: Shared<(usize, u32), i64>,
    pub(crate) widths: Shared<usize, crate::division_combat_inputs::TemplateWidth>,
    pub(crate) width_arena: Vec<crate::division_combat_inputs::WidthTerm>,
    pub(crate) context_metadata:
        Option<Result<crate::division_combat_inputs::ContextMetadata, &'static str>>,
    pub(crate) context_modifiers: Shared<usize, [crate::division_combat_inputs::ContextModifier; 3]>,
    pub(crate) verified: Option<(usize, u64, usize)>,
    leader: Option<(usize, u64, LeaderChain)>,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub(crate) fn field<T: Raw>(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        offset: usize,
        reason: &'static str,
    ) -> Result<T, &'static str> {
        const { assert!(size_of::<T>() <= 8) };
        let address = pointer.checked_add(offset).ok_or(reason)?;
        let key = (address, size_of::<T>());
        if self.fields.len() == self.fields.capacity() && !self.fields.contains_key(&key) {
            crate::allocation::reserve_map(&mut self.fields, 1).map_err(|_| "allocation failed")?;
        }
        let bytes = *self.fields.entry(key).or_insert_with(|| {
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

pub(crate) fn share<K: Hash + Eq, V>(
    map: &mut Shared<K, V>,
    key: K,
    compute: impl FnOnce() -> Result<V, &'static str>,
) -> Result<&V, &'static str> {
    if map.len() == map.capacity() && !map.contains_key(&key) {
        crate::allocation::reserve_map(map, 1).map_err(|_| "allocation failed")?;
    }
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
    span(0x4a8, 0x4ac, 0x104),
    span(0x4c0, 0x4f8, 0x108),
    span(0x600, 0x628, 0x140),
];
const ARMY_STRIDE: usize = 0x168;
const STAT_FIELDS: [RecordField; 6] = [
    span(0x000, 0x008, 0x000),
    span(0x0a8, 0x0e8, 0x008),
    span(0x120, 0x128, 0x048),
    span(0x220, 0x228, 0x050),
    span(0x290, 0x2a8, 0x058),
    span(0x5a8, 0x5b0, 0x070),
];
const STAT_STRIDE: usize = 0x78;
const ARMY_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ARMY_FIELDS, ARMY_STRIDE).unwrap();
const STAT_LAYOUT: RecordLayout<'static> = RecordLayout::new(&STAT_FIELDS, STAT_STRIDE).unwrap();
const STAT_POINTER: usize = 0x060;
const _: () = assert!(
    ARMY_FIELDS[3].source_offset == crate::cunit::STAT_BLOCK
        && ARMY_FIELDS[3].destination_offset == STAT_POINTER
);

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
    pub fn clear(&mut self) {
        self.armies.clear();
    }

    pub fn capture(&mut self, memory: &impl Memory, armies: &[usize]) -> io::Result<()> {
        let count = armies.len();
        self.armies.clear();
        self.stats.clear();
        crate::allocation::reserve(&mut self.armies, count)?;
        crate::allocation::reserve_push_batch(&mut self.stats, count)?;
        crate::allocation::resize(
            &mut self.army_bytes,
            count
                .checked_mul(ARMY_STRIDE)
                .ok_or_else(crate::allocation::oom)?,
            0,
        )?;
        crate::allocation::resize(&mut self.army_complete, count, false)?;
        crate::allocation::resize(
            &mut self.stat_bytes,
            count
                .checked_mul(STAT_STRIDE)
                .ok_or_else(crate::allocation::oom)?,
            0,
        )?;
        crate::allocation::resize(&mut self.stat_complete, count, false)?;
        self.armies.extend_from_slice(armies);
        gather(
            memory,
            &self.armies,
            &ARMY_LAYOUT,
            &mut self.army_bytes,
            &mut self.army_complete,
        );
        for row in 0..count {
            let at = row * ARMY_STRIDE + STAT_POINTER;
            self.stats.push(if self.army_complete[row] {
                u64::from_le_bytes(self.army_bytes[at..at + 8].try_into().unwrap()) as usize
            } else {
                0
            });
        }
        gather(
            memory,
            &self.stats,
            &STAT_LAYOUT,
            &mut self.stat_bytes,
            &mut self.stat_complete,
        );
        Ok(())
    }

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
        row += copied + usize::from(row + copied < end);
    }
}

pub struct DivisionImage<'a, M> {
    inner: &'a M,
    army: usize,
    army_row: Option<&'a [u8]>,
    stat: usize,
    stat_row: Option<&'a [u8]>,
}

impl<'a, M: Memory> DivisionImage<'a, M> {
    fn served(&self, address: usize, len: usize) -> Option<&'a [u8]> {
        let within = |base: usize, row: Option<&'a [u8]>, fields: &[RecordField]| {
            let row = row?;
            let offset = address.checked_sub(base)?;
            let end = offset.checked_add(len)?;
            let index = fields
                .partition_point(|field| field.source_offset <= offset)
                .checked_sub(1)?;
            let field = &fields[index];
            if end > field.source_offset + field.len {
                return None;
            }
            let start = field.destination_offset + offset - field.source_offset;
            Some(&row[start..start + len])
        };
        if address == 0 {
            return None;
        }
        within(self.army, self.army_row, &ARMY_FIELDS)
            .or_else(|| within(self.stat, self.stat_row, &STAT_FIELDS))
    }

    fn copy_served<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> bool {
        let mut bytes: [&[u8]; N] = [&[]; N];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let Some(observed) = self.served(addresses[index], destinations[index].len()) else {
                return false;
            };
            *slot = observed;
        }
        for (destination, bytes) in destinations.iter_mut().zip(bytes) {
            destination.copy_from_slice(bytes);
        }
        true
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

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        self.inner.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::ReservedCountryObservation> {
        self.inner
            .authenticated_reserved_country_observation(base, game_state)
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
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if self.copy_served(addresses, destinations) {
            return N;
        }
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if self.copy_served(addresses, destinations) {
            return N;
        }
        self.inner
            .copy_scattered_prefetched(addresses, destinations)
    }
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
mod tests;
