#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use crate::{
    ccountry,
    crelation::{self, status as crelation_status, subject as csubject_relation},
    cwar_relation::goals as cwar_goal,
};
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnedRow {
    pub source: Source,
    pub owner: u32,
    pub ordinal: u32,
    pub first_country: u32,
    pub second_country: u32,
    pub retired: u8,
    pub first_goal_count: u32,
    pub second_goal_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub relation: usize,
    pub diplomacy: usize,
    pub owner_slot: u32,
    pub first_slot: u32,
    pub second_slot: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryRow {
    pub country: u32,
    pub major: u8,
    pub divisions: i32,
    pub factories: i32,
    pub view_subject: u8,
    pub score_available: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub owned: usize,
    pub goals: usize,
    pub relations: usize,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("war overview field unreadable or address overflow")
}
fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, stride))
        .ok_or("war overview array invalid or unreadable")
}
fn vtable(memory: &impl Memory, base: usize, pointer: usize, rva: Rva) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, rva).map(|p| p as u64);
    if pointer == 0 || expected.is_none() || read::<u64>(memory, pointer) != expected {
        Err("war overview vtable mismatch")
    } else {
        Ok(())
    }
}
fn element(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|p| read::<u64>(memory, p))
        .map(|p| p as usize)
        .ok_or("war overview pointer unreadable")
}
fn diplomacy(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let pointer = field::<u64>(memory, country.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
    vtable(memory, base, pointer, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
    if field::<u64>(memory, pointer, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != country.pointer as u64 {
        return Err("war overview diplomacy owner mismatch");
    }
    Ok(pointer)
}

pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Result<Demand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = Demand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let owned = array(
            memory,
            diplomacy(memory, base, country)?,
            engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
            8,
        )?;
        demand.relations = demand
            .relations
            .checked_add(owned.count as usize)
            .ok_or("war discovery demand overflow")?;
        for index in 0..owned.count {
            let relation = element(memory, owned, index)?;
            if field::<u32>(memory, relation, crelation::TYPE_ID)? != super::TYPE {
                continue;
            }
            vtable(memory, base, relation, super::VTABLE_RVA)?;
            demand.owned = demand
                .owned
                .checked_add(1)
                .ok_or("war owned demand overflow")?;
            for offset in super::GOALS {
                demand.goals = demand
                    .goals
                    .checked_add(array(memory, relation, offset, 8)?.count as usize)
                    .ok_or("war goal demand overflow")?;
            }
        }
    }
    Ok(demand)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct OwnerHeader {
    generation: u64,
    raw: u32,
    country: usize,
    diplomacy: usize,
    owned: Array,
    start: usize,
    owned_start: usize,
    owned_count: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountrySource {
    pub country: u32,
    pub slot: u32,
    pub diplomacy: usize,
}
#[derive(Clone, Copy, Debug)]
struct Visit {
    country: engine_native_memory::country_identity::Country,
    diplomacy: usize,
    has_war: bool,
}

#[derive(Debug, Default)]
struct Held {
    visits: Vec<Visit>,
    owned: usize,
    goals: Vec<cwar_goal::relation::Row>,
    countries: Vec<u8>,
}

pub struct DiscoveryCache {
    generation: u64,
    headers: Vec<OwnerHeader>,
    pointers: Vec<usize>,
    types: Vec<u8>,
    rows: Vec<OwnedRow>,
    next_rows: Vec<OwnedRow>,
    reachable: Vec<bool>,
    enemies: Vec<bool>,
    required: usize,
    scope: Option<(usize, usize)>,
    viewpoint: Option<Result<u32, &'static str>>,
    held: Option<Held>,
    next: Held,
    carried: bool,
}
impl DiscoveryCache {
    pub fn new(countries: usize, relations: usize) -> Self {
        Self::try_new(countries, relations).unwrap_or_else(|error| {
            let _ = engine_allocation::owned_vec::<u8>(Err(error));
            Self::try_new(0, 0).unwrap()
        })
    }
    pub fn try_new(countries: usize, relations: usize) -> std::io::Result<Self> {
        use engine_allocation as allocation;
        let types = allocation::mul(relations, 4)?;
        Ok(Self {
            generation: 0,
            headers: allocation::filled(countries, OwnerHeader::default())?,
            pointers: allocation::filled(relations, 0)?,
            types: allocation::filled(types, 0)?,
            rows: allocation::filled(relations, OwnedRow::default())?,
            next_rows: allocation::filled(relations, OwnedRow::default())?,
            reachable: allocation::filled(countries, false)?,
            enemies: allocation::filled(countries, false)?,
            required: 0,
            scope: None,
            viewpoint: None,
            held: None,
            next: Held::default(),
            carried: false,
        })
    }
    pub fn country_sources(&self) -> impl Iterator<Item = CountrySource> + '_ {
        self.headers
            .iter()
            .enumerate()
            .filter(|(_, h)| h.country != 0 && h.generation == self.generation)
            .map(|(slot, h)| CountrySource {
                country: h.raw,
                slot: slot as u32,
                diplomacy: h.diplomacy,
            })
    }
    pub fn viewpoint(&self) -> Result<u32, &'static str> {
        self.viewpoint
            .unwrap_or(Err("war overview viewpoint unavailable"))
    }
    pub fn invalidate(&mut self) {
        self.viewpoint = None;
        self.held = None;
        self.headers.fill(OwnerHeader::default());
    }
    pub fn held_rows(&self) -> Option<&[OwnedRow]> {
        self.held.as_ref().map(|held| &self.rows[..held.owned])
    }
    pub fn carried(&self) -> bool {
        self.carried
    }
    pub fn grow(&mut self, countries: usize, relations: usize) {
        let _ = self.try_grow(countries, relations);
    }
    pub fn try_grow(&mut self, countries: usize, relations: usize) -> std::io::Result<()> {
        use engine_allocation as allocation;
        let countries = countries.max(self.headers.len());
        let relations = relations.max(self.required).max(self.pointers.len());
        let types = allocation::mul(relations, 4)?;
        allocation::resize(&mut self.headers, countries, OwnerHeader::default())?;
        allocation::resize(&mut self.reachable, countries, false)?;
        allocation::resize(&mut self.enemies, countries, false)?;
        allocation::resize(&mut self.pointers, relations, 0)?;
        allocation::resize(&mut self.types, types, 0)?;
        allocation::resize(&mut self.rows, relations, OwnedRow::default())?;
        allocation::resize(&mut self.next_rows, relations, OwnedRow::default())?;
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.headers.capacity() * std::mem::size_of::<OwnerHeader>()
            + self.pointers.capacity() * std::mem::size_of::<usize>()
            + self.types.capacity()
            + (self.rows.capacity() + self.next_rows.capacity()) * std::mem::size_of::<OwnedRow>()
            + self.reachable.capacity()
            + self.enemies.capacity()
            + [self.held.as_ref(), Some(&self.next)]
                .into_iter()
                .flatten()
                .map(|held| {
                    held.visits.capacity() * std::mem::size_of::<Visit>()
                        + held.goals.capacity() * std::mem::size_of::<cwar_goal::relation::Row>()
                        + held.countries.capacity()
                })
                .sum::<usize>()
    }
}

fn gather<const F: usize, const B: usize>(
    memory: &impl Memory,
    pointers: &[usize],
    spans: [(usize, usize); F],
    bytes: &mut [u8; B],
) -> Result<(), &'static str> {
    if pointers.is_empty() {
        return Ok(());
    }
    use engine_native_memory::memory::{
        RecordLayout,
        RecordSources,
        packed_fields,
        packed_stride,
    };
    let fields = packed_fields(spans);
    let layout = RecordLayout::new(&fields, packed_stride(spans)).unwrap();
    if memory.copy_records(RecordSources::Pointers(pointers), &layout, bytes) != pointers.len() * F
    {
        return Err("war overview record gather unreadable or address overflow");
    }
    Ok(())
}
fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}
fn pointer(bytes: &[u8]) -> usize {
    u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize
}

#[inline(never)]
fn gather_owned_types(
    memory: &impl Memory,
    owned_pages: &[u8],
    vectors: &[Array],
    type_starts: &mut [usize],
    page_types: &mut [u8; 64 * 64 * 4],
) -> Result<(), &'static str> {
    let mut type_pointers = [0; 64 * 64];
    let mut type_count = 0;
    for (index, vector) in vectors.iter().enumerate() {
        type_starts[index] = type_count;
        for ordinal in 0..vector.count.min(64) as usize {
            type_pointers[type_count] = pointer(&owned_pages[index * 512 + ordinal * 8..]);
            type_count += 1;
        }
    }
    gather(
        memory,
        &type_pointers[..type_count],
        [(crelation::TYPE_ID, 4)],
        page_types,
    )
}

#[derive(Clone, Copy, Default)]
struct Occurrence {
    diplomacy: usize,
    owner: u32,
    ordinal: u32,
    pointer: usize,
}

#[derive(Clone, Copy, Default)]
struct GoalPage {
    occurrence: Occurrence,
    side: u8,
    first: u32,
    count: usize,
    address: usize,
}

fn copy_goal_pages(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    pages: &[GoalPage],
    goals: &mut [cwar_goal::relation::Row],
    used: &mut usize,
) -> Result<(), &'static str> {
    if pages.is_empty() {
        return Ok(());
    }
    let mut bytes = [0; 64 * 512];
    let mut chunks = bytes.chunks_exact_mut(512);
    let mut destinations: [&mut [u8]; 64] = std::array::from_fn(|i| {
        &mut chunks.next().unwrap()[..pages.get(i).map_or(0, |p| p.count * 8)]
    });
    let addresses = std::array::from_fn(|i| pages.get(i).map_or(0, |p| p.address));
    if memory.copy_scattered(addresses, &mut destinations) != 64 {
        return Err("war overview goal handle page unreadable");
    }
    for (i, page) in pages.iter().enumerate() {
        for index in 0..page.count {
            goals[*used] = cwar_goal::relation::read_goal(
                memory,
                base,
                countries,
                cwar_goal::relation::Row {
                    owner: page.occurrence.owner,
                    relation_ordinal: page.occurrence.ordinal,
                    side: page.side,
                    ordinal: page.first + index as u32,
                    handle: pointer(&bytes[i * 512 + index * 8..]) as u64,
                    ..Default::default()
                },
            )?;
            *used += 1;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_wars(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    pending: &[Occurrence],
    output: &mut Buffer<'_, OwnedRow>,
    goals: &mut Buffer<'_, cwar_goal::relation::Row>,
    cache: &mut DiscoveryCache,
    used: &mut [usize; 3],
    wanted: &mut [usize; 3],
) -> Result<(), &'static str> {
    let pointers: [usize; 64] = std::array::from_fn(|i| pending.get(i).map_or(0, |r| r.pointer));
    let mut bytes = [0; 64 * 53];
    gather(
        memory,
        &pointers[..pending.len()],
        [
            (0, 8),
            (crelation::TYPE_ID, 4),
            (crelation::FIRST_COUNTRY, 4),
            (crelation::SECOND_COUNTRY, 4),
            (crelation::RETIRED, 1),
            (super::GOALS[0], 16),
            (super::GOALS[1], 16),
        ],
        &mut bytes,
    )?;
    let mut pages = [GoalPage::default(); 64];
    let mut page_len = 0;
    let mut planned = 0;
    for (index, occurrence) in pending.iter().enumerate() {
        let record = &bytes[index * 53..(index + 1) * 53];
        if pointer(record)
            != engine_sites::at(base, super::VTABLE_RVA).ok_or("war vtable withheld")?
            || word(&record[8..]) != super::TYPE
        {
            return Err("war overview vtable mismatch");
        }
        let (a, b) = (word(&record[12..]), word(&record[16..]));
        let (first, second, owner) = (
            countries.resolve(a)?,
            countries.resolve(b)?,
            countries.resolve(occurrence.owner)?,
        );
        if first.canonical_slot == second.canonical_slot
            || (owner.canonical_slot != first.canonical_slot
                && owner.canonical_slot != second.canonical_slot)
        {
            return Err("war overview owned endpoints mismatch");
        }
        let lists = [
            Array::from_header(record[21..37].try_into().unwrap(), 8)
                .ok_or("war overview array invalid or unreadable")?,
            Array::from_header(record[37..53].try_into().unwrap(), 8)
                .ok_or("war overview array invalid or unreadable")?,
        ];
        output.ensure(used[0].checked_add(1).ok_or("war owned demand overflow")?)?;
        if let Some(row) = output.get_mut(used[0]) {
            *row = OwnedRow {
                source: Source {
                    relation: occurrence.pointer,
                    diplomacy: occurrence.diplomacy,
                    owner_slot: owner.canonical_slot,
                    first_slot: first.canonical_slot,
                    second_slot: second.canonical_slot,
                },
                owner: occurrence.owner,
                ordinal: occurrence.ordinal,
                first_country: a,
                second_country: b,
                retired: record[20],
                first_goal_count: lists[0].count,
                second_goal_count: lists[1].count,
            };
            if let Some(next) = cache.next_rows.get_mut(used[0]) {
                *next = *row;
            }
            used[0] += 1;
        }
        for (side, list) in lists.into_iter().enumerate() {
            wanted[1] = wanted[1]
                .checked_add(list.count as usize)
                .ok_or("war goal count overflow")?;
            goals.ensure(wanted[1])?;
            let mut first = 0;
            while first < list.count && used[1] + planned < goals.len() {
                let count = ((list.count - first) as usize)
                    .min(64)
                    .min(goals.len() - used[1] - planned);
                pages[page_len] = GoalPage {
                    occurrence: *occurrence,
                    side: side as u8,
                    first,
                    count,
                    address: list
                        .element(first, 8)
                        .ok_or("war goal pointer address overflow")?,
                };
                page_len += 1;
                planned += count;
                if page_len == pages.len() {
                    copy_goal_pages(memory, base, countries, &pages, goals, &mut used[1])?;
                    page_len = 0;
                    planned = 0;
                }
                first += count as u32;
            }
        }
    }
    copy_goal_pages(
        memory,
        base,
        countries,
        &pages[..page_len],
        goals,
        &mut used[1],
    )?;
    Ok(())
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<OwnedRow>,
    goals: &mut Vec<cwar_goal::relation::Row>,
    scores: &mut Vec<CountryRow>,
) -> [Outcome; 3] {
    let mut cache = DiscoveryCache::new(scores.len() + 1, 0);
    walk_cached(memory, base, state, output, goals, scores, &mut cache)
}

#[allow(clippy::too_many_arguments)]
pub fn walk_cached(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<OwnedRow>,
    goals: &mut Vec<cwar_goal::relation::Row>,
    scores: &mut Vec<CountryRow>,
    cache: &mut DiscoveryCache,
) -> [Outcome; 3] {
    walk_cached_buffer(
        memory,
        base,
        state,
        Buffer::Growing(output),
        Buffer::Growing(goals),
        Buffer::Growing(scores),
        cache,
        &mut |_, _| {},
        &mut || false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_cached_with_owners(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<OwnedRow>,
    goals: &mut Vec<cwar_goal::relation::Row>,
    scores: &mut Vec<CountryRow>,
    cache: &mut DiscoveryCache,
    owners: &mut dyn FnMut(engine_native_memory::country_identity::Country, usize),
    certify: &mut dyn FnMut() -> bool,
) -> [Outcome; 3] {
    walk_cached_buffer(
        memory,
        base,
        state,
        Buffer::Growing(output),
        Buffer::Growing(goals),
        Buffer::Growing(scores),
        cache,
        owners,
        certify,
    )
}

#[allow(clippy::too_many_arguments)]
fn carry_held(
    countries: &ccountry::Reader<'_, impl Memory>,
    output: &mut Buffer<'_, OwnedRow>,
    goals: &mut Buffer<'_, cwar_goal::relation::Row>,
    scores: &mut Buffer<'_, CountryRow>,
    cache: &mut DiscoveryCache,
    owners: &mut dyn FnMut(engine_native_memory::country_identity::Country, usize),
    certify: &mut dyn FnMut() -> bool,
    used: &mut [usize; 3],
    wanted: &mut [usize; 3],
) -> Result<bool, &'static str> {
    let DiscoveryCache {
        generation,
        headers,
        rows,
        next_rows,
        held,
        next,
        ..
    } = cache;
    let Some(held) = held.as_ref() else {
        return Ok(false);
    };
    if held.countries != next.countries || held.owned > rows.len().min(next_rows.len()) {
        return Ok(false);
    }
    let mut visits = held.visits.iter();
    for slot in 1..countries.count() {
        if let Some(country) = countries.at(slot)?
            && visits.next().map(|visit| visit.country) != Some(country)
        {
            return Ok(false);
        }
    }
    if visits.next().is_some()
        || !certify()
        || !output.ensure(held.owned)?
        || !goals.ensure(held.goals.len())?
    {
        return Ok(false);
    }
    output[..held.owned].copy_from_slice(&rows[..held.owned]);
    next_rows[..held.owned].copy_from_slice(&rows[..held.owned]);
    goals[..held.goals.len()].copy_from_slice(&held.goals);
    (used[0], used[1]) = (held.owned, held.goals.len());
    (wanted[0], wanted[1]) = (held.owned, held.goals.len());
    for visit in &held.visits {
        if let Some(header) = headers.get_mut(visit.country.canonical_slot as usize) {
            header.generation = *generation;
        }
        owners(visit.country, visit.diplomacy);
        if visit.has_war {
            wanted[2] += 1;
            if let Some(row) = scores.get_mut(used[2]) {
                *row = CountryRow {
                    country: visit.country.id,
                    major: 0,
                    ..Default::default()
                };
                used[2] += 1;
            }
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn walk_cached_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: Buffer<'_, OwnedRow>,
    mut goals: Buffer<'_, cwar_goal::relation::Row>,
    mut scores: Buffer<'_, CountryRow>,
    cache: &mut DiscoveryCache,
    owners: &mut dyn FnMut(engine_native_memory::country_identity::Country, usize),
    certify: &mut dyn FnMut() -> bool,
) -> [Outcome; 3] {
    if cache.scope != Some((base, state)) {
        cache.invalidate();
        cache.scope = Some((base, state));
    }
    cache.viewpoint = None;
    cache.carried = false;
    cache.next.visits.clear();
    cache.next.goals.clear();
    let witnessed = ccountry::copy_links_and_roster(memory, state, &mut cache.next.countries);
    let mut holdable = witnessed;
    let mut used = [0_usize; 3];
    let mut wanted = [0_usize; 3];
    let mut score_failure = None;
    let result = (|| {
        cache.generation = cache
            .generation
            .checked_add(1)
            .ok_or("war country source generation exhausted")?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        if output.is_growing() {
            cache
                .try_grow(countries.count() as usize, cache.pointers.len())
                .map_err(|_| "war discovery allocation failed")?;
        }
        scores.ensure(countries.count().saturating_sub(1) as usize)?;
        let view = (|| {
            let playing = field::<i32>(memory, state, crate::cgame_state::LOCAL_PLAYER_COUNTRY)?;
            let raw = if playing > 0 {
                playing as u32
            } else {
                field::<u32>(memory, state, crate::cgame_state::OBSERVED_COUNTRY)?
            };
            if raw == 0 {
                Ok(None)
            } else {
                Ok(Some(countries.resolve(raw)?))
            }
        })();
        cache.viewpoint = Some(view.map(|v| v.map_or(0, |c| c.id)));
        if let Err(reason) = view {
            score_failure = Some(reason);
        }
        let view = view.ok().flatten();
        cache.reachable.fill(false);
        cache.enemies.fill(false);
        cache.carried = witnessed
            && carry_held(
                &countries,
                &mut output,
                &mut goals,
                &mut scores,
                cache,
                owners,
                certify,
                &mut used,
                &mut wanted,
            )?;
        if !cache.carried {
            let mut entry_start = 0_usize;
            let mut pending = [Occurrence::default(); 64];
            let mut pending_len = 0;
            for page in (1..countries.count()).step_by(64) {
                let mut seeds = [None; 64];
                let mut pointers = [0; 64];
                let mut n = 0;
                for slot in page..(page + 64).min(countries.count()) {
                    if let Some(country) = countries.at(slot)? {
                        seeds[n] = Some(country);
                        pointers[n] = country.pointer;
                        n += 1;
                    }
                }
                let mut roots = [0; 64 * 8];
                gather(
                    memory,
                    &pointers[..n],
                    [(ccountry::DIPLOMACY_STATUS, 8)],
                    &mut roots,
                )?;
                let roots: [usize; 64] = std::array::from_fn(|i| pointer(&roots[i * 8..]));
                let mut records = [0; 64 * 32];
                gather(
                    memory,
                    &roots[..n],
                    [
                        (0, 8),
                        (engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 16),
                        (engine_native_parts::cdiplomacy_status_layout::COUNTRY, 8),
                    ],
                    &mut records,
                )?;
                let mut vectors = [Array::default(); 64];
                let mut addresses = [0; 64];
                for index in 0..n {
                    let record = &records[index * 32..(index + 1) * 32];
                    vectors[index] = Array::from_header(record[8..24].try_into().unwrap(), 8)
                        .ok_or("war overview array invalid or unreadable")?;
                    if vectors[index].count != 0 {
                        addresses[index] = vectors[index]
                            .element(0, 8)
                            .ok_or("war owned vector address overflow")?;
                    }
                }
                let mut owned_pages = [0; 64 * 512];
                let mut chunks = owned_pages.chunks_exact_mut(512);
                let mut destinations: [&mut [u8]; 64] = std::array::from_fn(|i| {
                    &mut chunks.next().unwrap()[..vectors[i].count.min(64) as usize * 8]
                });
                if memory.copy_scattered(addresses, &mut destinations) != 64 {
                    return Err("war overview owned pointer page unreadable");
                }
                let mut type_starts = [0; 64];
                let mut page_types = [0; 64 * 64 * 4];
                gather_owned_types(
                    memory,
                    &owned_pages,
                    &vectors[..n],
                    &mut type_starts,
                    &mut page_types,
                )?;
                for index in 0..n {
                    let country = seeds[index].unwrap();
                    let slot = country.canonical_slot;
                    let root = roots[index];
                    let record = &records[index * 32..(index + 1) * 32];
                    if pointer(record)
                        != engine_sites::at(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)
                            .ok_or("war diplomacy vtable withheld")?
                    {
                        return Err("war overview vtable mismatch");
                    }
                    if pointer(&record[24..]) != country.pointer {
                        return Err("war overview diplomacy owner mismatch");
                    }
                    owners(country, root);
                    let owned = vectors[index];
                    holdable &= engine_allocation::advisory(|| {
                        engine_allocation::reserve(&mut cache.next.visits, 1)
                    })
                    .is_ok();
                    if holdable {
                        cache.next.visits.push(Visit {
                            country,
                            diplomacy: root,
                            has_war: false,
                        });
                    }
                    if output.is_growing() {
                        let required = entry_start
                            .checked_add(owned.count as usize)
                            .ok_or("war discovery demand overflow")?;
                        cache
                            .try_grow(countries.count() as usize, required)
                            .map_err(|_| "war discovery allocation failed")?;
                        output.ensure(required)?;
                    }
                    let mut header = OwnerHeader {
                        generation: cache.generation,
                        raw: country.id,
                        country: country.pointer,
                        diplomacy: root,
                        owned,
                        start: entry_start,
                        owned_start: wanted[0].min(output.len()),
                        owned_count: 0,
                    };
                    let previous_header = cache
                        .headers
                        .get(slot as usize)
                        .copied()
                        .unwrap_or_default();
                    let unchanged = previous_header.country == header.country
                        && previous_header.diplomacy == header.diplomacy
                        && previous_header.owned == header.owned
                        && previous_header.start == header.start
                        && previous_header.owned_start == header.owned_start;
                    let mut has_war = false;
                    for first_ordinal in (0..owned.count).step_by(64) {
                        let count = (owned.count - first_ordinal).min(64) as usize;
                        let mut handles = [0; 64 * 8];
                        let handles = if first_ordinal == 0 {
                            &owned_pages[index * 512..(index + 1) * 512]
                        } else {
                            if !engine_native_memory::read_slice(
                                memory,
                                owned
                                    .element(first_ordinal, 8)
                                    .ok_or("war owned vector address overflow")?,
                                &mut handles[..count * 8],
                            ) {
                                return Err("war overview owned pointer page unreadable");
                            }
                            &handles[..]
                        };
                        let pointers: [usize; 64] =
                            std::array::from_fn(|i| pointer(&handles[i * 8..]));
                        let mut types = [0; 64 * 4];
                        let types = if first_ordinal == 0 {
                            &page_types[type_starts[index] * 4..(type_starts[index] + count) * 4]
                        } else {
                            gather(
                                memory,
                                &pointers[..count],
                                [(crelation::TYPE_ID, 4)],
                                &mut types,
                            )?;
                            &types[..count * 4]
                        };
                        let mut wars = [0; 64];
                        let mut ordinals = [0; 64];
                        let mut n = 0;
                        let range = entry_start + first_ordinal as usize
                            ..entry_start + first_ordinal as usize + count;
                        let reusable = unchanged
                            && cache.pointers.get(range.clone()) == Some(&pointers[..count])
                            && cache.types.get(range.start * 4..range.end * 4)
                                == Some(&types[..count * 4]);
                        if reusable {
                            let old = &cache.rows[previous_header.owned_start
                                ..previous_header.owned_start + previous_header.owned_count];
                            let first = old.partition_point(|r| r.ordinal < first_ordinal);
                            for row in old[first..]
                                .iter()
                                .take_while(|r| r.ordinal < first_ordinal + count as u32)
                            {
                                wars[n] = pointers[(row.ordinal - first_ordinal) as usize];
                                ordinals[n] = row.ordinal;
                                n += 1;
                            }
                        } else {
                            for (index, &source) in pointers[..count].iter().enumerate() {
                                if word(&types[index * 4..]) == super::TYPE {
                                    wars[n] = source;
                                    ordinals[n] = first_ordinal + index as u32;
                                    n += 1;
                                }
                            }
                        }
                        if let Some(old) = cache.pointers.get_mut(range.clone()) {
                            old.copy_from_slice(&pointers[..count]);
                        }
                        if let Some(old) = cache.types.get_mut(range.start * 4..range.end * 4) {
                            old.copy_from_slice(&types[..count * 4]);
                        }
                        for index in 0..n {
                            has_war = true;
                            wanted[0] += 1;
                            pending[pending_len] = Occurrence {
                                diplomacy: root,
                                owner: country.id,
                                ordinal: ordinals[index],
                                pointer: wars[index],
                            };
                            pending_len += 1;
                            if pending_len == pending.len() {
                                copy_wars(
                                    memory,
                                    base,
                                    &countries,
                                    &pending,
                                    &mut output,
                                    &mut goals,
                                    cache,
                                    &mut used,
                                    &mut wanted,
                                )?;
                                pending_len = 0;
                            }
                        }
                    }
                    entry_start = entry_start
                        .checked_add(owned.count as usize)
                        .ok_or("war discovery demand overflow")?;
                    header.owned_count = wanted[0].min(output.len()) - header.owned_start;
                    if let Some(old) = cache.headers.get_mut(slot as usize) {
                        *old = header;
                    }
                    if has_war {
                        if holdable && let Some(visit) = cache.next.visits.last_mut() {
                            visit.has_war = true;
                        }
                        wanted[2] += 1;
                        if let Some(row) = scores.get_mut(used[2]) {
                            *row = CountryRow {
                                country: country.id,
                                major: 0,
                                ..Default::default()
                            };
                            used[2] += 1;
                        }
                    }
                }
            }
            copy_wars(
                memory,
                base,
                &countries,
                &pending[..pending_len],
                &mut output,
                &mut goals,
                cache,
                &mut used,
                &mut wanted,
            )?;
            cache.required = entry_start;
            holdable &= used[1] == wanted[1]
                && used[0] <= cache.next_rows.len()
                && engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut cache.next.goals, used[1])
                })
                .is_ok();
            if holdable {
                cache.next.goals.extend_from_slice(&goals[..used[1]]);
            }
        }
        for rows in scores[..used[2]].chunks_mut(64) {
            if score_failure.is_some() {
                break;
            }
            let mut pointers = [0; 64];
            for (index, row) in rows.iter().enumerate() {
                match countries.resolve(row.country) {
                    Ok(country) => pointers[index] = country.pointer,
                    Err(reason) => {
                        score_failure.get_or_insert(reason);
                        break;
                    }
                }
            }
            if score_failure.is_some() {
                break;
            }
            let mut majors = [0; 64];
            match gather(
                memory,
                &pointers[..rows.len()],
                [(ccountry::DYNAMIC_MAJOR, 1)],
                &mut majors,
            ) {
                Ok(()) => {
                    for (index, row) in rows.iter_mut().enumerate() {
                        row.major = majors[index];
                    }
                }
                Err(reason) => {
                    score_failure.get_or_insert(reason);
                }
            }
        }
        if let Some(view) = view {
            if cache.reachable.len() < countries.count() as usize {
                score_failure.get_or_insert("war score reachability storage truncated");
            } else {
                cache.reachable[view.canonical_slot as usize] = true;
                for row in &output[..used[0]] {
                    if countries.resolve(row.owner)?.canonical_slot == view.canonical_slot {
                        let other = if countries.resolve(row.first_country)?.canonical_slot
                            == view.canonical_slot
                        {
                            row.second_country
                        } else {
                            row.first_country
                        };
                        let other = countries.resolve(other)?.canonical_slot as usize;
                        cache.reachable[other] = true;
                        if row.retired == 0 {
                            cache.enemies[other] = true;
                        }
                    }
                }
                for row in &output[..used[0]] {
                    if cache.enemies[countries.resolve(row.owner)?.canonical_slot as usize] {
                        cache.reachable
                            [countries.resolve(row.first_country)?.canonical_slot as usize] = true;
                        cache.reachable
                            [countries.resolve(row.second_country)?.canonical_slot as usize] = true;
                    }
                }
            }
        }
        if wanted[0] != used[0] {
            score_failure.get_or_insert("war score reachability owned rows truncated");
        }
        let mut templates = crate::cproduction_status::WarFactoryTemplates::new();
        for row in &mut scores[..used[2]] {
            if score_failure.is_some() {
                break;
            }
            let country = match countries.resolve(row.country) {
                Ok(country) => country,
                Err(reason) => {
                    score_failure.get_or_insert(reason);
                    break;
                }
            };
            if !view.is_some_and(|_| {
                cache.reachable.get(country.canonical_slot as usize) == Some(&true)
            }) {
                continue;
            }
            let result = (|| {
                row.divisions =
                    array(memory, country.pointer, ccountry::DIVISIONS, 8)?.count as i32;
                row.factories = crate::cproduction_status::war_overview_factories(
                    memory,
                    base,
                    &countries,
                    country,
                    &mut templates,
                )?;
                row.view_subject =
                    cached_subject(memory, base, &countries, view.unwrap(), country)? as u8;
                row.score_available = 1;
                Ok(())
            })();
            if let Err(reason) = result {
                score_failure.get_or_insert(reason);
            }
        }
        Ok(())
    })();
    std::mem::swap(&mut cache.rows, &mut cache.next_rows);
    if let Err(reason) = result {
        cache.invalidate();
        return [Outcome::Unavailable { reason }; 3];
    }
    if wanted[0] != used[0] {
        cache.invalidate();
    } else if !cache.carried {
        let previous = cache.held.take();
        if holdable && wanted[1] == used[1] {
            let mut held = std::mem::replace(&mut cache.next, previous.unwrap_or_default());
            held.owned = used[0];
            cache.held = Some(held);
        }
    }
    let mut outcomes = std::array::from_fn(|index| {
        let unread = wanted[index] - used[index];
        match u32::try_from(unread) {
            Ok(0) => Outcome::Rows { len: used[index] },
            Ok(unread_rows) => Outcome::Truncated {
                len: used[index],
                unread_rows,
            },
            Err(_) => Outcome::Unavailable {
                reason: "war overview unread count overflow",
            },
        }
    });
    if let Some(reason) = score_failure {
        outcomes[2] = Outcome::Unavailable { reason };
    }
    outcomes
}

fn cached_subject(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    view: engine_native_memory::country_identity::Country,
    target: engine_native_memory::country_identity::Country,
) -> Result<bool, &'static str> {
    if view.canonical_slot == target.canonical_slot {
        return Ok(false);
    }
    let root = diplomacy(memory, base, view)?;
    let matrix = array(memory, root, engine_native_parts::cdiplomacy_status_layout::RELATIONS, 8)?;
    let status = element(memory, matrix, target.canonical_slot)?;
    vtable(memory, base, status, crelation_status::VTABLE_RVA)?;
    if field::<u64>(memory, status, crelation_status::DIPLOMACY_STATUS)? != root as u64
        || countries
            .resolve(field(memory, status, crelation_status::TARGET_COUNTRY)?)?
            .canonical_slot
            != target.canonical_slot
    {
        return Err("war naming subject status mismatch");
    }
    let pointer = field::<u64>(memory, status, crelation_status::SUBJECT_RELATION)? as usize;
    if pointer == 0 {
        return Ok(false);
    }
    vtable(memory, base, pointer, csubject_relation::VTABLE_RVA)?;
    if field::<u32>(memory, pointer, crelation::TYPE_ID)? != csubject_relation::TYPE_VALUE {
        return Err("war naming subject type mismatch");
    }
    Ok(countries
        .resolve(field(memory, pointer, crelation::FIRST_COUNTRY)?)?
        .canonical_slot
        == target.canonical_slot
        || countries
            .resolve(field(memory, pointer, crelation::SECOND_COUNTRY)?)?
            .canonical_slot
            == target.canonical_slot)
}
