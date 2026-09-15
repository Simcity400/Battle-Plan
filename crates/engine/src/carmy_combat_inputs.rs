//! Raw CArmy operands consumed by CCombatantEntry::Update (1415C7E00) and
//! its modifier-icon builder (1415CA660). No displayed statistic is composed.

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WidthTerm {
    pub subunit: u32,
    pub count: i32,
    pub width: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManpowerTerm {
    /// Raw first word; source-country meaning is not proven for CArmy instances.
    pub source: u32,
    pub amount: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextModifier {
    pub raw: i64,
    pub present: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    pub template: u64,
    pub effective_owner: u32,
    /// Selected native context source:0 absent,1 leader+408,2 leader+4C8.
    pub leader_modifier_kind: u8,
    pub country_modifiers: [ContextModifier; 3],
    pub leader_modifiers: [ContextModifier; 3],
    /// Per-key raw flags and feature selector, in CONTEXT_MODIFIER_KEYS order.
    pub modifier_metadata: [[u32; 2]; 3],
    pub feature_flags: u32,
    /// Zero:null eligibility callback; one:authenticated1401AE190 flags getter.
    pub eligibility_callback: u8,
    /// Cached movement speed operand; reinforcement applies current terrain etc.
    pub base_speed: i64,
    /// Last two entries: minimum fighting strength/organisation140C684A0.
    pub defines: [i64; 8],
    pub width_term_count: usize,
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
    pub name: crate::msvc_string::Text<256>,
    pub strength: i64,
    /// Committed strength bar multiplies raw CArmy+468 (140C6F8B0).
    pub strength_combat_factor: i64,
    /// Fuel icon140C6F960: CArmy+620,stat+120,define33197D0; all raw fixed5.
    pub fuel_inputs: [i64; 3],
    pub organisation: i64,
    pub experience: i64,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub retreat: u8,
    /// Defense, breakthrough, hardness, soft attack, hard attack; all raw fixed5.
    pub stat_values: [i64; 5],
    pub max_organisation: i64,
    pub max_strength: i64,
    pub armour: i64,
    pub piercing: i64,
    pub air_attack: i64,
    /// Direct stat-block +D0 getter 140BEE1B0, used for initiative.
    pub reconnaissance: i64,
    /// Initiative operand at stat+E0 used by reinforcement getter140C6D610.
    pub initiative: i64,
    /// Full direct formation leader handle; zero if unit has no group/leader.
    pub commander: u64,
    /// Attack accumulator, defense accumulator, attack base, defense base.
    pub scales: [i64; 4],
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

/// The caller supplies the authenticated combat/side/role membership.
/// Terminal checks retain CArmy/stat-block identities and formation leader link.
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
    let current = values::<3>(&bytes::<24>(memory, pointer, carmy::STRENGTH)?);
    let entrenchment = values::<2>(&bytes::<16>(memory, pointer, carmy::ENTRENCHMENT)?);
    let maxima = values::<2>(&bytes::<16>(
        memory,
        stat,
        csub_unit_definition::MAXIMUM_ORGANISATION,
    )?);
    let armour = values::<2>(&bytes::<16>(memory, stat, ARMOUR)?);
    let group_pointer = bytes::<8>(memory, pointer, carmy::ORDERS_GROUP)?;
    let group = u64::from_le_bytes(group_pointer) as usize;
    let commander = commander(memory, base, group)?;
    let row = Row {
        division: expected_handle,
        name: display_name(memory, pointer)?,
        strength: current[0],
        strength_combat_factor: i64::from_le_bytes(bytes(memory, pointer, 0x468)?),
        fuel_inputs: [
            i64::from_le_bytes(bytes(memory, pointer, 0x620)?),
            i64::from_le_bytes(bytes(memory, stat, 0x120)?),
            i64::from_le_bytes(bytes(memory, base, 0x0331_97d0)?),
        ],
        organisation: current[1],
        experience: current[2],
        entrenchment: entrenchment[0],
        entrenchment_cap: entrenchment[1],
        retreat: bytes::<1>(memory, pointer, carmy::RETREAT)?[0],
        stat_values: values(&bytes::<40>(memory, stat, csub_unit_definition::DEFENSE)?),
        max_organisation: maxima[0],
        max_strength: maxima[1],
        armour: armour[0],
        piercing: armour[1],
        air_attack: i64::from_le_bytes(bytes(memory, stat, AIR_ATTACK)?),
        reconnaissance: i64::from_le_bytes(bytes(memory, stat, 0xd0)?),
        initiative: i64::from_le_bytes(bytes(memory, stat, 0xe0)?),
        commander,
        scales: values(&bytes::<32>(memory, pointer, cunit::ATTACK_ACCUMULATOR)?),
    };
    if bytes::<8>(memory, pointer, 0)? != identity
        || bytes::<8>(memory, pointer, cunit::HANDLE)? != handle
        || bytes::<8>(memory, pointer, cunit::STAT_BLOCK)? != stat_pointer
        || bytes::<8>(memory, stat, 0)? != stat_identity
        || bytes::<8>(memory, pointer, carmy::ORDERS_GROUP)? != group_pointer
        || self::commander(memory, base, group)? != commander
    {
        return Err("land combat division identity changed");
    }
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
    let header = bytes::<32>(memory, source, 0)?;
    let descriptor = crate::msvc_string::Descriptor::read(memory, source)?;
    let mut result = crate::msvc_string::Text {
        len: descriptor.len,
        ..Default::default()
    };
    descriptor.copy(memory, &mut result.bytes)?;
    if bytes::<32>(memory, source, 0)? != header
        || bytes::<8>(memory, pointer, DISPLAY_NAME_OWNER)? != owner
    {
        return Err("land combat division name source changed");
    }
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

    if bytes::<8>(memory, pointer, 0)? != identity
        || bytes::<8>(memory, pointer, cunit::HANDLE)? != handle
        || bytes::<16>(memory, pointer, cunit::COMBAT_MODIFIER_DATA)? != header
    {
        return Err("land combat division identity or modifier header changed");
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
    if bytes::<8>(memory, pointer, 0)? != identity
        || bytes::<8>(memory, pointer, cunit::HANDLE)? != handle
        || bytes::<8>(memory, pointer, 0x3d8)? != container_identity
        || bytes::<16>(memory, pointer, carmy::MANPOWER_DATA)? != header
    {
        return Err("land combat manpower identity or header changed");
    }
    Ok(count)
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
        context_modifiers(memory, accumulator)?
    } else {
        [ContextModifier::default(); 3]
    };
    let table = u64::from_le_bytes(bytes(memory, base, metadata::TABLE_RVA)?) as usize;
    if table == 0 {
        return Err("land combat context metadata table null");
    }
    let mut modifier_metadata = [[0; 2]; 3];
    let mut metadata_witness = [[0; 12]; 3];
    for (index, key) in CONTEXT_MODIFIER_KEYS.into_iter().enumerate() {
        let row = table
            .checked_add(key as usize * metadata::STRIDE)
            .ok_or("land combat context metadata address overflow")?;
        let raw = bytes::<12>(memory, row, metadata::FLAGS)?;
        modifier_metadata[index] = [
            u32::from_le_bytes(raw[..4].try_into().unwrap()),
            u32::from_le_bytes(raw[8..].try_into().unwrap()),
        ];
        metadata_witness[index] = raw;
    }
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
    let mut defines = [0; 8];
    for (value, offset) in defines.iter_mut().zip(CONTEXT_DEFINE_RVAS) {
        *value = i64::from_le_bytes(bytes(memory, base, offset)?);
    }
    let template_pointer =
        u64::from_le_bytes(bytes(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    let mut template_handle = 0;
    let mut template_identity = [0; 16];
    let mut term_header = [0; 16];
    let mut count = 0;
    if template_pointer != 0 {
        template_identity = bytes(memory, template_pointer, 0)?;
        template_handle = u64::from_le_bytes(template_identity[8..].try_into().unwrap());
        if Some(u64::from_le_bytes(
            template_identity[..8].try_into().unwrap(),
        )) != base.checked_add(template::VTABLE_RVA).map(|p| p as u64)
            || template_handle as u32 != template::REFERENCE_TYPE
            || template_handle >> 32 == 0
        {
            return Err("land combat width template identity mismatch");
        }
        term_header = bytes(memory, template_pointer, 0xc0)?;
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
            if bytes::<12>(memory, subunit, 0)? != raw {
                return Err("land combat width subunit identity changed");
            }
        }
        let mut terminal = [0; WIDTH_TERM_LIMIT * 16];
        if !read_slice(memory, array.data, &mut terminal[..count * 16])
            || terminal[..count * 16] != packed[..count * 16]
        {
            return Err("land combat width terms changed");
        }
    }
    if bytes::<8>(memory, pointer, 0)? != identity
        || bytes::<8>(memory, pointer, cunit::HANDLE)? != handle
        || bytes::<8>(memory, pointer, cunit::ROSTER_OWNER)? != owners
        || u64::from_le_bytes(bytes(memory, pointer, cunit::STAT_BLOCK)?) as usize != stat_pointer
        || bytes::<8>(memory, stat_pointer, 0)? != stat_identity
        || u64::from_le_bytes(bytes(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize
            != template_pointer
        || countries.resolve(owner)?.pointer != country.pointer
        || crate::carmy_leader::movement_accumulator(memory, base, pointer)? != leader
        || u64::from_le_bytes(bytes(memory, base, metadata::TABLE_RVA)?) as usize != table
        || u64::from_le_bytes(bytes(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA)?) != callback
        || u32::from_le_bytes(bytes(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA)?)
            != feature_flags
        || (template_pointer != 0
            && (bytes::<16>(memory, template_pointer, 0)? != template_identity
                || bytes::<16>(memory, template_pointer, 0xc0)? != term_header))
    {
        return Err("land combat division context identity changed");
    }
    for (index, key) in CONTEXT_MODIFIER_KEYS.into_iter().enumerate() {
        let row = table
            .checked_add(key as usize * metadata::STRIDE)
            .ok_or("land combat context metadata address overflow")?;
        if bytes::<12>(memory, row, metadata::FLAGS)? != metadata_witness[index] {
            return Err("land combat context metadata changed");
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
) -> Result<[ContextModifier; 3], &'static str> {
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
    if bytes::<16>(memory, pointer, 0)? != header {
        return Err("land combat context modifier header changed");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::Cell;

    struct Counted<'a> {
        inner: SliceMemory<'a>,
        copies: Cell<usize>,
        bytes: Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.bytes.set(self.bytes.get() + destination.len());
            self.inner.copy(address, destination)
        }
    }

    fn fixture(count: u32) -> Vec<u8> {
        let mut image = vec![0_u8; 0x5000];
        let mut put = |address: usize, value: u64| {
            image[address..address + 8].copy_from_slice(&value.to_le_bytes());
        };
        put(0x1000, carmy::VTABLE_RVA as u64);
        put(0x1018, 0x1234_0000_0033);
        put(0x1138, 0x2000);
        put(0x1340, 0x3800);
        put(0x3860, 0xff);
        put(0x3870, 1);
        put(0x3878, 15);
        put(0x1170, 0x3000);
        put(0x1178, (count as u64) << 32 | count as u64);
        put(0x2000, csub_unit_definition::VTABLE_RVA as u64);
        put(0x1420, (-123_i64) as u64);
        put(0x2280, 1);
        put(0x3000, 27);
        put(0x3008, (-900_i64) as u64);
        put(0x3010, 800);
        put(0x3018, 2);
        image
    }

    #[test]
    fn preserves_signed_values_order_and_raw_maximum_with_bounded_copy_cost() {
        let mut image = fixture(2);
        image.resize(0x0331_97d8, 0);
        let memory = Counted {
            inner: SliceMemory {
                base: 0,
                bytes: &image,
            },
            copies: Cell::new(0),
            bytes: Cell::new(0),
        };
        let mut modifiers = [Modifier::default(); 2];
        let row = capture_stats(&memory, 0, 0x1000, 0x1234_0000_0033).unwrap();
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 0x1234_0000_0033, &mut modifiers).unwrap(),
            2
        );
        assert_eq!(row.strength, -123);
        assert_eq!(row.name.as_bytes(), &[255]);
        assert_eq!(row.max_organisation, 1);
        assert_eq!(
            modifiers[0],
            Modifier {
                id: 27,
                value_a: -900,
                value_b: 800
            }
        );
        assert_eq!(modifiers[1].id, 2);
        assert_eq!(memory.copies.get(), 36);
        assert_eq!(memory.bytes.get(), 505);
        println!(
            "division scalar/name plus two modifiers: {} guarded copies / {} bytes",
            memory.copies.get(),
            memory.bytes.get()
        );
    }

    #[test]
    fn refuses_modifier_pressure_and_wrong_identity() {
        let image = fixture(2);
        let memory = SliceMemory {
            base: 0,
            bytes: &image,
        };
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 1, &mut []),
            Err("land combat division identity mismatch")
        );
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 0x1234_0000_0033, &mut []),
            Err("land combat division modifier arena full")
        );
        let image = fixture(257);
        let memory = SliceMemory {
            base: 0,
            bytes: &image,
        };
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 0x1234_0000_0033, &mut []),
            Err("land combat division modifier bound exceeded")
        );
    }

    #[test]
    fn bad_stat_identity_does_not_hide_independent_modifiers() {
        let mut image = fixture(2);
        image[0x2000..0x2008].fill(0);
        let memory = SliceMemory {
            base: 0,
            bytes: &image,
        };
        assert_eq!(
            capture_stats(&memory, 0, 0x1000, 0x1234_0000_0033),
            Err("land combat division stat identity mismatch")
        );
        let mut modifiers = [Modifier::default(); 2];
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 0x1234_0000_0033, &mut modifiers),
            Ok(2)
        );
    }

    struct ContextMemory {
        image: Vec<u8>,
        globals: std::collections::BTreeMap<usize, u8>,
        copies: Cell<usize>,
        copied: Cell<usize>,
    }
    impl Memory for ContextMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.copied.set(self.copied.get() + destination.len());
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self
                    .image
                    .get(address + index)
                    .or_else(|| self.globals.get(&(address + index)))
                else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl ContextMemory {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            if address < self.image.len() {
                self.image[address..address + bytes.len()].copy_from_slice(bytes);
            } else {
                for (index, value) in bytes.iter().enumerate() {
                    self.globals.insert(address + index, *value);
                }
            }
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u64(address + 8, (count as u64) << 32 | count as u64);
        }
    }
    fn context_fixture() -> ContextMemory {
        let mut memory = ContextMemory {
            image: fixture(2),
            globals: Default::default(),
            copies: Cell::new(0),
            copied: Cell::new(0),
        };
        memory.image.resize(0x40000, 0);
        memory.u64(0x11d8, 1);
        memory.array(0x5000 + crate::cgame_state::COUNTRIES, 0x6000, 2);
        memory.array(0x5000 + crate::cgame_state::COUNTRY_LINKS, 0x6100, 2);
        memory.u64(0x6008, 0x9000);
        memory.put(0x6104, &1_u32.to_le_bytes());
        memory.u64(0x9000, crate::ccountry::VTABLE_RVA as u64);
        memory.put(0x9008, &1_u32.to_le_bytes());
        memory.array(0x9000 + crate::ccountry::MODIFIER_ACCUMULATOR, 0xa000, 3);
        for (index, key) in CONTEXT_MODIFIER_KEYS.into_iter().enumerate() {
            memory.put(0xa000 + index * 16, &key.to_le_bytes());
            memory.u64(0xa008 + index * 16, (-111_i64 - index as i64) as u64);
        }
        memory.u64(crate::modifier_definition::TABLE_RVA, 0x10000);
        memory.u64(crate::modifier_definition::ELIGIBILITY_CALLBACK_RVA, 0);
        memory.put(
            crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
            &0_u32.to_le_bytes(),
        );
        for (index, offset) in CONTEXT_DEFINE_RVAS.into_iter().enumerate() {
            memory.u64(offset, index as u64 + 1);
        }
        memory.u64(0x13b8, 0xb000);
        memory.u64(
            0xb000,
            crate::creferenced_division_template::VTABLE_RVA as u64,
        );
        memory.u64(0xb008, 1_u64 << 32 | 52);
        memory.array(0xb0c0, 0xc000, 2);
        memory.u64(0xc000, 0xd000);
        memory.u64(0xc010, 0xd000);
        memory.put(0xc008, &3_i32.to_le_bytes());
        memory.put(0xc018, &(-2_i32).to_le_bytes());
        memory.u64(0xd000, csub_unit_definition::VTABLE_RVA as u64);
        memory.put(0xd008, &7_u32.to_le_bytes());
        memory.u64(0xd058, 12345);
        memory
    }

    #[test]
    fn width_context_preserves_order_signed_counts_and_raw_modifiers() {
        let memory = context_fixture();
        let mut terms = [WidthTerm::default(); 2];
        let row =
            capture_context(&memory, 0, 0x5000, 0x1000, 0x1234_0000_0033, &mut terms).unwrap();
        assert_eq!(row.width_term_count, 2);
        assert_eq!(
            terms,
            [
                WidthTerm {
                    subunit: 7,
                    count: 3,
                    width: 12345
                },
                WidthTerm {
                    subunit: 7,
                    count: -2,
                    width: 12345
                }
            ]
        );
        assert_eq!(row.country_modifiers.map(|v| v.raw), [-111, -112, -113]);
        assert_eq!(row.leader_modifiers, [ContextModifier::default(); 3]);
        assert_eq!(row.defines, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(row.template, 1_u64 << 32 | 52);
        assert_eq!(memory.copies.get(), 70);
        assert_eq!(memory.copied.get(), 704);
    }

    #[test]
    fn width_context_reports_short_arena_and_modifier_bound() {
        let mut memory = context_fixture();
        assert_eq!(
            capture_context(&memory, 0, 0x5000, 0x1000, 0x1234_0000_0033, &mut []),
            Err("land combat width term arena full")
        );
        memory.array(0x9000 + crate::ccountry::MODIFIER_ACCUMULATOR, 0xa000, 4097);
        assert_eq!(
            capture_context(&memory, 0, 0x5000, 0x1000, 0x1234_0000_0033, &mut []),
            Err("land combat context modifier bound exceeded")
        );
    }

    #[test]
    fn width_context_selects_authenticated_leader_raw_modifiers() {
        let mut memory = context_fixture();
        memory.u64(0x10c0, 0xe000);
        memory.u64(0xe000, crate::corders_group::VTABLE_RVA as u64);
        memory.u64(0xe008, 1_u64 << 32 | 53);
        memory.u64(0xe088, 0xf000);
        memory.u64(0xf000, crate::carmy_leader::VTABLE_RVA as u64);
        memory.u64(0xf008, 77_u64 << 32 | 4713);
        memory.array(0xf418, 0xa100, 1);
        memory.put(0xa100, &0x10f_u32.to_le_bytes());
        memory.u64(0xa108, (-777_i64) as u64);
        let row = capture_context(
            &memory,
            0,
            0x5000,
            0x1000,
            0x1234_0000_0033,
            &mut [WidthTerm::default(); 2],
        )
        .unwrap();
        assert_eq!(row.leader_modifier_kind, 1);
        assert_eq!(
            row.leader_modifiers[1],
            ContextModifier {
                raw: -777,
                present: 1
            }
        );
        assert_eq!(commander(&memory, 0, 0xe000), Ok(77_u64 << 32 | 4713));
        memory.u64(0xe008, 1_u64 << 32 | 99);
        assert_eq!(
            commander(&memory, 0, 0xe000),
            Err("land combat division group identity mismatch")
        );
    }

    #[test]
    fn manpower_preserves_every_signed_term_without_summing() {
        let mut image = fixture(2);
        image[0x13d8..0x13e0]
            .copy_from_slice(&(crate::carmy_manpower_values::VTABLE_RVA as u64).to_le_bytes());
        image[0x13e0..0x13e8].copy_from_slice(&0x4000_u64.to_le_bytes());
        image[0x13e8..0x13f0].copy_from_slice(&(2_u64 << 32 | 2).to_le_bytes());
        image[0x4000..0x4004].copy_from_slice(&123_u32.to_le_bytes());
        image[0x4004..0x4008].copy_from_slice(&(-123_i32).to_le_bytes());
        image[0x4008..0x400c].copy_from_slice(&456_u32.to_le_bytes());
        image[0x400c..0x4010].copy_from_slice(&i32::MAX.to_le_bytes());
        let memory = Counted {
            inner: SliceMemory {
                base: 0,
                bytes: &image,
            },
            copies: Cell::new(0),
            bytes: Cell::new(0),
        };
        let mut terms = [ManpowerTerm::default(); 2];
        assert_eq!(
            capture_manpower(&memory, 0, 0x1000, 0x1234_0000_0033, &mut terms),
            Ok(2)
        );
        assert_eq!(
            terms,
            [
                ManpowerTerm {
                    source: 123,
                    amount: -123
                },
                ManpowerTerm {
                    source: 456,
                    amount: i32::MAX
                }
            ]
        );
        assert_eq!(memory.copies.get(), 9);
        assert_eq!(memory.bytes.get(), 96);
        assert_eq!(
            capture_manpower(&memory, 0, 0x1000, 0x1234_0000_0033, &mut []),
            Err("land combat manpower arena full")
        );
    }
}
