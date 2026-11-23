
use crate::combat_sources::{Run, Sources, share};
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Buffer};
use engine_native::{carmy, ccountry, cgame_state, cprovince, cunit};
use engine_executable::modifier_keys as keys;
use engine_sites::Rva;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierTerm {
    pub scope: u8,
    pub key: u32,
    pub raw: i64,
    pub present: u8,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SupplyTerm {
    pub subunit: u32,
    pub count: i32,
    pub special: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AcclimatizationTerm {
    pub weight: i64,
    pub active: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportTerm {
    pub country: u32,
    pub regional_value: i64,
    pub own_intel: i64,
    pub enemy_intel: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    pub scalar_values: [i64; 32],
    pub flags: [u8; 12],
    pub ids: [u32; 8],
    pub modifier_count: usize,
    pub supply_term_count: usize,
    pub acclimatization_count: usize,
    pub transport_term_count: usize,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    p: usize,
    o: usize,
) -> Result<T, &'static str> {
    p.checked_add(o)
        .and_then(|at| read(memory, at))
        .ok_or("reinforcement input unreadable")
}

fn copy_input_spans<const N: usize>(
    memory: &impl Memory,
    sources: [(usize, usize); N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    let addresses = sources.map(|(pointer, offset)| pointer.checked_add(offset));
    if addresses.iter().any(Option::is_none) {
        for (address, destination) in addresses.into_iter().zip(destinations) {
            let address = address.ok_or("reinforcement input unreadable")?;
            if !engine_native_memory::read_slice(memory, address, destination) {
                return Err("reinforcement input unreadable");
            }
        }
        return Ok(());
    }
    if memory.copy_scattered(addresses.map(Option::unwrap), destinations) == N {
        Ok(())
    } else {
        Err("reinforcement input unreadable")
    }
}

fn global(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or(UNREADABLE)
}

fn formation_flags(memory: &impl Memory, army: usize) -> Result<[u8; 4], &'static str> {
    let mut values = [[0; 1]; 4];
    copy_input_spans(
        memory,
        [carmy::RETREAT, 0x24d, 0x2ae, 0x2e].map(|offset| (army, offset)),
        &mut values.each_mut().map(|value| value.as_mut_slice()),
    )?;
    Ok(values.map(|value| value[0]))
}

fn supply_scalars(
    memory: &impl Memory,
    base: usize,
    army: usize,
) -> Result<[i64; 8], &'static str> {
    let mut values = [[0; 8]; 8];
    let mut fields = values.iter_mut();
    copy_input_spans(
        memory,
        [
            (army, 0x600),
            (army, 0x610),
            (army, 0x40),
            (army, 0x58),
            (army, 0x498),
            (global(base, Rva(0x033e_3678))?, 0),
            (global(base, Rva(0x033e_3844))?, 0),
            (global(base, Rva(0x033e_3990))?, 0),
        ],
        &mut std::array::from_fn(|index| {
            &mut fields.next().unwrap()[..if (4..7).contains(&index) { 4 } else { 8 }]
        }),
    )?;
    Ok(std::array::from_fn(|index| {
        if (4..7).contains(&index) {
            i64::from(i32::from_le_bytes(values[index][..4].try_into().unwrap()))
        } else {
            i64::from_le_bytes(values[index])
        }
    }))
}

fn pointer(memory: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, p, o).map(|p| p as usize)
}

fn array(memory: &impl Memory, p: usize, o: usize, stride: usize) -> Result<Array, &'static str> {
    let header = field(memory, p, o)?;
    Array::from_header(header, stride).ok_or("reinforcement array invalid")
}

fn element(array: Array, index: u32, stride: usize) -> Result<usize, &'static str> {
    array
        .element(index, stride)
        .ok_or("reinforcement array index invalid")
}

fn auth(memory: &impl Memory, base: usize, p: usize, rva: Rva) -> Result<(), &'static str> {
    if p == 0 || engine_sites::at(base, rva).map(|n| n as u64) != Some(field(memory, p, 0)?) {
        return Err("reinforcement dependency identity invalid");
    }
    Ok(())
}

const UNREADABLE: &str = "reinforcement input unreadable";

#[allow(clippy::too_many_arguments)]
fn append_modifiers(
    memory: &impl Memory,
    base: usize,
    sources: &mut Sources,
    accumulator: usize,
    scope: u8,
    keys: &[u32],
    output: &mut Buffer<'_, ModifierTerm>,
    len: &mut usize,
) -> Result<(), &'static str> {
    if accumulator == 0 {
        return Ok(());
    }
    let required = len
        .checked_add(keys.len())
        .ok_or("reinforcement modifier arena overflow")?;
    if !output.ensure(required)? {
        return Err("reinforcement modifier arena full");
    }
    let start = *len;
    for key in keys {
        if *key == 0 {
            return Err("reinforcement modifier key invalid");
        }
        output[*len] = ModifierTerm {
            scope,
            key: *key,
            ..ModifierTerm::default()
        };
        *len += 1;
    }
    let mut shared = true;
    for term in &mut output[start..*len] {
        if let Some(value) = memory.modifier_accumulator_value(accumulator, term.key) {
            let value = value?;
            term.raw = value.raw;
            term.present = u8::from(value.present);
        } else {
            shared = false;
        }
    }
    if !shared {
        for term in &mut output[start..*len] {
            term.raw = 0;
            term.present = 0;
        }
        let run = *share(&mut sources.censuses, accumulator, || {
            census(memory, accumulator, &mut sources.census_entries)
        })?;
        let entries = run.of(&sources.census_entries);
        for term in &mut output[start..*len] {
            if let Ok(index) = entries.binary_search_by_key(&term.key, |entry| entry.0) {
                term.present = 1;
                term.raw = entries[index].1;
            }
        }
    }
    use engine_native_parts::modifier_definition as metadata;
    let table =
        sources.field::<u64>(memory, global(base, metadata::TABLE_RVA)?, 0, UNREADABLE)? as usize;
    for term in &mut output[start..*len] {
        if term.present != 0 {
            let key = term.key;
            let (flags, feature) = *share(&mut sources.metadata, key, || {
                let row = table
                    .checked_add(key as usize * metadata::STRIDE)
                    .filter(|_| table != 0)
                    .ok_or("reinforcement modifier metadata null")?;
                let bytes = field::<[u8; 12]>(memory, row, metadata::FLAGS)?;
                Ok((
                    u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    u32::from_le_bytes(bytes[8..].try_into().unwrap()),
                ))
            })?;
            term.metadata_flags = flags;
            term.feature_selector = feature;
        }
    }
    Ok(())
}

fn census(
    memory: &impl Memory,
    accumulator: usize,
    arena: &mut Vec<(u32, i64)>,
) -> Result<Run, &'static str> {
    let entries = array(memory, accumulator, 0, 16)?;
    let start = arena.len();
    engine_allocation::reserve_push_batch(arena, entries.count as usize)
        .map_err(|_| "reinforcement modifier allocation failed")?;
    let read = (|| {
        let mut batch = [0_u8; 64 * 16];
        let mut previous = 0;
        for first in (0..entries.count as usize).step_by(64) {
            let count = (entries.count as usize - first).min(64);
            if !engine_native_memory::read_slice(
                memory,
                element(entries, first as u32, 16)?,
                &mut batch[..count * 16],
            ) {
                return Err("reinforcement modifier entries unreadable");
            }
            for entry in batch[..count * 16].chunks_exact(16) {
                let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
                if key <= previous {
                    return Err("reinforcement modifier order invalid");
                }
                previous = key;
                arena.push((key, i64::from_le_bytes(entry[8..16].try_into().unwrap())));
            }
        }
        Ok(())
    })();
    match read {
        Ok(()) => Ok(Run {
            start,
            len: arena.len() - start,
        }),
        Err(reason) => {
            arena.truncate(start);
            Err(reason)
        }
    }
}

fn state_accumulator(
    memory: &impl Memory,
    base: usize,
    game: usize,
    state: usize,
    owner: u32,
) -> Result<usize, &'static str> {
    use engine_native::cstate;
    auth(memory, base, state, cstate::VTABLE_RVA)?;
    let links = array(memory, game, cgame_state::COUNTRY_LINKS, 4)?;
    let canonical = |raw: u32| field::<u32>(memory, element(links, raw, 4)?, 0);
    let hash = canonical(owner)?;
    let header = field::<[u8; 17]>(memory, state, cstate::COUNTRY_MODIFIERS)?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let buckets = mask
        .checked_add(1)
        .filter(|n| n.is_power_of_two())
        .ok_or("reinforcement state hash invalid")?;
    let extent = buckets
        .checked_add(u32::from(header[16]))
        .ok_or("reinforcement state hash overflow")?;
    let mut selected = None;
    for probe in 1..=255_u32 {
        let index = (hash & mask)
            .checked_add(probe - 1)
            .filter(|n| *n < extent)
            .ok_or("reinforcement state hash extent invalid")?;
        let bucket = data
            .checked_add(index as usize * cstate::COUNTRY_MODIFIER_STRIDE)
            .filter(|_| data != 0)
            .ok_or("reinforcement state hash null")?;
        let distance = field::<u8>(memory, bucket, 4)?;
        if distance == 0 || probe > u32::from(distance) {
            break;
        }
        let raw = field::<u32>(memory, bucket, 8)?;
        if raw == owner || (raw != 0 && owner != 0 && canonical(raw)? == hash) {
            selected = Some(bucket);
            break;
        }
        if probe == 255 {
            return Err("reinforcement state hash bound exceeded");
        }
    }
    selected
        .map_or_else(
            || state.checked_add(cstate::RESOURCE_MODIFIER + 0x10),
            |p| p.checked_add(0x20),
        )
        .ok_or("reinforcement state accumulator overflow")
}

fn acclimatization(
    memory: &impl Memory,
    army: usize,
    row: &mut ContextRow,
    output: &mut Buffer<'_, AcclimatizationTerm>,
) -> Result<(), &'static str> {
    let object = army
        .checked_add(carmy::ACCLIMATIZATION)
        .ok_or("reinforcement acclimatization overflow")?;
    let active = pointer(memory, object, 0x20)?;
    row.flags[4] = u8::from(active != 0);
    if active == 0 {
        return Ok(());
    }
    let entries = array(memory, object, 8, 16)?;
    let count = entries.count as usize;
    if !output.ensure(count)? {
        return Err("reinforcement acclimatization arena full");
    }
    for (index, term) in output[..count].iter_mut().enumerate() {
        let entry = element(entries, index as u32, 16)?;
        *term = AcclimatizationTerm {
            weight: field(memory, entry, 8)?,
            active: u8::from(pointer(memory, entry, 0)? == active),
        };
    }
    row.scalar_values[0] = field(memory, object, 0x38)?;
    row.scalar_values[1] = field(memory, active, 0x30)?;
    row.acclimatization_count = count;
    Ok(())
}

fn infrastructure(
    memory: &impl Memory,
    base: usize,
    state: usize,
    row: &mut ContextRow,
) -> Result<(), &'static str> {
    use engine_native_parts::{cbuilding, cbuilding_database, cbuilding_template};
    use engine_native::{cprovince::buildings as cbuilding_status, cstate};
    if state == 0 {
        return Ok(());
    }
    auth(memory, base, state, cstate::VTABLE_RVA)?;
    row.flags[9] = 1;
    row.ids[3] = field(memory, state, cstate::ID)?;
    let database = pointer(memory, global(base, cbuilding_database::SINGLETON_RVA)?, 0)?;
    let definition = pointer(memory, database, 0x3b0)?;
    if database == 0 || definition == 0 {
        return Err("reinforcement infrastructure definition null");
    }
    row.scalar_values[20] = i64::from(field::<i32>(memory, definition, 0x2bc)?);
    if row.scalar_values[20] < 1 {
        row.scalar_values[21] = i64::from(field::<i32>(memory, definition, 0x2b8)?);
        if row.scalar_values[21] < 1 {
            row.scalar_values[22] =
                i64::from(field::<i32>(memory, global(base, Rva(0x033e_5274))?, 0)?);
        }
    }
    row.scalar_values[23] = field(memory, global(base, Rva(0x033e_5080))?, 0)?;
    row.flags[10] = field(
        memory,
        definition,
        cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
    )?;
    let status = state
        .checked_add(cstate::BUILDING_STATUS)
        .ok_or("reinforcement building status overflow")?;
    if row.flags[10] != 0
        && field::<i32>(memory, status, cbuilding_status::BUILDING_INDEX_COUNT)? != 0
    {
        let indices = array(memory, status, cbuilding_status::BUILDING_INDICES, 4)?;
        let ordinal = field::<u32>(memory, definition, cbuilding_template::DATABASE_ORDINAL)?;
        let index = field::<i32>(memory, element(indices, ordinal, 4)?, 0)?;
        if index != cbuilding_status::NO_BUILDING_INDEX {
            let buildings = array(memory, status, cbuilding_status::BUILDINGS, 8)?;
            let building = pointer(
                memory,
                element(
                    buildings,
                    u32::try_from(index)
                        .map_err(|_| "reinforcement infrastructure index invalid")?,
                    8,
                )?,
                0,
            )?;
            if building != 0 {
                if pointer(memory, building, cbuilding::TEMPLATE)? != definition {
                    return Err("reinforcement infrastructure template mismatch");
                }
                row.scalar_values[24] =
                    i64::from(field::<i16>(memory, building, cbuilding::HEALTHY_LEVEL)?);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Infrastructure {
    id: u32,
    lookup: u8,
    scalars: [i64; 5],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SupplyEntries {
    count: usize,
    entries: Result<Run, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Priority {
    id: u32,
    token: u32,
    categories: Result<Categories, &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Categories {
    count: u32,
    tokens: Run,
    failure: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportEnemy {
    raw: u32,
    enemy: engine_native_memory::country_identity::Country,
    lanes: Result<(i64, i64), &'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportOwner {
    enemies: Run,
    failure: Option<&'static str>,
}

fn auth_shared(
    memory: &impl Memory,
    base: usize,
    sources: &mut Sources,
    p: usize,
    rva: Rva,
) -> Result<(), &'static str> {
    if p == 0
        || engine_sites::at(base, rva).map(|n| n as u64)
            != Some(sources.field::<u64>(memory, p, 0, UNREADABLE)?)
    {
        return Err("reinforcement dependency identity invalid");
    }
    Ok(())
}

fn infrastructure_shared(
    memory: &impl Memory,
    base: usize,
    sources: &mut Sources,
    state: usize,
    row: &mut ContextRow,
) -> Result<(), &'static str> {
    if state == 0 {
        return Ok(());
    }
    let shared = *share(&mut sources.infrastructure, state, || {
        let mut scratch = ContextRow::default();
        infrastructure(memory, base, state, &mut scratch)?;
        let mut scalars = [0; 5];
        scalars.copy_from_slice(&scratch.scalar_values[20..25]);
        Ok(Infrastructure {
            id: scratch.ids[3],
            lookup: scratch.flags[10],
            scalars,
        })
    })?;
    row.flags[9] = 1;
    row.ids[3] = shared.id;
    row.flags[10] = shared.lookup;
    row.scalar_values[20..25].copy_from_slice(&shared.scalars);
    Ok(())
}

fn supply_entries(
    memory: &impl Memory,
    base: usize,
    template: usize,
    arena: &mut Vec<SupplyTerm>,
) -> Result<SupplyEntries, &'static str> {
    let entries = array(memory, template, 0xc0, 16)?;
    let count = entries.count as usize;
    let start = arena.len();
    engine_allocation::reserve_push_batch(arena, count)
        .map_err(|_| "reinforcement supply allocation failed")?;
    let read = (0..count).try_for_each(|index| {
        let entry = element(entries, index as u32, 16)?;
        let unit = pointer(memory, entry, 0)?;
        auth(memory, base, unit, engine_native_parts::csub_unit_definition::VTABLE_RVA)?;
        arena.push(SupplyTerm {
            subunit: field(memory, unit, 8)?,
            count: field(memory, entry, 8)?,
            special: field(memory, unit, 0x5b7)?,
        });
        Ok(())
    });
    Ok(SupplyEntries {
        count,
        entries: read.map(|()| Run { start, len: count }),
    })
}

fn supply_terms(
    memory: &impl Memory,
    base: usize,
    sources: &mut Sources,
    template: usize,
    row: &mut ContextRow,
    output: &mut Buffer<'_, SupplyTerm>,
) -> Result<(), &'static str> {
    let shared = *share(&mut sources.supply_terms, template, || {
        supply_entries(memory, base, template, &mut sources.supply_arena)
    })?;
    if !output.ensure(shared.count)? {
        return Err("reinforcement supply term arena full");
    }
    let run = shared.entries?;
    output[..run.len].copy_from_slice(run.of(&sources.supply_arena));
    row.supply_term_count = run.len;
    Ok(())
}

fn priority(
    memory: &impl Memory,
    base: usize,
    template: usize,
    arena: &mut Vec<u32>,
) -> Result<Priority, &'static str> {
    let selected = carmy::movement_priority_unit(memory, base, template)?;
    let id = field(memory, selected, 8)?;
    let token = field::<u32>(memory, selected, 0x5d8)?;
    let categories = array(memory, selected, 0x508, 8).map(|categories| {
        let start = arena.len();
        let failure = engine_allocation::reserve_push_batch(arena, categories.count as usize)
            .map_err(|_| "reinforcement category allocation failed")
            .and_then(|()| {
                (0..categories.count).try_for_each(|index| {
                    let category = pointer(memory, element(categories, index, 8)?, 0)?;
                    arena.push(field::<u32>(memory, category, 0x60)?);
                    Ok(())
                })
            })
            .err();
        Categories {
            count: categories.count,
            tokens: Run {
                start,
                len: arena.len() - start,
            },
            failure,
        }
    });
    Ok(Priority {
        id,
        token,
        categories,
    })
}

pub(crate) fn owned_war_relation(
    memory: &impl Memory,
    relations: Array,
) -> Result<bool, &'static str> {
    use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
    const BATCH: usize = 64;
    const POINTER: RecordLayout<'static> = RecordLayout::new(
        &[RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }],
        8,
    )
    .unwrap();
    const KIND: RecordLayout<'static> = RecordLayout::new(
        &[RecordField {
            source_offset: 8,
            destination_offset: 0,
            len: 4,
        }],
        4,
    )
    .unwrap();
    let mut words = [0_u8; BATCH * 8];
    let mut kinds = [0_u8; BATCH * 4];
    let mut pointers = [0_usize; BATCH];
    let mut first = 0_u32;
    while first < relations.count {
        let len = ((relations.count - first) as usize).min(BATCH);
        let base = element(relations, first, 8)?;
        let mut addressable = len;
        for lane in 1..len {
            if relations.element(first + lane as u32, 8).is_none() {
                addressable = lane;
                break;
            }
        }
        let read = memory
            .copy_records(
                RecordSources::Strided {
                    base,
                    stride: 8,
                    count: addressable,
                },
                &POINTER,
                &mut words[..addressable * 8],
            )
            .min(addressable);
        for (lane, pointer) in pointers[..read].iter_mut().enumerate() {
            *pointer =
                u64::from_le_bytes(words[lane * 8..lane * 8 + 8].try_into().unwrap()) as usize;
        }
        let typed = memory
            .copy_records(
                RecordSources::Pointers(&pointers[..read]),
                &KIND,
                &mut kinds[..read * 4],
            )
            .min(read);
        if kinds[..typed * 4]
            .chunks_exact(4)
            .any(|raw| u32::from_le_bytes(raw.try_into().unwrap()) == 0x380a)
        {
            return Ok(true);
        }
        if typed < read || read < addressable {
            return Err(UNREADABLE);
        }
        if addressable < len {
            return Err("reinforcement array index invalid");
        }
        first += len as u32;
    }
    Ok(false)
}

#[allow(clippy::too_many_arguments)]
fn air(
    memory: &impl Memory,
    base: usize,
    game: usize,
    sources: &mut Sources,
    countries: &ccountry::Reader<'_, impl Memory>,
    army: usize,
    stats: usize,
    location: usize,
    row: &mut ContextRow,
) -> Result<(), &'static str> {
    use engine_native_politics::cdiplomacy_status;
    let owner = countries.resolve(field(memory, army, cunit::LOGICAL_OWNER)?)?;
    let active = *share(&mut sources.air_owners, owner.pointer, || {
        let diplomacy = pointer(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)?;
        if field::<i32>(memory, diplomacy, cdiplomacy_status::ENEMY_COUNTRY_COUNT)? != 0 {
            return Ok(true);
        }
        let relations = array(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 8)?;
        owned_war_relation(memory, relations)
    })?;
    let region =
        sources.field::<u64>(memory, location, cprovince::STRATEGIC_REGION, UNREADABLE)? as usize;
    if !active || region == 0 {
        return Ok(());
    }
    let (index, values) = *share(&mut sources.air_entries, (owner.pointer, region), || {
        let manager = pointer(memory, game, cgame_state::STRATEGIC_AIR_MANAGER)?;
        let countries = array(memory, manager, 0x30, 8)?;
        let country = pointer(memory, element(countries, owner.canonical_slot, 8)?, 0)?;
        let index = field::<i32>(memory, region, 0x60)?;
        let regions = array(memory, country, 0xe0, 0xa0)?;
        let index = u32::try_from(index).map_err(|_| "reinforcement air region negative")?;
        let entry = element(regions, index, 0xa0)?;
        let mut values = [field(memory, entry, 0)?, field(memory, entry, 8)?, 0];
        if values[1] > 0 {
            values[2] = field(memory, entry, 0x18)?;
        }
        Ok((index, values))
    })?;
    row.flags[7] = 1;
    row.ids[7] = index;
    row.scalar_values[13] = values[0];
    row.scalar_values[14] = values[1];
    if row.scalar_values[14] > 0 {
        row.scalar_values[15] = values[2];
        row.scalar_values[16] = field(memory, stats, 0x220)?;
        row.scalar_values[17] =
            sources.field(memory, global(base, Rva(0x033e_1850))?, 0, UNREADABLE)?;
        if row.scalar_values[16] != 0 {
            row.scalar_values[18] =
                sources.field(memory, global(base, Rva(0x033e_18b8))?, 0, UNREADABLE)?;
            row.scalar_values[19] =
                sources.field(memory, global(base, Rva(0x033e_1830))?, 0, UNREADABLE)?;
        }
    }
    Ok(())
}

fn transport_intel_lane(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    source: usize,
    target: usize,
    target_country: engine_native_memory::country_identity::Country,
) -> Result<i64, &'static str> {
    use engine_native_intelligence::{ccountry_intel, country_intel_row};
    let raw = field::<i32>(memory, target, ccountry_intel::OWNER)?;
    if raw <= 0 {
        return Ok(0);
    }
    let resolved = countries.resolve(raw as u32)?;
    if resolved.pointer != target_country.pointer {
        return Err("reinforcement transport intel owner mismatch");
    }
    auth(memory, base, source, ccountry_intel::VTABLE_RVA)?;
    let rows = array(
        memory,
        source,
        ccountry_intel::DATA,
        country_intel_row::STRIDE,
    )?;
    let value = match rows.element(resolved.canonical_slot, country_intel_row::STRIDE) {
        Some(row) => field(memory, row, country_intel_row::NAVY)?,
        None => 0,
    };
    Ok(value)
}

fn transport_owner(
    memory: &impl Memory,
    base: usize,
    game: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    owner: engine_native_memory::country_identity::Country,
    arena: &mut Vec<TransportEnemy>,
) -> Result<TransportOwner, &'static str> {
    use engine_native_intelligence::ccountry_intel;
    use engine_native_politics::cdiplomacy_status;
    use engine_native_naval::cstrategic_navy_manager;
    cstrategic_navy_manager::authenticate(memory, base, game, owner)?;
    let owner_intel = pointer(memory, owner.pointer, ccountry::INTEL_TABLE)?;
    auth(memory, base, owner_intel, ccountry_intel::VTABLE_RVA)?;
    let diplomacy = pointer(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)?;
    auth(memory, base, diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
    if pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != owner.pointer {
        return Err("reinforcement transport diplomacy backlink mismatch");
    }
    let enemies = array(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES, 4)?;
    let start = arena.len();
    let mut failure = None;
    for index in 0..enemies.count {
        let eligible = (|| {
            let raw = field::<u32>(memory, element(enemies, index, 4)?, 0)?;
            let enemy = countries.resolve(raw)?;
            if field::<i32>(memory, enemy.pointer, ccountry::FOG_NETWORK_TABLE_COUNT)? <= 0 {
                return Ok(None);
            }
            let enemy_diplomacy = pointer(memory, enemy.pointer, ccountry::DIPLOMACY_STATUS)?;
            auth(memory, base, enemy_diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
            if pointer(memory, enemy_diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != enemy.pointer {
                return Err("reinforcement transport enemy backlink mismatch");
            }
            if field::<u8>(memory, enemy_diplomacy, cdiplomacy_status::CAPITULATED)? != 0 {
                return Ok(None);
            }
            let enemy_intel = pointer(memory, enemy.pointer, ccountry::INTEL_TABLE)?;
            auth(memory, base, enemy_intel, ccountry_intel::VTABLE_RVA)?;
            Ok(Some((raw, enemy, enemy_intel)))
        })();
        let (raw, enemy, enemy_intel) = match eligible {
            Ok(Some(eligible)) => eligible,
            Ok(None) => continue,
            Err(reason) => {
                failure = Some(reason);
                break;
            }
        };
        let lanes = transport_intel_lane(memory, base, countries, owner_intel, enemy_intel, enemy)
            .and_then(|own| {
                transport_intel_lane(memory, base, countries, enemy_intel, owner_intel, owner)
                    .map(|other| (own, other))
            });
        if arena.len() == arena.capacity() {
            engine_allocation::reserve(arena, 1)
                .map_err(|_| "reinforcement enemy allocation failed")?;
        }
        arena.push(TransportEnemy { raw, enemy, lanes });
        if lanes.is_err() {
            break;
        }
    }
    Ok(TransportOwner {
        enemies: Run {
            start,
            len: arena.len() - start,
        },
        failure,
    })
}

#[allow(clippy::too_many_arguments)]
fn transport_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    sources: &mut Sources,
    army: usize,
    location: usize,
    modifiers: &mut Buffer<'_, ModifierTerm>,
    terms: &mut Vec<TransportTerm>,
    start: usize,
) -> Result<ContextRow, &'static str> {
    use engine_native_parts::{
        cstatic_modifier,
        cstrategic_region,
        modifier_definition as metadata,
    };
    use engine_native_naval::cstrategic_navy_manager;
    use engine_native::cweather_manager::{self, region_weather as sweather_per_region};
    let mut row = ContextRow::default();
    row.flags[11] = 2;
    row.ids[0] = sources.field(memory, location, cprovince::ID, UNREADABLE)?;
    row.ids[5] = sources.field(
        memory,
        global(base, metadata::TRADE_FEATURE_FLAGS_RVA)?,
        0,
        UNREADABLE,
    )?;
    let callback = sources.field::<u64>(
        memory,
        global(base, metadata::ELIGIBILITY_CALLBACK_RVA)?,
        0,
        UNREADABLE,
    )? as usize;
    row.ids[6] = if callback == 0 {
        0
    } else if engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
        1
    } else {
        return Err("reinforcement modifier callback unsupported");
    };
    let region =
        sources.field::<u64>(memory, location, cprovince::STRATEGIC_REGION, UNREADABLE)? as usize;
    auth_shared(memory, base, sources, region, cstrategic_region::VTABLE_RVA)?;
    let region_id = sources.field::<u32>(memory, region, cstrategic_region::ID, UNREADABLE)?;
    row.ids[7] = region_id;
    let manager =
        sources.field::<u64>(memory, game, cgame_state::WEATHER_MANAGER, UNREADABLE)? as usize;
    auth_shared(memory, base, sources, manager, cweather_manager::VTABLE_RVA)?;
    let weather = array(
        memory,
        manager,
        cweather_manager::REGIONS,
        sweather_per_region::STRIDE,
    )?;
    if let Some(record) = weather.element(region_id, sweather_per_region::STRIDE) {
        auth_shared(
            memory,
            base,
            sources,
            record,
            sweather_per_region::VTABLE_RVA,
        )?;
        if sources.field::<u32>(memory, record, sweather_per_region::ID, UNREADABLE)? != region_id {
            return Err("reinforcement transport weather identity mismatch");
        }
        append_modifiers(
            memory,
            base,
            sources,
            record + sweather_per_region::MODIFIERS,
            7,
            &[keys::NAVAL_SPEED_FACTOR],
            modifiers,
            &mut row.modifier_count,
        )?;
    }
    let owner_raw = field::<u32>(memory, army, cunit::LOGICAL_OWNER)?;
    let countries = ccountry::Reader::new(memory, base, game)?;
    let owner = countries.resolve(owner_raw)?;
    if row.ids[5] & (1 << 4) != 0 {
        let definitions =
            sources.field::<u64>(memory, global(base, Rva(0x033d_e590))?, 0, UNREADABLE)? as usize;
        if definitions == 0 {
            return Err("reinforcement transport static modifier set null");
        }
        let definition = sources.field::<u64>(memory, definitions, 0xf8, UNREADABLE)? as usize;
        if definition != 0 {
            auth_shared(
                memory,
                base,
                sources,
                definition,
                cstatic_modifier::VTABLE_RVA,
            )?;
            let walked = *share(&mut sources.transport_owners, owner.pointer, || {
                transport_owner(
                    memory,
                    base,
                    game,
                    &countries,
                    owner,
                    &mut sources.transport_enemies,
                )
            })?;
            let required = start
                .checked_add(row.transport_term_count)
                .and_then(|len| len.checked_add(walked.enemies.len))
                .ok_or("reinforcement transport term count overflow")?;
            if required > terms.len() {
                engine_allocation::reserve_push_batch(terms, required - terms.len())
                    .map_err(|_| "reinforcement transport allocation failed")?;
            }
            for index in walked.enemies.start..walked.enemies.start + walked.enemies.len {
                let entry = sources.transport_enemies[index];
                let regional_value = *share(
                    &mut sources.regional,
                    (entry.enemy.pointer, region_id),
                    || {
                        cstrategic_navy_manager::regional_operand(
                            memory,
                            base,
                            game,
                            entry.enemy,
                            region_id,
                        )
                    },
                )?;
                let (own_intel, enemy_intel) = entry.lanes?;
                let slot = start
                    .checked_add(row.transport_term_count)
                    .ok_or("reinforcement transport term index overflow")?;
                if terms.len() <= slot {
                    terms.resize_with(slot + 1, TransportTerm::default);
                }
                terms[slot] = TransportTerm {
                    country: entry.raw,
                    regional_value,
                    own_intel,
                    enemy_intel,
                };
                row.transport_term_count += 1;
            }
            if let Some(reason) = walked.failure {
                return Err(reason);
            }
            for (index, rva) in [
                (25, Rva(0x033e_1d68)),
                (26, Rva(0x033e_1000)),
                (27, Rva(0x033e_1088)),
                (28, Rva(0x033e_2200)),
            ] {
                row.scalar_values[index] =
                    sources.field(memory, global(base, rva)?, 0, UNREADABLE)?;
            }
            append_modifiers(
                memory,
                base,
                sources,
                owner.pointer + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
                9,
                &[keys::NAVAL_MINES_EFFECT_REDUCTION],
                modifiers,
                &mut row.modifier_count,
            )?;
            append_modifiers(
                memory,
                base,
                sources,
                definition + cstatic_modifier::ACCUMULATOR,
                8,
                &[keys::NAVAL_SPEED_FACTOR],
                modifiers,
                &mut row.modifier_count,
            )?;
            let metadata_table =
                sources.field::<u64>(memory, global(base, metadata::TABLE_RVA)?, 0, UNREADABLE)?
                    as usize;
            row.scalar_values[29] = i64::from(sources.field::<u32>(
                memory,
                metadata_table,
                keys::NAVAL_SPEED_FACTOR as usize * metadata::STRIDE + metadata::CATEGORY_MASK,
                UNREADABLE,
            )?);
        }
    }
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_context(memory: &impl Memory, base: usize, game: usize, sources: &mut Sources, army: usize, expected_handle: u64, modifiers: &mut Vec<ModifierTerm>, supply: &mut Vec<SupplyTerm>, acclimatization_terms: &mut Vec<AcclimatizationTerm>, transport_terms: &mut Vec<TransportTerm>, transport_start: usize) -> Result<ContextRow, &'static str> {
    capture_context_buffer(memory, base, game, sources, army, expected_handle, Buffer::Growing(modifiers), Buffer::Growing(supply), Buffer::Growing(acclimatization_terms), transport_terms, transport_start)
}

pub fn capture_context_buffer(
    memory: &impl Memory,
    base: usize,
    game: usize,
    sources: &mut Sources,
    army: usize,
    expected_handle: u64,
    mut modifiers: Buffer<'_, ModifierTerm>,
    mut supply: Buffer<'_, SupplyTerm>,
    mut acclimatization_terms: Buffer<'_, AcclimatizationTerm>,
    transport_terms: &mut Vec<TransportTerm>,
    transport_start: usize,
) -> Result<ContextRow, &'static str> {
    let verified = sources
        .verified
        .filter(|(pointer, handle, _)| *pointer == army && *handle == expected_handle);
    if verified.is_none() {
        auth(memory, base, army, carmy::VTABLE_RVA)?;
        if expected_handle == 0 || field::<u64>(memory, army, cunit::HANDLE)? != expected_handle {
            return Err("reinforcement army handle mismatch");
        }
    }
    let transfer = pointer(memory, army, carmy::NAVAL_TRANSFER_MEMBERSHIP)?;
    let stats = match verified {
        Some((_, _, stats)) => stats,
        None => {
            let stats = pointer(memory, army, cunit::STAT_BLOCK)?;
            auth(memory, base, stats, engine_native_parts::csub_unit_definition::VTABLE_RVA)?;
            stats
        }
    };
    let location = pointer(memory, army, cunit::LOCATION)?;
    share(&mut sources.definitions, location, || {
        carmy::movement_definition(memory, base, location)
    })?;
    if transfer != 0 {
        return transport_context(
            memory,
            base,
            game,
            sources,
            army,
            location,
            &mut modifiers,
            transport_terms,
            transport_start,
        );
    }
    let template = pointer(memory, army, carmy::STANDARD_TEMPLATE)?;
    auth_shared(
        memory,
        base,
        sources,
        template,
        engine_native::creferenced_division_template::VTABLE_RVA,
    )?;
    let owners = field::<[u8; 8]>(memory, army, cunit::ROSTER_OWNER)?;
    let roster = u32::from_le_bytes(owners[..4].try_into().unwrap());
    let controller = u32::from_le_bytes(owners[4..].try_into().unwrap());
    let owner_raw = if controller as i32 > 0 {
        controller
    } else {
        roster
    };
    let countries = ccountry::Reader::new(memory, base, game)?;
    let owner = countries.resolve(owner_raw)?;
    let leader = sources.leader_chain(memory, base, army, expected_handle)?;
    let country_accumulator = owner
        .pointer
        .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
        .ok_or("reinforcement country overflow")?;
    let actual_state =
        sources.field::<u64>(memory, location, cprovince::STATE, UNREADABLE)? as usize;
    let mut row = ContextRow::default();
    row.ids[0] = sources.field(memory, location, cprovince::ID, UNREADABLE)?;
    let route = array(memory, army, cunit::ORDINARY_ROUTE_DATA, 4)?;
    let next = if route.count == 0 {
        0
    } else {
        carmy::movement_route_province(memory, base, game, route, 0)?
    };
    if next != 0 {
        row.ids[1] = sources.field(memory, next, cprovince::ID, UNREADABLE)?;
    }
    row.flags[..4].copy_from_slice(&formation_flags(memory, army)?);
    row.flags[5] = u8::from(carmy::movement_attacking(memory, base, game, army)?);
    use engine_native_parts::modifier_definition as metadata;
    let callback = sources.field::<u64>(
        memory,
        global(base, metadata::ELIGIBILITY_CALLBACK_RVA)?,
        0,
        UNREADABLE,
    )? as usize;
    row.ids[6] = if callback == 0 {
        0
    } else if engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA) == Some(callback) {
        1
    } else {
        return Err("reinforcement modifier callback unsupported");
    };
    row.ids[5] = sources.field(
        memory,
        global(base, metadata::TRADE_FEATURE_FLAGS_RVA)?,
        0,
        UNREADABLE,
    )?;
    let country_keys = [
        keys::ARMY_SPEED_FACTOR,
        keys::ARMY_ATTACK_SPEED_FACTOR,
        keys::OUT_OF_SUPPLY_FACTOR,
        keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
        keys::ARMY_RETREAT_SPEED_FACTOR,
    ];
    append_modifiers(
        memory,
        base,
        sources,
        country_accumulator,
        0,
        &country_keys,
        &mut modifiers,
        &mut row.modifier_count,
    )?;
    if let Some(leader) = leader {
        append_modifiers(
            memory,
            base,
            sources,
            leader,
            1,
            &[
                keys::ARMY_SPEED_FACTOR,
                keys::OUT_OF_SUPPLY_FACTOR,
                keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
            ],
            &mut modifiers,
            &mut row.modifier_count,
        )?;
    }
    if actual_state != 0 {
        let accumulator = *share(&mut sources.state_sources, (actual_state, roster), || {
            state_accumulator(memory, base, game, actual_state, roster)
        })?;
        row.ids[2] = sources.field(memory, actual_state, engine_native::cstate::ID, UNREADABLE)?;
        let controller =
            sources.field::<u32>(memory, actual_state, engine_native::cstate::CONTROLLER, UNREADABLE)?;
        row.flags[6] = *share(&mut sources.controllers, (owner_raw, controller), || {
            Ok(
                if ccountry::movement_friendly(memory, base, game, owner_raw, controller)? {
                    1
                } else if ccountry::movement_enemy(memory, base, game, owner_raw, controller)? {
                    2
                } else {
                    0
                },
            )
        })?;
        append_modifiers(
            memory,
            base,
            sources,
            accumulator,
            2,
            &[
                keys::ARMY_SPEED_FACTOR,
                keys::ARMY_ATTACK_SPEED_FACTOR,
                keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER,
                keys::ENEMY_ARMY_SPEED_FACTOR,
            ],
            &mut modifiers,
            &mut row.modifier_count,
        )?;
    }
    acclimatization(memory, army, &mut row, &mut acclimatization_terms)?;
    let (province_weather, region_weather) = *share(&mut sources.weather, location, || {
        engine_native::cweather_manager::movement_modifiers(memory, base, game, location)
    })?;
    for (accumulator, scope) in [(province_weather, 3), (region_weather, 4)] {
        append_modifiers(
            memory,
            base,
            sources,
            accumulator,
            scope,
            &[keys::ARMY_SPEED_FACTOR, keys::ARMY_ATTACK_SPEED_FACTOR],
            &mut modifiers,
            &mut row.modifier_count,
        )?;
    }
    let selected = *share(&mut sources.priorities, template, || {
        priority(memory, base, template, &mut sources.category_tokens)
    })?;
    row.ids[4] = selected.id;
    if selected.token != 0 {
        append_modifiers(
            memory,
            base,
            sources,
            country_accumulator,
            5,
            &[selected.token],
            &mut modifiers,
            &mut row.modifier_count,
        )?;
    }
    let categories = selected.categories?;
    for index in 0..categories.count as usize {
        if index == categories.tokens.len {
            return Err(categories.failure.unwrap_or(UNREADABLE));
        }
        let token = sources.category_tokens[categories.tokens.start + index];
        if token != 0 {
            append_modifiers(
                memory,
                base,
                sources,
                country_accumulator,
                6,
                &[token],
                &mut modifiers,
                &mut row.modifier_count,
            )?;
        }
    }
    air(
        memory, base, game, sources, &countries, army, stats, location, &mut row,
    )?;
    if row.flags[2] == 0 {
        row.flags[11] = 1;
        for (index, value) in [2, 3, 4, 10, 5, 6, 7, 8]
            .into_iter()
            .zip(supply_scalars(memory, base, army)?)
        {
            row.scalar_values[index] = value;
        }
        supply_terms(memory, base, sources, template, &mut row, &mut supply)?;
        if row.flags[3] == 0 {
            let area =
                sources.field::<u64>(memory, location, cprovince::CONTROLLER_AREA, UNREADABLE)?
                    as usize;
            let access = *share(&mut sources.core_supply, (owner_raw, area), || {
                ccountry::has_core_supply_access(memory, base, game, owner_raw, area)
            })?;
            if !access {
                row.flags[8] = 1;
                row.scalar_values[9] = sources.field(
                    memory,
                    global(
                        base,
                        if row.scalar_values[10] > 0 {
                            Rva(0x033e_3ad0)
                        } else {
                            Rva(0x033e_3a20)
                        },
                    )?,
                    0,
                    UNREADABLE,
                )?;
            }
        }
    }
    if row.flags[0] != 0 {
        row.scalar_values[11] =
            sources.field(memory, global(base, Rva(0x033e_3c20))?, 0, UNREADABLE)?;
    } else if row.flags[1] != 0 {
        row.scalar_values[12] =
            sources.field(memory, global(base, Rva(0x033e_3cb8))?, 0, UNREADABLE)?;
    }
    let infrastructure_state = if next == 0 {
        actual_state
    } else {
        sources.field::<u64>(memory, next, cprovince::STATE, UNREADABLE)? as usize
    };
    infrastructure_shared(memory, base, sources, infrastructure_state, &mut row)?;
    Ok(row)
}

pub mod speed;

#[cfg(test)]
mod tests;
