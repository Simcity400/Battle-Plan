#![allow(dead_code, clippy::all)]

use crate::{Memory, carmy, cpdx_array::Array, csub_unit_definition, cunit, read, read_slice};

pub const MODIFIER_LIMIT: usize = 256;
pub const ARMOUR: usize = 0x298;
pub const AIR_ATTACK: usize = 0x220;
pub const WIDTH_TERM_LIMIT: usize = 64;
pub const NAME_GROUP_MEMBER: usize = 0x340;
pub const DIRECTION_GROUP: usize = 0x4a8;
pub const CONTEXT_MODIFIER_KEYS: [u32; 3] = [0xfb, 0x10f, 0x250];

pub use crate::division_combat_inputs::{
    ContextModifier,
    ContextRow,
    Modifier,
    Row,
    WidthTerm,
};

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
    pointer: usize,
    expected_handle: u64,
    groups: &mut crate::cname_group_member::GroupKeys,
    names: &mut crate::cname_group_member::Names<'_>,
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
            != engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
                .map(|p| p as u64)
    {
        return Err("land combat division stat identity mismatch");
    }
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
    groups: &mut crate::cname_group_member::GroupKeys,
    names: &mut crate::cname_group_member::Names<'_>,
) -> Result<Option<crate::cname_group_member::Inputs>, &'static str> {
    use crate::cname_group_member::Naming;
    let member = usize::try_from(member)
        .ok()
        .filter(|p| *p != 0)
        .ok_or("land combat division name pointer invalid")?;
    Ok(
        match crate::cname_group_member::inputs(memory, base, member, groups, names)? {
            Naming::Row(row) => Some(row),
            Naming::DoesNotFit => None,
        },
    )
}

pub fn capture_modifiers(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
    modifiers: &mut [Modifier],
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
    if count > MODIFIER_LIMIT {
        return Err("land combat division modifier bound exceeded");
    }
    if count > modifiers.len() {
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

#[derive(Default)]
pub struct ContextModifiers {
    country: Option<(usize, [ContextModifier; 3])>,
    leader: Option<(usize, [ContextModifier; 3])>,
}

pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    countries: &crate::ccountry::Reader<'_, impl Memory>,
    pointer: usize,
    expected_handle: u64,
    terms: &mut [WidthTerm],
    modifiers: &mut ContextModifiers,
) -> Result<ContextRow, &'static str> {
    use crate::{
        ccountry, creferenced_division_template as template, modifier_definition as metadata,
    };
    let identity = bytes::<8>(memory, pointer, 0)?;
    let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
    if expected_handle == 0
        || u64::from_le_bytes(handle) != expected_handle
        || Some(u64::from_le_bytes(identity))
            != engine_sites::at(base, carmy::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat division context identity mismatch");
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
        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
        .ok_or("land combat context country address overflow")?;
    let country_modifiers = context_modifiers(memory, country_accumulator, &mut modifiers.country)?;
    let leader = crate::carmy_leader::movement_accumulator(memory, base, pointer)?;
    let leader_accumulator = leader.unwrap_or(0);
    let leader_modifier_kind = if let Some(accumulator) = leader {
        let group = u64::from_le_bytes(bytes(memory, pointer, carmy::ORDERS_GROUP)?) as usize;
        let leader_pointer =
            u64::from_le_bytes(bytes(memory, group, crate::corders_group::LEADER)?) as usize;
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
        context_modifiers(memory, accumulator, &mut modifiers.leader)?
    } else {
        [ContextModifier::default(); 3]
    };
    let table = u64::from_le_bytes(bytes(memory, base, metadata::TABLE_RVA.0)?) as usize;
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
    let callback = u64::from_le_bytes(bytes(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA.0)?);
    let eligibility_callback = if callback == 0 {
        0
    } else if Some(callback)
        == engine_sites::at(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA)
            .map(|p| p as u64)
    {
        1
    } else {
        return Err("land combat context modifier callback unsupported");
    };
    let feature_flags =
        u32::from_le_bytes(bytes(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA.0)?);
    let stat_pointer = u64::from_le_bytes(bytes(memory, pointer, cunit::STAT_BLOCK)?) as usize;
    let stat_identity = bytes::<8>(memory, stat_pointer, 0)?;
    if Some(u64::from_le_bytes(stat_identity))
        != engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
            .map(|p| p as u64)
    {
        return Err("land combat context stat identity mismatch");
    }
    let base_speed = i64::from_le_bytes(bytes(memory, stat_pointer, 0x290)?);
    let template_pointer =
        u64::from_le_bytes(bytes(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    let mut template_handle = 0;
    let mut count = 0;
    if template_pointer != 0 {
        let template_identity = bytes::<16>(memory, template_pointer, 0)?;
        template_handle = u64::from_le_bytes(template_identity[8..].try_into().unwrap());
        if Some(u64::from_le_bytes(
            template_identity[..8].try_into().unwrap(),
        )) != engine_sites::at(base, template::VTABLE_RVA).map(|p| p as u64)
            || template_handle as u32 != template::REFERENCE_TYPE
            || template_handle >> 32 == 0
        {
            return Err("land combat width template identity mismatch");
        }
        let term_header = bytes(memory, template_pointer, 0xc0)?;
        let array =
            Array::from_header(term_header, 16).ok_or("land combat width term header invalid")?;
        count = array.count as usize;
        if count > WIDTH_TERM_LIMIT {
            return Err("land combat width term bound exceeded");
        }
        if count > terms.len() {
            return Err("land combat width term arena full");
        }
        let mut packed = [0; WIDTH_TERM_LIMIT * 16];
        if !read_slice(memory, array.data, &mut packed[..count * 16]) {
            return Err("land combat width terms unreadable");
        }
        for (output, input) in terms[..count].iter_mut().zip(packed.chunks_exact(16)) {
            let subunit = u64::from_le_bytes(input[..8].try_into().unwrap()) as usize;
            let raw = bytes::<12>(memory, subunit, 0)?;
            if subunit == 0
                || Some(u64::from_le_bytes(raw[..8].try_into().unwrap()))
                    != engine_sites::at(base, csub_unit_definition::VTABLE_RVA)
                        .map(|p| p as u64)
            {
                return Err("land combat width subunit identity mismatch");
            }
            *output = WidthTerm {
                subunit: u32::from_le_bytes(raw[8..].try_into().unwrap()),
                count: i32::from_le_bytes(input[8..12].try_into().unwrap()),
                width: i64::from_le_bytes(bytes(memory, subunit, 0x58)?),
            };
        }
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

fn context_modifiers(
    memory: &impl Memory,
    pointer: usize,
    retained: &mut Option<(usize, [ContextModifier; 3])>,
) -> Result<[ContextModifier; 3], &'static str> {
    if let Some((address, values)) = *retained
        && address == pointer
    {
        return Ok(values);
    }
    let header = bytes::<16>(memory, pointer, 0)?;
    let array =
        Array::from_header(header, 16).ok_or("land combat context modifier header invalid")?;
    if array.count > 4096 {
        return Err("land combat context modifier bound exceeded");
    }
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
    *retained = Some((pointer, output));
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
            group.checked_add(crate::corders_group::LEADER).unwrap_or(0),
        ],
        &mut [&mut group_identity, &mut leader],
    );
    if copied == 0 {
        return Err("land combat division inputs unreadable");
    }
    let vtable = u64::from_le_bytes(group_identity[..8].try_into().unwrap());
    let group_handle = u64::from_le_bytes(group_identity[8..].try_into().unwrap());
    if ![
        crate::corders_group::VTABLE_RVA,
        crate::carmy_group::VTABLE_RVA,
    ]
    .iter()
    .any(|&key| engine_sites::at(base, key).map(|value| value as u64) == Some(vtable))
        || group_handle as u32 != crate::corders_group::REFERENCE_TYPE
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
        != engine_sites::at(base, crate::carmy_leader::VTABLE_RVA)
            .map(|value| value as u64)
        || handle as u32 != crate::cunit_leader::REFERENCE_TYPE
        || handle >> 32 == 0
    {
        return Err("land combat division commander identity mismatch");
    }
    Ok(handle)
}
