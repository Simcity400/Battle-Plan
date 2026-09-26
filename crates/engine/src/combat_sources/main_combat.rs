//! TEST ONLY: main's carmy_combat_inputs readers,
//! copied verbatim from origin/main c53208146b, except that its row types
//! are the current module's (they are unchanged), so outputs compare
//! directly. The sharing equivalence tests run this as the baseline:
//! every read in main's order, every failure with main's reason.
#![allow(dead_code, clippy::all)]

use crate::{Memory, carmy, cpdx_array::Array, csub_unit_definition, cunit, read, read_slice};

/// Explicit bound, exceeding it makes this division unavailable.
pub const MODIFIER_LIMIT: usize = 256;
/// Stat-block armour and piercing operands, consumed together by 1415CA660.
pub const ARMOUR: usize = 0x298;
/// Anti-air operand passed to 141297FD0 by 1415CA660.
pub const AIR_ATTACK: usize = 0x220;
/// Width getter140C6E270 traverses one term per native subunit type/count.
pub const WIDTH_TERM_LIMIT: usize = 64;
pub const MANPOWER_TERM_LIMIT: usize = 64;
/// CArmy primary virtual+68 ->140C72F00 returns *(army+340)+60.
/// Committed1415BA240 and reserve1415BAC10 bind this to their name widget.
pub const DISPLAY_NAME_OWNER: usize = 0x340;
pub const DISPLAY_NAME: usize = 0x60;
/// Raw contextual keys used by width140C6E270 and reinforcement140C6D610.
pub const CONTEXT_MODIFIER_KEYS: [u32; 3] = [0xfb, 0x10f, 0x250];
/// Reinforcement width penalty/cap, base chance, initiative/speed coefficients,
/// and speed baseline:140C6D610 and140C76E90.
pub const CONTEXT_DEFINE_RVAS: [usize; 8] = [
    0x0331_ccf8,
    0x0331_cd90,
    0x0331_a260,
    0x0331_dd20,
    0x0331_a310,
    0x0331_e020,
    0x0293_c980,
    0x0293_c978,
];

pub use crate::carmy_combat_inputs::{
    ContextModifier, ContextRow, ManpowerTerm, Modifier, Row, WidthTerm,
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

/// Keep checked-address errors at their original ordinal, after every readable
/// prefix span. No failed scatter is retried, including a partially copied field.
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

/// The caller supplies the authenticated combat/side/role membership.
/// CArmy, stat-block and formation leader identities are authenticated on entry.
pub fn capture_stats(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
) -> Result<Row, &'static str> {
    let identity = bytes::<8>(memory, pointer, 0)?;
    let expected_vtable = base
        .checked_add(carmy::VTABLE_RVA)
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
            != base
                .checked_add(csub_unit_definition::VTABLE_RVA)
                .map(|p| p as u64)
    {
        return Err("land combat division stat identity mismatch");
    }
    let (mut current, mut entrenchment, mut maxima, mut armour, mut group_pointer) =
        ([0; 24], [0; 16], [0; 16], [0; 16], [0; 8]);
    copy_input_spans(
        memory,
        [
            (pointer, carmy::STRENGTH),
            (pointer, carmy::ENTRENCHMENT),
            (stat, csub_unit_definition::MAXIMUM_ORGANISATION),
            (stat, ARMOUR),
            (pointer, carmy::ORDERS_GROUP),
        ]
        .map(|(p, offset)| {
            p.checked_add(offset)
                .ok_or("land combat division inputs unreadable")
        }),
        &mut [
            &mut current,
            &mut entrenchment,
            &mut maxima,
            &mut armour,
            &mut group_pointer,
        ],
    )?;
    let current = values::<3>(&current);
    let entrenchment = values::<2>(&entrenchment);
    let maxima = values::<2>(&maxima);
    let armour = values::<2>(&armour);
    let group = u64::from_le_bytes(group_pointer) as usize;
    let commander = commander(memory, base, group)?;
    let name = display_name(memory, pointer)?;
    let (mut strength_factor, mut fuel, mut fuel_capacity, mut fuel_define, mut retreat) =
        ([0; 8], [0; 8], [0; 8], [0; 8], [0; 1]);
    let (mut stats, mut air_attack, mut reconnaissance, mut initiative, mut scales) =
        ([0; 40], [0; 8], [0; 8], [0; 8], [0; 32]);
    copy_input_spans(
        memory,
        [
            (pointer, carmy::STRENGTH_FACTOR),
            (pointer, 0x620),
            (stat, 0x120),
            (base, 0x0331_97d0),
            (pointer, carmy::RETREAT),
            (stat, csub_unit_definition::DEFENSE),
            (stat, AIR_ATTACK),
            (stat, 0xd0),
            (stat, 0xe0),
            (pointer, cunit::ATTACK_ACCUMULATOR),
        ]
        .map(|(p, offset)| {
            p.checked_add(offset)
                .ok_or("land combat division inputs unreadable")
        }),
        &mut [
            &mut strength_factor,
            &mut fuel,
            &mut fuel_capacity,
            &mut fuel_define,
            &mut retreat,
            &mut stats,
            &mut air_attack,
            &mut reconnaissance,
            &mut initiative,
            &mut scales,
        ],
    )?;
    let row = Row {
        division: expected_handle,
        name,
        strength: current[0],
        strength_combat_factor: i64::from_le_bytes(strength_factor),
        fuel_inputs: [
            i64::from_le_bytes(fuel),
            i64::from_le_bytes(fuel_capacity),
            i64::from_le_bytes(fuel_define),
        ],
        organisation: current[1],
        experience: current[2],
        entrenchment: entrenchment[0],
        entrenchment_cap: entrenchment[1],
        retreat: retreat[0],
        stat_values: values(&stats),
        max_organisation: maxima[0],
        max_strength: maxima[1],
        armour: armour[0],
        piercing: armour[1],
        air_attack: i64::from_le_bytes(air_attack),
        reconnaissance: i64::from_le_bytes(reconnaissance),
        initiative: i64::from_le_bytes(initiative),
        commander,
        scales: values(&scales),
    };
    Ok(row)
}

fn display_name(
    memory: &impl Memory,
    pointer: usize,
) -> Result<crate::msvc_string::Text<256>, &'static str> {
    let owner = bytes::<8>(memory, pointer, DISPLAY_NAME_OWNER)?;
    let source = usize::try_from(u64::from_le_bytes(owner))
        .ok()
        .filter(|p| *p != 0)
        .and_then(|p| p.checked_add(DISPLAY_NAME))
        .ok_or("land combat division name pointer invalid")?;
    let descriptor = crate::msvc_string::Descriptor::read(memory, source).map_err(|reason| {
        if reason == "string header unreadable" {
            "land combat division inputs unreadable"
        } else {
            reason
        }
    })?;
    let mut result = crate::msvc_string::Text {
        len: descriptor.len,
        ..Default::default()
    };
    descriptor.copy(memory, &mut result.bytes)?;
    Ok(result)
}

/// Independent modifier acquisition: no stat-block or commander dependency.
/// Arena pressure fails before copying any modifier; publication is all or none.
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
            != base.checked_add(carmy::VTABLE_RVA).map(|p| p as u64)
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

/// Label tooltip1415BC060 sums all signed manpower amounts through140C5A040 /
/// 140C5A0F0. Preserve original terms; do not reduce them to a derived total.
pub fn capture_manpower(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
    terms: &mut [ManpowerTerm],
) -> Result<usize, &'static str> {
    let identity = bytes::<8>(memory, pointer, 0)?;
    let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
    if expected_handle == 0
        || u64::from_le_bytes(handle) != expected_handle
        || Some(u64::from_le_bytes(identity))
            != base.checked_add(carmy::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat manpower division identity mismatch");
    }
    let container_identity = bytes::<8>(memory, pointer, 0x3d8)?;
    if Some(u64::from_le_bytes(container_identity))
        != base
            .checked_add(crate::carmy_manpower_values::VTABLE_RVA)
            .map(|p| p as u64)
    {
        return Err("land combat manpower container identity mismatch");
    }
    let header = bytes::<16>(memory, pointer, carmy::MANPOWER_DATA)?;
    let array = Array::from_header(header, 8).ok_or("land combat manpower header invalid")?;
    let count = array.count as usize;
    if count > MANPOWER_TERM_LIMIT {
        return Err("land combat manpower term bound exceeded");
    }
    if count > terms.len() {
        return Err("land combat manpower arena full");
    }
    let mut packed = [0; MANPOWER_TERM_LIMIT * 8];
    if !read_slice(memory, array.data, &mut packed[..count * 8]) {
        return Err("land combat manpower terms unreadable");
    }
    for (output, input) in terms[..count].iter_mut().zip(packed.chunks_exact(8)) {
        *output = ManpowerTerm {
            source: u32::from_le_bytes(input[..4].try_into().unwrap()),
            amount: i32::from_le_bytes(input[4..].try_into().unwrap()),
        };
    }
    Ok(count)
}

/// Last complete country and leader modifier inputs within one acquisition
/// pass. Construct a fresh value for every pass; no observation crosses hours.
/// Each slot keeps only the three raw values and presence bits, never a partial
/// census or error. Different accumulator addresses take the ordinary full walk.
#[derive(Default)]
pub struct ContextModifiers {
    country: Option<(usize, [ContextModifier; 3])>,
    leader: Option<(usize, [ContextModifier; 3])>,
}

/// Width composition and contextual reinforcement operands. Terms preserve
/// native order, signed multiplicities and unscaled definition widths. No
/// width, chance, modifier clamp or speed bonus is calculated here.
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    state: usize,
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
            != base.checked_add(carmy::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat division context identity mismatch");
    }
    let owners = bytes::<8>(memory, pointer, cunit::ROSTER_OWNER)?;
    let tags = [
        u32::from_le_bytes(owners[..4].try_into().unwrap()),
        u32::from_le_bytes(owners[4..].try_into().unwrap()),
    ];
    let owner = if tags[1] as i32 > 0 { tags[1] } else { tags[0] };
    let countries = ccountry::Reader::new(memory, base, state)?;
    let country = countries.resolve(owner)?;
    let country_modifiers = context_modifiers(
        memory,
        country
            .pointer
            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
            .ok_or("land combat context country address overflow")?,
        &mut modifiers.country,
    )?;
    let leader = crate::carmy_leader::movement_accumulator(memory, base, pointer)?;
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
    let table = u64::from_le_bytes(bytes(memory, base, metadata::TABLE_RVA)?) as usize;
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
    let callback = u64::from_le_bytes(bytes(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA)?);
    let eligibility_callback = if callback == 0 {
        0
    } else if Some(callback)
        == base
            .checked_add(metadata::ELIGIBILITY_FLAGS_GETTER_RVA)
            .map(|p| p as u64)
    {
        1
    } else {
        return Err("land combat context modifier callback unsupported");
    };
    let feature_flags = u32::from_le_bytes(bytes(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA)?);
    let stat_pointer = u64::from_le_bytes(bytes(memory, pointer, cunit::STAT_BLOCK)?) as usize;
    let stat_identity = bytes::<8>(memory, stat_pointer, 0)?;
    if Some(u64::from_le_bytes(stat_identity))
        != base
            .checked_add(csub_unit_definition::VTABLE_RVA)
            .map(|p| p as u64)
    {
        return Err("land combat context stat identity mismatch");
    }
    let base_speed = i64::from_le_bytes(bytes(memory, stat_pointer, 0x290)?);
    let mut defines = [[0; 8]; 8];
    copy_input_spans(
        memory,
        CONTEXT_DEFINE_RVAS.map(|offset| {
            base.checked_add(offset)
                .ok_or("land combat division inputs unreadable")
        }),
        &mut defines.each_mut().map(|raw| raw.as_mut_slice()),
    )?;
    let defines = defines.map(i64::from_le_bytes);
    let template_pointer =
        u64::from_le_bytes(bytes(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    let mut template_handle = 0;
    let mut count = 0;
    if template_pointer != 0 {
        let template_identity = bytes::<16>(memory, template_pointer, 0)?;
        template_handle = u64::from_le_bytes(template_identity[8..].try_into().unwrap());
        if Some(u64::from_le_bytes(
            template_identity[..8].try_into().unwrap(),
        )) != base.checked_add(template::VTABLE_RVA).map(|p| p as u64)
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
                    != base
                        .checked_add(csub_unit_definition::VTABLE_RVA)
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
        effective_owner: owner,
        leader_modifier_kind,
        country_modifiers,
        leader_modifiers,
        modifier_metadata,
        feature_flags,
        eligibility_callback,
        base_speed,
        defines,
        width_term_count: count,
    })
}

/// Complete bounded accumulator census in64-entry copies, shared across keys.
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
    let group_identity = bytes::<16>(memory, group, 0)?;
    let vtable = u64::from_le_bytes(group_identity[..8].try_into().unwrap());
    let group_handle = u64::from_le_bytes(group_identity[8..].try_into().unwrap());
    if ![
        crate::corders_group::VTABLE_RVA,
        crate::carmy_group::VTABLE_RVA,
    ]
    .iter()
    .any(|rva| base.checked_add(*rva).map(|value| value as u64) == Some(vtable))
        || group_handle as u32 != crate::corders_group::REFERENCE_TYPE
        || group_handle >> 32 == 0
    {
        return Err("land combat division group identity mismatch");
    }
    let leader = u64::from_le_bytes(bytes(memory, group, crate::corders_group::LEADER)?) as usize;
    if leader == 0 {
        return Ok(0);
    }
    let identity = bytes::<16>(memory, leader, 0)?;
    let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
        != base
            .checked_add(crate::carmy_leader::VTABLE_RVA)
            .map(|value| value as u64)
        || handle as u32 != crate::cunit_leader::REFERENCE_TYPE
        || handle >> 32 == 0
    {
        return Err("land combat division commander identity mismatch");
    }
    Ok(handle)
}
