use engine_native::carmy_combat_inputs::{EXILE_ORIGIN, NAME_GROUP_MEMBER};

use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice};
use engine_native::{carmy, cunit};
use engine_native_parts::csub_unit_definition;

pub const ARMOUR: usize = 0x298;
pub const AIR_ATTACK: usize = 0x220;
pub const DIRECTION_GROUP: usize = 0x4a8;
pub const CONTEXT_MODIFIER_KEYS: [u32; 3] = [
    engine_executable::modifier_keys::LAND_REINFORCE_RATE,
    engine_executable::modifier_keys::COMBAT_WIDTH_FACTOR,
    engine_executable::modifier_keys::INITIATIVE_FACTOR,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WidthTerm {
    pub subunit: u32,
    pub count: i32,
    pub width: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextModifier {
    pub raw: i64,
    pub present: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    pub template: u64,
    pub leader_modifier_kind: u8,
    pub country_modifiers: [ContextModifier; 3],
    pub leader_modifiers: [ContextModifier; 3],
    pub modifier_metadata: [[u32; 2]; 3],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub base_speed: i64,
    pub width_term_count: usize,
    pub owner: u32,
    pub country_accumulator: usize,
    pub leader_accumulator: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    pub id: u32,
    pub value_a: i64,
    pub value_b: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub division: u64,
    pub naming: Option<engine_native::cname_group_member::Inputs>,
    pub strength_combat_factor: i64,
    pub fuel_inputs: [i64; 2],
    pub stat_values: [i64; 5],
    pub armour: i64,
    pub piercing: i64,
    pub air_attack: i64,
    pub reconnaissance: i64,
    pub initiative: i64,
    pub commander: u64,
    pub scales: [i64; 4],
    pub direction_group: i32,
    pub categories: u64,
}

fn global_bytes<const N: usize>(
    memory: &impl Memory,
    base: usize,
    key: engine_sites::Rva,
) -> Result<[u8; N], &'static str> {
    engine_sites::at(base, key)
        .and_then(|p| read(memory, p))
        .ok_or("land combat division inputs unreadable")
}

fn bytes<const N: usize>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<[u8; N], &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("land combat division inputs unreadable")
}

fn values<const N: usize>(source: &[u8]) -> [i64; N] {
    std::array::from_fn(|index| {
        i64::from_le_bytes(source[index * 8..index * 8 + 8].try_into().unwrap())
    })
}

fn copy_input_spans<const N: usize>(
    memory: &impl Memory,
    addresses: [Result<usize, &'static str>; N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    let copied = memory.copy_scattered_prefetched(addresses.map(|p| p.unwrap_or(0)), destinations);
    if copied == N {
        Ok(())
    } else {
        Err(addresses[copied]
            .err()
            .unwrap_or("land combat division inputs unreadable"))
    }
}


pub fn capture_stats(
    memory: &impl Memory,
    base: usize,
    sources: &mut crate::combat_sources::Sources,
    pointer: usize,
    expected_handle: u64,
    groups: &mut engine_native::cname_group_member::GroupKeys,
    names: &mut engine_native::cname_group_member::Names<'_>,
) -> Result<Row, &'static str> {
    let identity = bytes::<8>(memory, pointer, 0)?;
    let expected_vtable = engine_sites::at(base, carmy::VTABLE_RVA)
        .ok_or("land combat division identity mismatch")? as u64;
    let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
    if pointer == 0
        || u64::from_le_bytes(identity) != expected_vtable
        || expected_handle == 0
        || u64::from_le_bytes(handle) != expected_handle
    {
        return Err("land combat division identity mismatch");
    }
    let stat_pointer = bytes::<8>(memory, pointer, cunit::STAT_BLOCK)?;
    let stat = usize::try_from(u64::from_le_bytes(stat_pointer))
        .map_err(|_| "land combat division stat pointer invalid")?;
    let stat_identity = bytes::<8>(memory, stat, 0)?;
    if stat == 0
        || Some(u64::from_le_bytes(stat_identity))
            != engine_sites::at(base, csub_unit_definition::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat division stat identity mismatch");
    }
    sources.verified = None;
    let (mut armour, mut group_pointer, mut member_pointer) = ([0; 16], [0; 8], [0; 8]);
    let (mut strength_factor, mut fuel, mut fuel_capacity, mut stats, mut air_attack) =
        ([0; 8], [0; 8], [0; 8], [0; 40], [0; 8]);
    let (mut reconnaissance, mut initiative, mut scales, mut direction_group, mut categories) =
        ([0; 8], [0; 8], [0; 32], [0; 4], [0; 8]);
    copy_input_spans(
        memory,
        [
            (stat, ARMOUR),
            (pointer, carmy::ORDERS_GROUP),
            (pointer, NAME_GROUP_MEMBER),
            (pointer, carmy::STRENGTH_FACTOR),
            (pointer, 0x620),
            (stat, 0x120),
            (stat, csub_unit_definition::DEFENSE),
            (stat, AIR_ATTACK),
            (stat, 0xd0),
            (stat, 0xe0),
            (pointer, cunit::ATTACK_ACCUMULATOR),
            (pointer, DIRECTION_GROUP),
            (stat, csub_unit_definition::AIR_FLAGS),
        ]
        .map(|(p, offset)| {
            p.checked_add(offset)
                .ok_or("land combat division inputs unreadable")
        }),
        &mut [
            &mut armour,
            &mut group_pointer,
            &mut member_pointer,
            &mut strength_factor,
            &mut fuel,
            &mut fuel_capacity,
            &mut stats,
            &mut air_attack,
            &mut reconnaissance,
            &mut initiative,
            &mut scales,
            &mut direction_group,
            &mut categories,
        ],
    )?;
    let armour = values::<2>(&armour);
    let group = u64::from_le_bytes(group_pointer) as usize;
    let commander = commander(memory, base, group)?;
    let naming = naming(
        memory,
        base,
        u64::from_le_bytes(member_pointer),
        groups,
        names,
    )?;
    sources.verified = Some((pointer, expected_handle, stat));
    Ok(Row {
        division: expected_handle,
        naming,
        strength_combat_factor: i64::from_le_bytes(strength_factor),
        fuel_inputs: [i64::from_le_bytes(fuel), i64::from_le_bytes(fuel_capacity)],
        stat_values: values(&stats),
        armour: armour[0],
        piercing: armour[1],
        air_attack: i64::from_le_bytes(air_attack),
        reconnaissance: i64::from_le_bytes(reconnaissance),
        initiative: i64::from_le_bytes(initiative),
        commander,
        scales: values(&scales),
        direction_group: i32::from_le_bytes(direction_group),
        categories: u64::from_le_bytes(categories),
    })
}

fn naming(
    memory: &impl Memory,
    base: usize,
    member: u64,
    groups: &mut engine_native::cname_group_member::GroupKeys,
    names: &mut engine_native::cname_group_member::Names<'_>,
) -> Result<Option<engine_native::cname_group_member::Inputs>, &'static str> {
    use engine_native::cname_group_member::Naming;
    let member = usize::try_from(member)
        .ok()
        .filter(|p| *p != 0)
        .ok_or("land combat division name pointer invalid")?;
    Ok(
        match engine_native::cname_group_member::read_inputs(memory, base, member, groups, names, false)? {
            Naming::Row(row) => Some(row),
            Naming::DoesNotFit => None,
        },
    )
}

pub const TEMPLATE_ORIGIN_TYPE: usize = 0x1f8;
pub const TEMPLATE_COUNTRY: usize = 0x1f4;
pub const TEMPLATE_COUNTRY_FALLBACK: usize = 0x1ec;
pub const OCCUPIER_SET: usize = 0x140;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExileRow {
    pub origin_type: i32,
    pub country: i32,
    pub country_fallback: i32,
    pub cores: (usize, usize),
    pub occupiers: (usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exile {
    Absent,
    Row(ExileRow),
    DoesNotFit { required: usize },
}

fn country_list(memory: &impl Memory, address: usize, limit: usize) -> Result<Array, &'static str> {
    let header = read::<[u8; 16]>(memory, address).ok_or("exile country list unreadable")?;
    if i32::from_le_bytes(header[12..16].try_into().unwrap()) <= 0 {
        return Ok(Array { data: 0, count: 0 });
    }
    let list = Array::from_header(header, 4).ok_or("exile country list header invalid")?;
    if list.count as usize > limit {
        return Err("exile country list exceeds the live country count");
    }
    Ok(list)
}

fn copy_tags(memory: &impl Memory, list: Array, tags: &mut [u32]) -> Result<(), &'static str> {
    let mut batch = [0_u8; 64 * 4];
    for (index, tags) in tags.chunks_mut(64).enumerate() {
        let source = list
            .data
            .checked_add(index * batch.len())
            .ok_or("exile country list entry unreadable")?;
        let bytes = &mut batch[..tags.len() * 4];
        if !read_slice(memory, source, bytes) {
            return Err("exile country list entry unreadable");
        }
        for (tag, raw) in tags.iter_mut().zip(bytes.chunks_exact(4)) {
            *tag = u32::from_le_bytes(raw.try_into().unwrap());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn capture_exile(memory: &impl Memory, countries: &engine_native::ccountry::Reader<'_, impl Memory>, pointer: usize, tags: &mut Vec<u32>) -> Result<Exile, &'static str> {
    capture_exile_buffer(memory, countries, pointer, engine_native_memory::walk::Buffer::Growing(tags))
}

pub fn capture_exile_buffer(
    memory: &impl Memory,
    countries: &engine_native::ccountry::Reader<'_, impl Memory>,
    pointer: usize,
    mut tags: engine_native_memory::walk::Buffer<'_, u32>,
) -> Result<Exile, &'static str> {
    let template =
        u64::from_le_bytes(bytes::<8>(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    if template == 0 {
        return Ok(Exile::Absent);
    }
    let operands = bytes::<16>(memory, template, TEMPLATE_COUNTRY_FALLBACK)?;
    let word = |at: usize| i32::from_le_bytes(operands[at..at + 4].try_into().unwrap());
    let mut row = ExileRow {
        country_fallback: word(0),
        country: word(TEMPLATE_COUNTRY - TEMPLATE_COUNTRY_FALLBACK),
        origin_type: word(TEMPLATE_ORIGIN_TYPE - TEMPLATE_COUNTRY_FALLBACK),
        ..ExileRow::default()
    };
    if row.origin_type != EXILE_ORIGIN {
        return Ok(Exile::Absent);
    }
    let limit = countries.count() as usize;
    let country = if row.country >= 1 {
        row.country
    } else {
        row.country_fallback
    } as u32;
    let province = u64::from_le_bytes(bytes::<8>(memory, pointer, cunit::LOCATION)?) as usize;
    let state = if province == 0 {
        0
    } else {
        u64::from_le_bytes(bytes::<8>(memory, province, engine_native::cprovince::STATE)?) as usize
    };
    let cores = if state == 0 {
        Array { data: 0, count: 0 }
    } else {
        country_list(
            memory,
            state
                .checked_add(engine_native::cstate::CORES)
                .ok_or("exile core list address overflow")?,
            limit,
        )?
    };
    let object = countries.resolve(country)?;
    let status = u64::from_le_bytes(bytes::<8>(
        memory,
        object.pointer,
        engine_native::ccountry::DIPLOMACY_STATUS,
    )?) as usize;
    if status == 0 {
        return Err("exile country diplomacy status missing");
    }
    let occupiers = country_list(
        memory,
        status
            .checked_add(OCCUPIER_SET)
            .ok_or("exile occupier set address overflow")?,
        limit,
    )?;
    let (core_count, occupier_count) = (cores.count as usize, occupiers.count as usize);
    let required = core_count + occupier_count;
    if !tags.ensure(required)? {
        return Ok(Exile::DoesNotFit { required });
    }
    copy_tags(memory, cores, &mut tags[..core_count])?;
    copy_tags(memory, occupiers, &mut tags[core_count..required])?;
    row.cores = (0, core_count);
    row.occupiers = (core_count, occupier_count);
    Ok(Exile::Row(row))
}

pub fn capture_modifiers_buffer(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
    mut modifiers: engine_native_memory::walk::Buffer<'_, Modifier>,
) -> Result<usize, &'static str> {
    let identity = bytes::<8>(memory, pointer, 0)?;
    let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
    if pointer == 0
        || expected_handle == 0
        || u64::from_le_bytes(handle) != expected_handle
        || Some(u64::from_le_bytes(identity))
            != engine_sites::at(base, carmy::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat division identity mismatch");
    }
    let header = bytes::<16>(memory, pointer, cunit::COMBAT_MODIFIER_DATA)?;
    let array =
        Array::from_header(header, 24).ok_or("land combat division modifier header invalid")?;
    let count = array.count as usize;
    if !modifiers.ensure(count)? {
        return Err("land combat division modifier arena full");
    }
    let mut batch = [0_u8; 64 * 24];
    for (index, output) in modifiers[..count].chunks_mut(64).enumerate() {
        let source = array
            .data
            .checked_add(index * batch.len())
            .ok_or("land combat division modifier address overflow")?;
        if !read_slice(memory, source, &mut batch[..output.len() * 24]) {
            return Err("land combat division modifiers unreadable");
        }
        for (output, input) in output.iter_mut().zip(batch.chunks_exact(24)) {
            *output = Modifier {
                id: u32::from_le_bytes(input[..4].try_into().unwrap()),
                value_a: i64::from_le_bytes(input[8..16].try_into().unwrap()),
                value_b: i64::from_le_bytes(input[16..24].try_into().unwrap()),
            };
        }
    }

    Ok(count)
}
#[cfg(test)]
pub(crate) fn capture_context(memory: &impl Memory, base: usize, countries: &engine_native::ccountry::Reader<'_, impl Memory>, sources: &mut crate::combat_sources::Sources, pointer: usize, expected_handle: u64, terms: &mut Vec<WidthTerm>) -> Result<ContextRow, &'static str> {
    capture_context_buffer(memory, base, countries, sources, pointer, expected_handle, engine_native_memory::walk::Buffer::Growing(terms))
}


pub fn capture_context_buffer(
    memory: &impl Memory,
    base: usize,
    countries: &engine_native::ccountry::Reader<'_, impl Memory>,
    sources: &mut crate::combat_sources::Sources,
    pointer: usize,
    expected_handle: u64,
    mut terms: engine_native_memory::walk::Buffer<'_, WidthTerm>,
) -> Result<ContextRow, &'static str> {
        use crate::combat_sources::share;
    let verified = sources
        .verified
        .filter(|(verified, handle, _)| *verified == pointer && *handle == expected_handle);
    if verified.is_none() {
        let identity = bytes::<8>(memory, pointer, 0)?;
        let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
        if expected_handle == 0
            || u64::from_le_bytes(handle) != expected_handle
            || Some(u64::from_le_bytes(identity))
                != engine_sites::at(base, carmy::VTABLE_RVA).map(|p| p as u64)
        {
            return Err("land combat division context identity mismatch");
        }
    }
    let owners = bytes::<8>(memory, pointer, cunit::ROSTER_OWNER)?;
    let tags = [
        u32::from_le_bytes(owners[..4].try_into().unwrap()),
        u32::from_le_bytes(owners[4..].try_into().unwrap()),
    ];
    let owner = if tags[1] as i32 > 0 { tags[1] } else { tags[0] };
    let country = countries.resolve(owner)?;
    let country_accumulator = country
        .pointer
        .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
        .ok_or("land combat context country address overflow")?;
    let country_modifiers = *share(&mut sources.context_modifiers, country_accumulator, || {
        context_modifiers(memory, country_accumulator)
    })?;
    let leader = sources.leader_chain(memory, base, pointer, expected_handle)?;
    let leader_accumulator = leader.unwrap_or(0);
    let leader_modifier_kind = if let Some(accumulator) = leader {
        let group = u64::from_le_bytes(bytes(memory, pointer, carmy::ORDERS_GROUP)?) as usize;
        let leader_pointer =
            u64::from_le_bytes(bytes(memory, group, engine_native::corders_group::LEADER)?) as usize;
        if leader_pointer.checked_add(0x418) == Some(accumulator) {
            1
        } else if leader_pointer.checked_add(0x4d8) == Some(accumulator) {
            2
        } else {
            return Err("land combat contextual leader source mismatch");
        }
    } else {
        0
    };
    let leader_modifiers = if let Some(accumulator) = leader {
        *share(&mut sources.context_modifiers, accumulator, || {
            context_modifiers(memory, accumulator)
        })?
    } else {
        [ContextModifier::default(); 3]
    };
    let ContextMetadata {
        modifier_metadata,
        eligibility_callback,
        feature_flags,
    } = (*sources
        .context_metadata
        .get_or_insert_with(|| context_metadata(memory, base)))?;
    let stat_pointer = match verified {
        Some((_, _, stat)) => stat,
        None => {
            let stat_pointer =
                u64::from_le_bytes(bytes(memory, pointer, cunit::STAT_BLOCK)?) as usize;
            let stat_identity = bytes::<8>(memory, stat_pointer, 0)?;
            if Some(u64::from_le_bytes(stat_identity))
                != engine_sites::at(base, csub_unit_definition::VTABLE_RVA).map(|p| p as u64)
            {
                return Err("land combat context stat identity mismatch");
            }
            stat_pointer
        }
    };
    let base_speed = i64::from_le_bytes(bytes(memory, stat_pointer, 0x290)?);
    let template_pointer =
        u64::from_le_bytes(bytes(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    let mut template_handle = 0;
    let mut count = 0;
    if template_pointer != 0 {
        let width = *share(&mut sources.widths, template_pointer, || {
            template_width(memory, base, template_pointer, &mut sources.width_arena)
        })?;
        template_handle = width.handle;
        if !terms.ensure(width.count)? {
            return Err("land combat width term arena full");
        }
        let run = width.terms?;
        terms[..run.len].copy_from_slice(run.of(&sources.width_arena));
        count = run.len;
    }
    Ok(ContextRow {
        template: template_handle,
        leader_modifier_kind,
        country_modifiers,
        leader_modifiers,
        modifier_metadata,
        feature_flags,
        eligibility_callback,
        base_speed,
        width_term_count: count,
        owner,
        country_accumulator,
        leader_accumulator,
    })
}

fn context_metadata(memory: &impl Memory, base: usize) -> Result<ContextMetadata, &'static str> {
    use engine_native_parts::modifier_definition as metadata;
    let table = u64::from_le_bytes(global_bytes(memory, base, metadata::TABLE_RVA)?) as usize;
    if table == 0 {
        return Err("land combat context metadata table null");
    }
    let mut metadata_bytes = [[0; 12]; 3];
    copy_input_spans(
        memory,
        CONTEXT_MODIFIER_KEYS.map(|key| {
            table
                .checked_add(key as usize * metadata::STRIDE)
                .ok_or("land combat context metadata address overflow")
                .and_then(|row| {
                    row.checked_add(metadata::FLAGS)
                        .ok_or("land combat division inputs unreadable")
                })
        }),
        &mut metadata_bytes.each_mut().map(|raw| raw.as_mut_slice()),
    )?;
    let modifier_metadata = metadata_bytes.map(|raw| {
        [
            u32::from_le_bytes(raw[..4].try_into().unwrap()),
            u32::from_le_bytes(raw[8..].try_into().unwrap()),
        ]
    });
    let callback = u64::from_le_bytes(global_bytes(
        memory,
        base,
        metadata::ELIGIBILITY_CALLBACK_RVA,
    )?);
    let eligibility_callback = if callback == 0 {
        0
    } else if Some(callback)
        == engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA).map(|p| p as u64)
    {
        1
    } else {
        return Err("land combat context modifier callback unsupported");
    };
    let feature_flags = u32::from_le_bytes(global_bytes(
        memory,
        base,
        metadata::TRADE_FEATURE_FLAGS_RVA,
    )?);
    Ok(ContextMetadata {
        modifier_metadata,
        eligibility_callback,
        feature_flags,
    })
}

fn template_width(
    memory: &impl Memory,
    base: usize,
    template_pointer: usize,
    arena: &mut Vec<WidthTerm>,
) -> Result<TemplateWidth, &'static str> {
    use engine_native::creferenced_division_template as template;
    let template_identity = bytes::<16>(memory, template_pointer, 0)?;
    let handle = u64::from_le_bytes(template_identity[8..].try_into().unwrap());
    if Some(u64::from_le_bytes(
        template_identity[..8].try_into().unwrap(),
    )) != engine_sites::at(base, template::VTABLE_RVA).map(|p| p as u64)
        || handle as u32 != template::REFERENCE_TYPE
        || handle >> 32 == 0
    {
        return Err("land combat width template identity mismatch");
    }
    let term_header = bytes(memory, template_pointer, 0xc0)?;
    let array =
        Array::from_header(term_header, 16).ok_or("land combat width term header invalid")?;
    let count = array.count as usize;
    let start = arena.len();
    let terms = (|| {
        engine_allocation::reserve_push_batch(arena, count).map_err(|_| "allocation failed")?;
        let mut packed = [0; 64 * 16];
        for first in (0..count).step_by(64) {
            let batch = (count - first).min(64);
            let address = array
                .element(first as u32, 16)
                .ok_or("land combat width term address overflow")?;
            if !read_slice(memory, address, &mut packed[..batch * 16]) {
                return Err("land combat width terms unreadable");
            }
            for input in packed[..batch * 16].chunks_exact(16) {
                let subunit = u64::from_le_bytes(input[..8].try_into().unwrap()) as usize;
                let raw = bytes::<12>(memory, subunit, 0)?;
                if subunit == 0
                    || Some(u64::from_le_bytes(raw[..8].try_into().unwrap()))
                        != engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
                            .map(|p| p as u64)
                {
                    return Err("land combat width subunit identity mismatch");
                }
                arena.push(WidthTerm {
                    subunit: u32::from_le_bytes(raw[8..].try_into().unwrap()),
                    count: i32::from_le_bytes(input[8..12].try_into().unwrap()),
                    width: i64::from_le_bytes(bytes(memory, subunit, 0x58)?),
                });
            }
        }
        Ok(crate::combat_sources::Run { start, len: count })
    })();
    Ok(TemplateWidth {
        handle,
        count,
        terms,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextMetadata {
    modifier_metadata: [[u32; 2]; 3],
    eligibility_callback: u8,
    feature_flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TemplateWidth {
    handle: u64,
    count: usize,
    terms: Result<crate::combat_sources::Run, &'static str>,
}

fn context_modifiers(
    memory: &impl Memory,
    pointer: usize,
) -> Result<[ContextModifier; 3], &'static str> {
    let header = bytes::<16>(memory, pointer, 0)?;
    let array =
        Array::from_header(header, 16).ok_or("land combat context modifier header invalid")?;
    let mut output = [ContextModifier::default(); 3];
    let mut packed = [0; 64 * 16];
    let mut previous = 0;
    for first in (0..array.count as usize).step_by(64) {
        let count = (array.count as usize - first).min(64);
        let address = array
            .data
            .checked_add(first * 16)
            .ok_or("land combat context modifier address overflow")?;
        if !read_slice(memory, address, &mut packed[..count * 16]) {
            return Err("land combat context modifiers unreadable");
        }
        for input in packed[..count * 16].chunks_exact(16) {
            let key = u32::from_le_bytes(input[..4].try_into().unwrap());
            if key <= previous {
                return Err("land combat context modifier keys not increasing");
            }
            previous = key;
            if let Some(index) = CONTEXT_MODIFIER_KEYS
                .iter()
                .position(|wanted| *wanted == key)
            {
                output[index] = ContextModifier {
                    raw: i64::from_le_bytes(input[8..].try_into().unwrap()),
                    present: 1,
                };
            }
        }
    }
    Ok(output)
}

fn commander(memory: &impl Memory, base: usize, group: usize) -> Result<u64, &'static str> {
    if group == 0 {
        return Ok(0);
    }
    let (mut group_identity, mut leader) = ([0_u8; 16], [0_u8; 8]);
    let copied = memory.copy_scattered_prefetched(
        [
            group,
            group.checked_add(engine_native::corders_group::LEADER).unwrap_or(0),
        ],
        &mut [&mut group_identity, &mut leader],
    );
    if copied == 0 {
        return Err("land combat division inputs unreadable");
    }
    let vtable = u64::from_le_bytes(group_identity[..8].try_into().unwrap());
    let group_handle = u64::from_le_bytes(group_identity[8..].try_into().unwrap());
    if engine_sites::which(
        base,
        vtable as usize,
        &[
            engine_native::corders_group::VTABLE_RVA,
            engine_native_parts::carmy_group::VTABLE_RVA,
        ],
    )
    .is_none()
        || group_handle as u32 != engine_native::corders_group::REFERENCE_TYPE
        || group_handle >> 32 == 0
    {
        return Err("land combat division group identity mismatch");
    }
    if copied != 2 {
        return Err("land combat division inputs unreadable");
    }
    let leader = u64::from_le_bytes(leader) as usize;
    if leader == 0 {
        return Ok(0);
    }
    let identity = bytes::<16>(memory, leader, 0)?;
    let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        != engine_sites::at(base, engine_native::carmy_leader::VTABLE_RVA).map(|value| value as u64)
        || handle as u32 != engine_native::cunit_leader::REFERENCE_TYPE
        || handle >> 32 == 0
    {
        return Err("land combat division commander identity mismatch");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests;
