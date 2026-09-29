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
/// The division's `CNameGroupMember`: 140C72F00 returns its generated +0x60
/// name, which committed 1415BA240 and reserve 1415BAC10 entries draw.
pub const NAME_GROUP_MEMBER: usize = 0x340;
/// 141298E00 groups attacking units with a positive value here into one direction.
pub const DIRECTION_GROUP: usize = 0x4a8;
/// Raw contextual keys used by width140C6E270 and reinforcement140C6D610.
pub const CONTEXT_MODIFIER_KEYS: [u32; 3] = [
    crate::modifier_keys::LAND_REINFORCE_RATE,
    crate::modifier_keys::COMBAT_WIDTH_FACTOR,
    crate::modifier_keys::INITIATIVE_FACTOR,
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
    /// Spans index the caller's name arena. `None`: a player's name or group
    /// key did not fit it; its bytes were counted for growth and the division
    /// is omitted this hour.
    pub naming: Option<crate::cname_group_member::Inputs>,
    /// Committed strength bar multiplies raw [`carmy::STRENGTH_FACTOR`] (140C6F8B0).
    pub strength_combat_factor: i64,
    /// Fuel icon 140C6F960: CArmy+620 and stat+120, raw fixed5; the define
    /// comes from the content.
    pub fuel_inputs: [i64; 2],
    /// Defense, breakthrough, hardness, soft attack, hard attack; all raw fixed5.
    pub stat_values: [i64; 5],
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
    /// Raw CUnit+0x4A8 latch word for the direction count.
    pub direction_group: i32,
    /// The equipment category mask of the unit's stat block
    /// (`*(CUnit+0x138)+0x5A8`), which CEquipmentTypeTrigger::vf23
    /// (141390370) ANDs with a category trigger's mask (140F731B0) for each
    /// unit of a CLandCombatant (+0x20). A division's block is its own
    /// (CUnit ctor 140BE9620 -> 141003A50 zeroes it; update_active_stats
    /// 140C7DEB0 -> CalculateStatsFromEquipment 140B8BBE0 leaves it), so it
    /// is recorded as stored rather than derived from content.
    pub categories: u64,
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
/// Strength, organisation and their maxima are the Divisions and DivisionStatus
/// values and are not read again here.
///
/// On success the division, its handle and its stat block are recorded in
/// `sources`, so the same division's later readers in the pass skip those
/// identity reads.
pub fn capture_stats(
    memory: &impl Memory,
    base: usize,
    sources: &mut crate::combat_sources::Sources,
    pointer: usize,
    expected_handle: u64,
    groups: &mut crate::cname_group_member::GroupKeys,
    names: &mut crate::cname_group_member::Names<'_>,
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
    sources.verified = None;
    // Every remaining CArmy and stat-block operand in one guarded scatter; the
    // commander and name reads depend only on its group and member pointers.
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

/// `CReferencedDivisionTemplate+0x1F8`, the saved origin type: 0 master host,
/// 1 subject, 2 exile. `CArmy::IsExiled` (140C79AC0) tests it for exactly 2.
pub const TEMPLATE_ORIGIN_TYPE: usize = 0x1f8;
/// The origin `CArmy::IsExiled` (140C79AC0) compares `+0x1F8` with.
pub const EXILE_ORIGIN: i32 = 2;
/// `+0x1F4` is the template country when positive, else `+0x1EC` (140C770A0).
pub const TEMPLATE_COUNTRY: usize = 0x1f4;
pub const TEMPLATE_COUNTRY_FALLBACK: usize = 0x1ec;
/// The occupier set 14129FAD0/14129FB60 search: a `CPdxArray<u32>` at the
/// exile country's diplomacy status (`CCountry+0xF88`) plus 0x140.
pub const OCCUPIER_SET: usize = 0x140;

/// The operands 1415CA660's exile frames 14 and 15 read for one committed
/// division whose template is exiled (origin [`EXILE_ORIGIN`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExileRow {
    pub origin_type: i32,
    pub country: i32,
    pub country_fallback: i32,
    /// Spans of the caller's tag destination: the location state's cores, then
    /// the exile country's occupier set.
    pub cores: (usize, usize),
    pub occupiers: (usize, usize),
}

/// One committed division's exile acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exile {
    /// No template, or one whose origin is not [`EXILE_ORIGIN`] (140C79AC0).
    Absent,
    /// Spans index the destination slice the caller supplied.
    Row(ExileRow),
    /// Both lists are valid and within their bound, but together need
    /// `required` tag slots; nothing was copied. The caller grows its arena
    /// from this count off the game thread and omits the division meanwhile.
    DoesNotFit { required: usize },
}

/// A counted `CPdxArray<u32>` of country tags; a nonpositive signed count holds
/// no entries, as the native loops test `0 < count`. Tags name countries, so a
/// list longer than the live country table (sentinel included) is refused
/// before any entry is copied or any destination is sized from it.
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

/// Copies a validated list in 64-tag guarded batches.
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

/// Reads one committed division's exile operands into `tags`, which the caller
/// preallocated; nothing here allocates. `countries` is the pass's country
/// reader: it resolves the exile country and bounds both lists by the live
/// country count.
pub fn capture_exile(
    memory: &impl Memory,
    countries: &crate::ccountry::Reader<'_, impl Memory>,
    pointer: usize,
    tags: &mut [u32],
) -> Result<Exile, &'static str> {
    let template =
        u64::from_le_bytes(bytes::<8>(memory, pointer, carmy::STANDARD_TEMPLATE)?) as usize;
    if template == 0 {
        return Ok(Exile::Absent);
    }
    // Fallback country +0x1EC, a padding word, primary +0x1F4, origin +0x1F8.
    let operands = bytes::<16>(memory, template, TEMPLATE_COUNTRY_FALLBACK)?;
    let word = |at: usize| i32::from_le_bytes(operands[at..at + 4].try_into().unwrap());
    let mut row = ExileRow {
        country_fallback: word(0),
        country: word(TEMPLATE_COUNTRY - TEMPLATE_COUNTRY_FALLBACK),
        origin_type: word(TEMPLATE_ORIGIN_TYPE - TEMPLATE_COUNTRY_FALLBACK),
        ..ExileRow::default()
    };
    // 140C79AC0: a nonnull template is exiled exactly when its origin is 2;
    // every other value takes the ordinary-owner path.
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
        u64::from_le_bytes(bytes::<8>(memory, province, crate::cprovince::STATE)?) as usize
    };
    let cores = if state == 0 {
        Array { data: 0, count: 0 }
    } else {
        country_list(
            memory,
            state
                .checked_add(crate::cstate::CORES)
                .ok_or("exile core list address overflow")?,
            limit,
        )?
    };
    // 14129FAD0/14129FB60 resolve the exile country (140BA56A0) and read its
    // diplomacy status with no null test, so an exiled division's country
    // always resolves in the game: a failure here is a read or identity
    // fault and fails the division with its reason.
    let object = countries.resolve(country)?;
    let status = u64::from_le_bytes(bytes::<8>(
        memory,
        object.pointer,
        crate::ccountry::DIPLOMACY_STATUS,
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
    if required > tags.len() {
        return Ok(Exile::DoesNotFit { required });
    }
    copy_tags(memory, cores, &mut tags[..core_count])?;
    copy_tags(memory, occupiers, &mut tags[core_count..required])?;
    row.cores = (0, core_count);
    row.occupiers = (core_count, occupier_count);
    Ok(Exile::Row(row))
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

/// Width composition and contextual reinforcement operands. Terms preserve
/// native order, signed multiplicities and unscaled definition widths. No
/// width, chance, modifier clamp or speed bonus is calculated here. The caller
/// supplies the pass's country reader rather than rebuilding one per division.
///
/// Per-source reads are shared through `sources` for the pass
/// ([`crate::combat_sources`]): every read a division still makes, and every
/// failure, comes at the unshared reader's point with its reason.
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    countries: &crate::ccountry::Reader<'_, impl Memory>,
    sources: &mut crate::combat_sources::Sources,
    pointer: usize,
    expected_handle: u64,
    terms: &mut [WidthTerm],
) -> Result<ContextRow, &'static str> {
    use crate::{ccountry, combat_sources::share};
    let verified = sources
        .verified
        .filter(|(verified, handle, _)| *verified == pointer && *handle == expected_handle);
    if verified.is_none() {
        let identity = bytes::<8>(memory, pointer, 0)?;
        let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
        if expected_handle == 0
            || u64::from_le_bytes(handle) != expected_handle
            || Some(u64::from_le_bytes(identity))
                != base.checked_add(carmy::VTABLE_RVA).map(|p| p as u64)
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
        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
        .ok_or("land combat context country address overflow")?;
    let country_modifiers = *share(&mut sources.context_modifiers, country_accumulator, || {
        context_modifiers(memory, country_accumulator)
    })?;
    let leader = sources.leader_chain(memory, base, pointer, expected_handle)?;
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
                != base
                    .checked_add(csub_unit_definition::VTABLE_RVA)
                    .map(|p| p as u64)
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
        // Main refuses a full arena before reading any term.
        if width.count > terms.len() {
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
    })
}

/// The pass's modifier definition operands for [`CONTEXT_MODIFIER_KEYS`], the
/// eligibility callback and the feature flags: module data, read once a pass.
fn context_metadata(memory: &impl Memory, base: usize) -> Result<ContextMetadata, &'static str> {
    use crate::modifier_definition as metadata;
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
    Ok(ContextMetadata {
        modifier_metadata,
        eligibility_callback,
        feature_flags,
    })
}

/// One template's width composition: its handle and term count, then its
/// terms in native order into the pass's arena, or the read that stopped them.
/// Each division checks its own arena between the two, as main does.
fn template_width(
    memory: &impl Memory,
    base: usize,
    template_pointer: usize,
    arena: &mut Vec<WidthTerm>,
) -> Result<TemplateWidth, &'static str> {
    use crate::creferenced_division_template as template;
    let template_identity = bytes::<16>(memory, template_pointer, 0)?;
    let handle = u64::from_le_bytes(template_identity[8..].try_into().unwrap());
    if Some(u64::from_le_bytes(
        template_identity[..8].try_into().unwrap(),
    )) != base.checked_add(template::VTABLE_RVA).map(|p| p as u64)
        || handle as u32 != template::REFERENCE_TYPE
        || handle >> 32 == 0
    {
        return Err("land combat width template identity mismatch");
    }
    let term_header = bytes(memory, template_pointer, 0xc0)?;
    let array =
        Array::from_header(term_header, 16).ok_or("land combat width term header invalid")?;
    let count = array.count as usize;
    if count > WIDTH_TERM_LIMIT {
        return Err("land combat width term bound exceeded");
    }
    let start = arena.len();
    let terms = (|| {
        let mut packed = [0; WIDTH_TERM_LIMIT * 16];
        if !read_slice(memory, array.data, &mut packed[..count * 16]) {
            return Err("land combat width terms unreadable");
        }
        for input in packed[..count * 16].chunks_exact(16) {
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
            arena.push(WidthTerm {
                subunit: u32::from_le_bytes(raw[8..].try_into().unwrap()),
                count: i32::from_le_bytes(input[8..12].try_into().unwrap()),
                width: i64::from_le_bytes(bytes(memory, subunit, 0x58)?),
            });
        }
        Ok(crate::combat_sources::Run { start, len: count })
    })();
    Ok(TemplateWidth {
        handle,
        count,
        terms,
    })
}

/// Per-pass shared context operands ([`crate::combat_sources`]).
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
    Ok(output)
}

fn commander(memory: &impl Memory, base: usize, group: usize) -> Result<u64, &'static str> {
    if group == 0 {
        return Ok(0);
    }
    // The identity and leader pointer share one guarded scatter; an identity
    // mismatch still takes precedence over an unreadable leader word.
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
    .any(|rva| base.checked_add(*rva).map(|value| value as u64) == Some(vtable))
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
    use std::cell::{Cell, RefCell};

    /// A pass-sized naming context for tests.
    struct NameArena {
        groups: crate::cname_group_member::GroupKeys,
        bytes: Vec<u8>,
    }
    impl NameArena {
        fn new() -> Self {
            Self {
                groups: crate::cname_group_member::GroupKeys::with_capacity(64, 64),
                bytes: vec![0; 4096],
            }
        }
        fn stats(
            &mut self,
            memory: &impl Memory,
            base: usize,
            pointer: usize,
            handle: u64,
        ) -> Result<Row, &'static str> {
            let mut names = crate::cname_group_member::Names {
                bytes: &mut self.bytes,
                len: 0,
                required: 0,
            };
            capture_stats(
                memory,
                base,
                &mut crate::combat_sources::Sources::new(),
                pointer,
                handle,
                &mut self.groups,
                &mut names,
            )
        }
        fn text(&self, span: (usize, usize)) -> &[u8] {
            &self.bytes[span.0..span.0 + span.1]
        }
    }

    /// The pre-hoist call shape: builds the country reader over the same
    /// memory, immediately before the division reads, as the pass does.
    fn capture_context_at(
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &mut crate::combat_sources::Sources,
        pointer: usize,
        expected_handle: u64,
        terms: &mut [WidthTerm],
    ) -> Result<ContextRow, &'static str> {
        let countries = crate::ccountry::Reader::new(memory, base, state)?;
        capture_context(
            memory,
            base,
            &countries,
            sources,
            pointer,
            expected_handle,
            terms,
        )
    }

    struct Observed<'a, M> {
        source: &'a M,
        reads: RefCell<Vec<(usize, usize)>>,
        scatters: RefCell<Vec<Vec<(usize, usize)>>>,
        fault: Option<usize>,
        change: Option<(usize, usize)>,
    }

    impl<'a, M> Observed<'a, M> {
        fn new(source: &'a M) -> Self {
            Self {
                source,
                reads: RefCell::default(),
                scatters: RefCell::default(),
                fault: None,
                change: None,
            }
        }
    }

    impl<M: Memory> Memory for Observed<'_, M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, destination.len()));
            if self.fault == Some(ordinal) {
                let prefix = destination.len().div_ceil(2);
                destination[..prefix].fill(0xed);
                return false;
            }
            let copied = self.source.copy(address, destination);
            if copied
                && let Some((at, byte)) = self.change
                && at == ordinal
            {
                destination[byte] ^= 1;
            }
            copied
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.scatters.borrow_mut().push(
                addresses
                    .iter()
                    .zip(destinations.iter())
                    .map(|(&p, d)| (p, d.len()))
                    .collect(),
            );
            for (index, (address, destination)) in
                addresses.into_iter().zip(destinations).enumerate()
            {
                if !read_slice(self, address, destination) {
                    return index;
                }
            }
            N
        }
    }

    // The scalar order before batching. Unchanged dependency helpers stay shared
    // so the differential covers their interleaving with both new leaf groups.
    fn scalar_stats(
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
        let armour = values::<2>(&bytes::<16>(memory, stat, ARMOUR)?);
        let group = u64::from_le_bytes(bytes::<8>(memory, pointer, carmy::ORDERS_GROUP)?) as usize;
        let member = u64::from_le_bytes(bytes::<8>(memory, pointer, NAME_GROUP_MEMBER)?);
        let strength_combat_factor = i64::from_le_bytes(bytes(memory, pointer, 0x468)?);
        let fuel_inputs = [
            i64::from_le_bytes(bytes(memory, pointer, 0x620)?),
            i64::from_le_bytes(bytes(memory, stat, 0x120)?),
        ];
        let stat_values = values(&bytes::<40>(memory, stat, csub_unit_definition::DEFENSE)?);
        let air_attack = i64::from_le_bytes(bytes(memory, stat, AIR_ATTACK)?);
        let reconnaissance = i64::from_le_bytes(bytes(memory, stat, 0xd0)?);
        let initiative = i64::from_le_bytes(bytes(memory, stat, 0xe0)?);
        let scales = values(&bytes::<32>(memory, pointer, cunit::ATTACK_ACCUMULATOR)?);
        let direction_group = i32::from_le_bytes(bytes(memory, pointer, DIRECTION_GROUP)?);
        let categories = u64::from_le_bytes(bytes(memory, stat, csub_unit_definition::AIR_FLAGS)?);
        let commander = commander(memory, base, group)?;
        Ok(Row {
            division: expected_handle,
            naming: naming(
                memory,
                base,
                member,
                &mut crate::cname_group_member::GroupKeys::with_capacity(64, 64),
                &mut crate::cname_group_member::Names {
                    bytes: &mut [0; 4096],
                    len: 0,
                    required: 0,
                },
            )?,
            strength_combat_factor,
            fuel_inputs,
            stat_values,
            armour: armour[0],
            piercing: armour[1],
            air_attack,
            reconnaissance,
            initiative,
            commander,
            scales,
            direction_group,
            categories,
        })
    }

    #[test]
    fn stats_scatter_matches_scalar_first_values_and_every_fault() {
        let mut source = context_fixture();
        source.u64(0x10c0, 0xe000);
        source.u64(0xe000, crate::corders_group::VTABLE_RVA as u64);
        source.u64(0xe008, 1_u64 << 32 | 53);
        source.u64(0xe088, 0xf000);
        source.u64(0xf000, crate::carmy_leader::VTABLE_RVA as u64);
        source.u64(0xf008, 77_u64 << 32 | 4713);
        for (index, (address, count)) in [
            (0x2298, 2),
            (0x1468, 1),
            (0x1620, 1),
            (0x2120, 1),
            (0x20a8, 5),
            (0x2220, 1),
            (0x20d0, 1),
            (0x20e0, 1),
            (0x1000 + cunit::ATTACK_ACCUMULATOR, 4),
        ]
        .into_iter()
        .enumerate()
        {
            for lane in 0..count {
                source.u64(
                    address + lane * 8,
                    (i64::MIN + index as i64 * 31 + lane as i64) as u64,
                );
            }
        }
        source.put(0x1000 + DIRECTION_GROUP, &(-3_i32).to_le_bytes());
        source.u64(
            0x2000 + csub_unit_definition::AIR_FLAGS,
            0x8000_0000_0000_0020,
        );
        let expected = Observed::new(&source);
        let actual = Observed::new(&source);
        let row = scalar_stats(&expected, 0, 0x1000, 0x1234_0000_0033).unwrap();
        assert_eq!(row.direction_group, -3);
        assert_eq!(row.categories, 0x8000_0000_0000_0020);
        let capture =
            |memory: &Observed<'_, _>, handle| NameArena::new().stats(memory, 0, 0x1000, handle);
        assert_eq!(capture(&actual, row.division), Ok(row));
        assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
        assert_eq!(
            *actual.scatters.borrow(),
            [
                vec![
                    (0x2298, 16),
                    (0x10c0, 8),
                    (0x1000 + NAME_GROUP_MEMBER, 8),
                    (0x1468, 8),
                    (0x1620, 8),
                    (0x2120, 8),
                    (0x20a8, 40),
                    (0x2220, 8),
                    (0x20d0, 8),
                    (0x20e0, 8),
                    (0x1000 + cunit::ATTACK_ACCUMULATOR, 32),
                    (0x1000 + DIRECTION_GROUP, 4),
                    (0x2000 + csub_unit_definition::AIR_FLAGS, 8)
                ],
                // The commander's group identity and leader word.
                vec![(0xe000, 16), (0xe000 + crate::corders_group::LEADER, 8)],
            ]
        );
        let trace = expected.reads.into_inner();
        for ordinal in 0..trace.len() {
            for change in [false, true] {
                let mut expected = Observed::new(&source);
                let mut actual = Observed::new(&source);
                if change {
                    expected.change = Some((ordinal, 0));
                    actual.change = expected.change;
                } else {
                    expected.fault = Some(ordinal);
                    actual.fault = expected.fault;
                }
                assert_eq!(
                    capture(&actual, row.division),
                    scalar_stats(&expected, 0, 0x1000, row.division),
                    "ordinal {ordinal}, change {change}"
                );
                assert_eq!(
                    *actual.reads.borrow(),
                    *expected.reads.borrow(),
                    "ordinal {ordinal}"
                );
                if !change {
                    assert_eq!(*actual.reads.borrow(), trace[..=ordinal]);
                }
            }
        }
    }

    #[test]
    fn stats_preserve_identity_module_overflow_order_and_later_address_failure() {
        let mut source = context_fixture();
        for fault in [None, Some(0)] {
            let mut actual = Observed::new(&source);
            actual.fault = fault;
            let result = NameArena::new().stats(&actual, usize::MAX, 0x1000, 0x1234_0000_0033);
            assert_eq!(
                result,
                Err(if fault.is_some() {
                    "land combat division inputs unreadable"
                } else {
                    "land combat division identity mismatch"
                })
            );
            assert_eq!(*actual.reads.borrow(), [(0x1000, 8)]);
            assert!(actual.scatters.borrow().is_empty());
        }
        // An unreadable armour span fails before the group read.
        let stat = usize::MAX - 0x2a0;
        source.u64(0x1138, stat as u64);
        source.u64(stat, csub_unit_definition::VTABLE_RVA as u64);
        let actual = Observed::new(&source);
        let expected = Observed::new(&source);
        assert_eq!(
            NameArena::new().stats(&actual, 0, 0x1000, 0x1234_0000_0033),
            scalar_stats(&expected, 0, 0x1000, 0x1234_0000_0033)
        );
        assert!(!actual.reads.borrow().contains(&(0x10c0, 8)));
    }

    #[test]
    fn input_scatter_keeps_partial_failure_empty_duplicates_and_address_error_prefix() {
        let source = SliceMemory {
            base: 0x1000,
            bytes: &[1, 2, 3, 4, 5, 6, 7, 8],
        };
        for fault in [None, Some(0), Some(1)] {
            let mut memory = Observed::new(&source);
            memory.fault = fault;
            let (mut first, mut duplicate, mut later) = ([0xa5; 8], [0xa5; 8], [0xa5; 8]);
            let result = copy_input_spans(
                &memory,
                [
                    Ok(0x1000),
                    Err("empty address"),
                    Ok(0x1000),
                    Err("late address"),
                ],
                &mut [&mut first, &mut [], &mut duplicate, &mut later],
            );
            assert_eq!(
                result,
                Err(if fault.is_some() {
                    "land combat division inputs unreadable"
                } else {
                    "late address"
                })
            );
            assert_eq!(later, [0xa5; 8]);
            if let Some(ordinal) = fault {
                let failed = if ordinal == 0 { first } else { duplicate };
                assert_eq!(failed, [0xed, 0xed, 0xed, 0xed, 0xa5, 0xa5, 0xa5, 0xa5]);
                if ordinal == 0 {
                    assert_eq!(duplicate, [0xa5; 8]);
                }
            } else {
                assert_eq!(first, [1, 2, 3, 4, 5, 6, 7, 8]);
                assert_eq!(duplicate, first);
            }
            assert_eq!(memory.reads.borrow().len(), fault.map_or(2, |i| i + 1));
        }
    }

    #[test]
    fn context_groups_keep_first_bytes_and_fault_prefixes() {
        use crate::modifier_definition as md;
        let source = context_fixture();
        let baseline = Observed::new(&source);
        let mut terms = [WidthTerm::default(); 2];
        let row = capture_context_at(
            &baseline,
            0,
            0x5000,
            &mut crate::combat_sources::Sources::new(),
            0x1000,
            0x1234_0000_0033,
            &mut terms,
        )
        .unwrap();
        assert_eq!(row.template, 1_u64 << 32 | 52);
        let metadata: Vec<_> = CONTEXT_MODIFIER_KEYS
            .map(|key| (0x10000 + key as usize * md::STRIDE + md::FLAGS, 12))
            .into();
        assert!(baseline.scatters.borrow().contains(&metadata));
        let trace = baseline.reads.into_inner();
        assert_eq!(trace.len(), 33);
        assert_eq!(trace.iter().map(|(_, len)| len).sum::<usize>(), 364);
        for group in [&metadata] {
            let start = trace
                .windows(group.len())
                .position(|p| p == group.as_slice())
                .unwrap();
            for ordinal in start..start + group.len() {
                let mut memory = Observed::new(&source);
                memory.fault = Some(ordinal);
                let mut out = [WidthTerm::default(); 2];
                assert_eq!(
                    capture_context_at(
                        &memory,
                        0,
                        0x5000,
                        &mut crate::combat_sources::Sources::new(),
                        0x1000,
                        0x1234_0000_0033,
                        &mut out
                    ),
                    Err("land combat division inputs unreadable")
                );
                assert_eq!(*memory.reads.borrow(), trace[..=ordinal]);
                assert_eq!(out, [WidthTerm::default(); 2]);
            }
        }
        for span in &metadata {
            assert_eq!(trace.iter().filter(|p| *p == span).count(), 1);
        }
        assert_eq!(trace.iter().filter(|p| **p == (0xc000, 32)).count(), 1);
    }

    #[test]
    fn context_metadata_address_errors_follow_readable_prefix_and_earlier_fault() {
        use crate::modifier_definition as md;
        let second_offset = CONTEXT_MODIFIER_KEYS[1] as usize * md::STRIDE;
        for (table, reason) in [
            (
                usize::MAX - second_offset + 1,
                "land combat context metadata address overflow",
            ),
            (
                usize::MAX - second_offset - md::FLAGS + 1,
                "land combat division inputs unreadable",
            ),
            (
                usize::MAX - second_offset - md::FLAGS - 5,
                "land combat division inputs unreadable",
            ),
        ] {
            let mut source = context_fixture();
            source.u64(md::TABLE_RVA, table as u64);
            let first = table + CONTEXT_MODIFIER_KEYS[0] as usize * md::STRIDE + md::FLAGS;
            source.put(first, &[0x17; 12]);
            let memory = Observed::new(&source);
            assert_eq!(
                capture_context_at(
                    &memory,
                    0,
                    0x5000,
                    &mut crate::combat_sources::Sources::new(),
                    0x1000,
                    0x1234_0000_0033,
                    &mut []
                ),
                Err(reason)
            );
            let trace = memory.reads.into_inner();
            assert_eq!(trace.last(), Some(&(first, 12)));
            let mut failed = Observed::new(&source);
            failed.fault = Some(trace.len() - 1);
            assert_eq!(
                capture_context_at(
                    &failed,
                    0,
                    0x5000,
                    &mut crate::combat_sources::Sources::new(),
                    0x1000,
                    0x1234_0000_0033,
                    &mut []
                ),
                Err("land combat division inputs unreadable")
            );
            assert_eq!(*failed.reads.borrow(), trace);
        }
    }

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
        // The division's name member: order i32::MAX and a one-byte custom name.
        put(0x1340, 0x3800);
        put(0x3800, crate::cname_group_member::VTABLE_RVA as u64);
        put(0x3880, i32::MAX as u64);
        put(0x3888, 0xff);
        put(0x3898, 1);
        put(0x38a0, 15);
        put(0x38a8, 1);
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
        let image = fixture(2);
        let memory = Counted {
            inner: SliceMemory {
                base: 0,
                bytes: &image,
            },
            copies: Cell::new(0),
            bytes: Cell::new(0),
        };
        let mut modifiers = [Modifier::default(); 2];
        let mut arena = NameArena::new();
        let row = arena.stats(&memory, 0, 0x1000, 0x1234_0000_0033).unwrap();
        assert_eq!(
            capture_modifiers(&memory, 0, 0x1000, 0x1234_0000_0033, &mut modifiers).unwrap(),
            2
        );
        let naming = row.naming.unwrap();
        assert_eq!(arena.text(naming.custom), &[255]);
        assert_eq!(naming.order, i32::MAX);
        assert_eq!(
            modifiers[0],
            Modifier {
                id: 27,
                value_a: -900,
                value_b: 800
            }
        );
        assert_eq!(modifiers[1].id, 2);
        // Four identity words, one thirteen-span scatter, one 0xAA name-member
        // copy and four modifier copies: 22 observations, 446 bytes.
        assert_eq!(memory.copies.get(), 22);
        assert_eq!(memory.bytes.get(), 446);
        println!(
            "division stats/name plus two modifiers: {} logical copy observations / {} bytes",
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
            NameArena::new().stats(&memory, 0, 0x1000, 0x1234_0000_0033),
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
        let row = capture_context_at(
            &memory,
            0,
            0x5000,
            &mut crate::combat_sources::Sources::new(),
            0x1000,
            0x1234_0000_0033,
            &mut terms,
        )
        .unwrap();
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
        assert_eq!(row.template, 1_u64 << 32 | 52);
        assert_eq!(memory.copies.get(), 33);
        assert_eq!(memory.copied.get(), 364);
    }

    #[test]
    fn context_modifier_reuse_is_shared_only_within_the_current_pass() {
        let mut memory = context_fixture();
        let mut sources = crate::combat_sources::Sources::new();
        let mut terms = [WidthTerm::default(); 2];
        let first = capture_context_at(
            &memory,
            0,
            0x5000,
            &mut sources,
            0x1000,
            0x1234_0000_0033,
            &mut terms,
        )
        .unwrap();
        let first_terms = terms;
        let before = (memory.copies.get(), memory.copied.get());
        let second = capture_context_at(
            &memory,
            0,
            0x5000,
            &mut sources,
            0x1000,
            0x1234_0000_0033,
            &mut terms,
        )
        .unwrap();
        assert_eq!(second, first);
        assert_eq!(terms, first_terms);
        // Within the pass the country census, the metadata, the defines and
        // the template's width terms are shared; the division's own identity,
        // owners, stat block, base speed and template pointer are read again.
        assert_eq!(memory.copies.get() - before.0, 17);

        memory.u64(0xa008, 170);
        sources.clear();
        let current = capture_context_at(
            &memory,
            0,
            0x5000,
            &mut sources,
            0x1000,
            0x1234_0000_0033,
            &mut terms,
        )
        .unwrap();
        assert_eq!(
            current.country_modifiers[0],
            ContextModifier {
                raw: 170,
                present: 1
            }
        );

        // A different address never aliases the previous country observation.
        memory.array(0xe000, 0xe100, 1);
        memory.put(0xe100, &0x10f_u32.to_le_bytes());
        memory.u64(0xe108, 999);
        let before = memory.copies.get();
        let other = context_modifiers(&memory, 0xe000).unwrap();
        assert_eq!(other[0], ContextModifier::default());
        assert_eq!(
            other[1],
            ContextModifier {
                raw: 999,
                present: 1
            }
        );
        assert_eq!(memory.copies.get() - before, 2);
    }

    #[test]
    fn context_modifier_census_fails_at_its_read_and_on_a_malformed_suffix() {
        let mut source = context_fixture();
        let address = 0x9000 + crate::ccountry::MODIFIER_ACCUMULATOR;
        for ordinal in [0, 1] {
            let mut observed = Observed::new(&source);
            observed.fault = Some(ordinal);
            assert!(context_modifiers(&observed, address).is_err());
            assert_eq!(observed.reads.borrow().len(), ordinal + 1);
        }
        // The selected values are already in the first chunk. A malformed
        // unselected suffix must still fail.
        source.array(address, 0x20000, 65);
        for (index, key) in CONTEXT_MODIFIER_KEYS
            .into_iter()
            .chain(1003..1065)
            .enumerate()
        {
            source.put(0x20000 + index * 16, &key.to_le_bytes());
            source.u64(0x20008 + index * 16, index as u64 + 1);
        }
        source.put(0x20000 + 64 * 16, &1_u32.to_le_bytes());
        let before = source.copies.get();
        assert_eq!(
            context_modifiers(&source, address),
            Err("land combat context modifier keys not increasing")
        );
        assert_eq!(source.copies.get() - before, 3);
        source.put(0x20000 + 64 * 16, &1064_u32.to_le_bytes());
        let values = context_modifiers(&source, address).unwrap();
        assert_eq!(values.map(|value| value.raw), [1, 2, 3]);
    }

    #[test]
    fn exile_lists_are_bounded_by_the_live_country_count_and_never_sized_from_it() {
        let mut memory = context_fixture();
        // A committed division on an exile template (origin 2) of tag 1.
        memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
        memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &2_i32.to_le_bytes());
        memory.u64(0x1000 + cunit::LOCATION, 0x7000);
        memory.u64(0x7000 + crate::cprovince::STATE, 0x7800);
        memory.array(0x7800 + crate::cstate::CORES, 0x7c00, 2);
        memory.put(0x7c00, &[1, 0, 0, 0, 2, 0, 0, 0]);
        memory.u64(0x9000 + crate::ccountry::DIPLOMACY_STATUS, 0xe800);
        memory.array(0xe800 + OCCUPIER_SET, 0xec00, 1);
        memory.put(0xec00, &2_u32.to_le_bytes());
        let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        assert_eq!(countries.count(), 2);
        let mut tags = [0; 3];
        assert_eq!(
            capture_exile(&memory, &countries, 0x1000, &mut tags),
            Ok(Exile::Row(ExileRow {
                origin_type: 2,
                country: 1,
                country_fallback: 0,
                cores: (0, 2),
                occupiers: (2, 1),
            }))
        );
        assert_eq!(tags, [1, 2, 2]);
        // A short destination copies nothing and reports its demand.
        let mut short = [0; 2];
        assert_eq!(
            capture_exile(&memory, &countries, 0x1000, &mut short),
            Ok(Exile::DoesNotFit { required: 3 })
        );
        assert_eq!(short, [0; 2]);
        // Longer than the country table, up to a count that would have
        // reserved gigabytes: refused before any entry is read.
        for count in [3, i32::MAX as u32] {
            memory.array(0x7800 + crate::cstate::CORES, 0x7c00, count);
            let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
            let before = memory.copies.get();
            assert_eq!(
                capture_exile(&memory, &countries, 0x1000, &mut [0; 8]),
                Err("exile country list exceeds the live country count")
            );
            // Template, operands, location, state and the list header only.
            assert_eq!(memory.copies.get() - before, 5);
        }
        // 140C79AC0 is exactly `origin == 2`: every other value, including
        // ones outside 0..=2, is not exiled and reads nothing further.
        for origin in [0, 1, 3, 7, -1, i32::MIN, i32::MAX] {
            memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &origin.to_le_bytes());
            let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
            let before = memory.copies.get();
            assert_eq!(
                capture_exile(&memory, &countries, 0x1000, &mut []),
                Ok(Exile::Absent),
                "origin {origin}"
            );
            // Template pointer and operands only.
            assert_eq!(memory.copies.get() - before, 2, "origin {origin}");
        }
    }

    /// An exiled division's country always resolves in the game (14129FB60
    /// reads its diplomacy status unguarded), so a resolution or read fault is
    /// the division's failure, never an empty occupier set. The exile country
    /// here differs from the division's ordinary owner, which resolves.
    #[test]
    fn exile_country_faults_fail_the_division_with_their_reason() {
        let mut memory = context_fixture();
        memory.put(0x1000 + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
        memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &2_i32.to_le_bytes());
        memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
        memory.u64(0x9000 + crate::ccountry::DIPLOMACY_STATUS, 0xe800);
        let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        assert!(matches!(
            capture_exile(&memory, &countries, 0x1000, &mut [0; 4]),
            Ok(Exile::Row(_))
        ));
        // A tag outside the link table.
        memory.put(0xb000 + TEMPLATE_COUNTRY, &5_i32.to_le_bytes());
        let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let unlinked = capture_exile(&memory, &countries, 0x1000, &mut [0; 4]);
        assert_eq!(unlinked, countries.resolve(5).map(|_| Exile::Absent));
        assert!(unlinked.is_err());
        // The exile country's diplomacy status unreadable, then null.
        memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
        memory.u64(0x9000 + crate::ccountry::DIPLOMACY_STATUS, 0);
        let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        assert_eq!(
            capture_exile(&memory, &countries, 0x1000, &mut [0; 4]),
            Err("exile country diplomacy status missing")
        );
        let status = 0x9000 + crate::ccountry::DIPLOMACY_STATUS;
        memory.u64(status, 0xe800);
        let mut observed = Observed::new(&memory);
        let countries = crate::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let _ = capture_exile(&observed, &countries, 0x1000, &mut [0; 4]);
        let ordinal = observed
            .reads
            .borrow()
            .iter()
            .position(|&(at, _)| at == status)
            .unwrap();
        observed = Observed::new(&memory);
        observed.fault = Some(ordinal);
        assert_eq!(
            capture_exile(&observed, &countries, 0x1000, &mut [0; 4]),
            Err("land combat division inputs unreadable")
        );
    }

    #[test]
    fn width_context_reports_short_arena_and_modifier_bound() {
        let mut memory = context_fixture();
        assert_eq!(
            capture_context_at(
                &memory,
                0,
                0x5000,
                &mut crate::combat_sources::Sources::new(),
                0x1000,
                0x1234_0000_0033,
                &mut []
            ),
            Err("land combat width term arena full")
        );
        memory.array(0x9000 + crate::ccountry::MODIFIER_ACCUMULATOR, 0xa000, 4097);
        assert_eq!(
            capture_context_at(
                &memory,
                0,
                0x5000,
                &mut crate::combat_sources::Sources::new(),
                0x1000,
                0x1234_0000_0033,
                &mut []
            ),
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
        let row = capture_context_at(
            &memory,
            0,
            0x5000,
            &mut crate::combat_sources::Sources::new(),
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

    /// Synthetic scale benchmark over guarded in-process reads (LiveMemory's
    /// SEH copies against heap objects laid out like the native graph). It
    /// measures reader cost only, never live gameplay speed.
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    mod scale {
        use super::*;
        use crate::{cname_group_member as name, creference_registry as registry};

        const ARMY: usize = 0x700;
        const STAT: usize = 0x2c0;
        const MEMBER: usize = 0xb0;
        const HEAP_NAME: usize = 0x40;
        const GROUPS: usize = 100;
        const NAME_GROUPS: usize = 8;

        pub(super) struct World {
            module: Vec<u8>,
            heap: Vec<u8>,
            pub(super) armies: Vec<(usize, u64)>,
        }

        impl World {
            pub(super) fn base(&self) -> usize {
                self.module.as_ptr() as usize
            }
            fn at(&mut self, address: usize) -> &mut [u8] {
                let module = self.module.as_ptr() as usize;
                let heap = self.heap.as_ptr() as usize;
                if (heap..heap + self.heap.len()).contains(&address) {
                    &mut self.heap[address - heap..]
                } else {
                    &mut self.module[address - module..]
                }
            }
            fn put(&mut self, address: usize, bytes: &[u8]) {
                self.at(address)[..bytes.len()].copy_from_slice(bytes);
            }
            fn u64(&mut self, address: usize, value: u64) {
                self.put(address, &value.to_le_bytes());
            }
            fn string(&mut self, address: usize, text: &[u8], heap: usize) {
                if text.len() <= 15 {
                    self.put(address, text);
                    self.put(address + text.len(), &[0]);
                    self.u64(address + 24, 15);
                } else {
                    self.put(heap, text);
                    self.put(heap + text.len(), &[0]);
                    self.u64(address, heap as u64);
                    self.u64(address + 24, 63);
                }
                self.u64(address + 16, text.len() as u64);
            }

            /// `templates` distinct division templates share the divisions
            /// round-robin; the registry keeps a load factor of at most one half.
            pub(super) fn new(count: usize, template_count: usize) -> Self {
                let capacity = (template_count * 2).next_power_of_two().max(1024);
                let overflow = capacity.trailing_zeros() as usize + 2;
                let module = vec![0_u8; registry::LOW_ROOTS_RVA + 0x1000];
                let sizes = [
                    count * ARMY,
                    count * STAT,
                    count * MEMBER,
                    count * HEAP_NAME,
                    GROUPS * 0x100,
                    GROUPS * 0x20,
                    template_count * 0x200,
                    NAME_GROUPS * 0x40,
                    0x40,
                    (capacity + overflow + 1) * 24,
                ];
                let heap = vec![0_u8; sizes.iter().sum::<usize>() + 64];
                let mut world = Self {
                    module,
                    heap,
                    armies: Vec::with_capacity(count),
                };
                let base = world.base();
                let mut next = (world.heap.as_ptr() as usize).next_multiple_of(16);
                let mut region = |size: usize| {
                    let start = next;
                    next += size;
                    start
                };
                let [
                    armies,
                    stats,
                    members,
                    names,
                    groups,
                    leaders,
                    templates,
                    name_groups,
                    table,
                    entries,
                ] = sizes.map(&mut region);
                for g in 0..GROUPS {
                    let (group, leader) = (groups + g * 0x100, leaders + g * 0x20);
                    world.u64(group, (base + crate::corders_group::VTABLE_RVA) as u64);
                    world.u64(group + 8, (g as u64 + 1) << 32 | 53);
                    world.u64(group + crate::corders_group::LEADER, leader as u64);
                    world.u64(leader, (base + crate::carmy_leader::VTABLE_RVA) as u64);
                    world.u64(leader + 8, (g as u64 + 1) << 32 | 4713);
                }
                for n in 0..NAME_GROUPS {
                    let group = name_groups + n * 0x40;
                    world.u64(group, (base + name::GROUP_VTABLE_RVA) as u64);
                    world.string(group + name::GROUP_KEY, format!("GRP_{n}").as_bytes(), 0);
                }
                // A valid robin-hood reference table holding every template.
                world.u64(base + registry::LOW_ROOTS_RVA + 52 * 8, table as u64);
                world.u64(table + registry::ENTRIES, entries as u64);
                world.put(
                    table + registry::COUNT,
                    &(template_count as u32).to_le_bytes(),
                );
                world.put(table + registry::MASK, &(capacity as u32 - 1).to_le_bytes());
                world.put(table + registry::OVERFLOW_DISTANCE, &[overflow as u8]);
                world.put(entries + (capacity + overflow) * 24 + 4, &[255]);
                for t in 0..template_count {
                    let template = templates + t * 0x200;
                    let handle = (t as u64 + 1) << 32 | 52;
                    world.u64(
                        template,
                        (base + crate::creferenced_division_template::VTABLE_RVA) as u64,
                    );
                    world.u64(template + 8, handle);
                    world.u64(
                        template + name::TEMPLATE_NAME_GROUP,
                        (name_groups + (t % NAME_GROUPS) * 0x40) as u64,
                    );
                    let hash = handle
                        .rotate_left(32)
                        .wrapping_mul(registry::HASH_MULTIPLIER);
                    let mut index = (((hash >> 32) as u32 ^ hash as u32) as usize) & (capacity - 1);
                    let (mut key, mut object, mut distance) = (handle, template as u64, 1_u8);
                    loop {
                        let cell = entries + index * 24;
                        let stored = world.at(cell)[4];
                        if stored == 0 {
                            world.put(cell + 4, &[distance]);
                            world.u64(cell + 8, key);
                            world.u64(cell + 16, object);
                            break;
                        }
                        if stored < distance {
                            let word = |world: &mut Self, at: usize| {
                                u64::from_le_bytes(world.at(at)[..8].try_into().unwrap())
                            };
                            let (old_key, old_object) =
                                (word(&mut world, cell + 8), word(&mut world, cell + 16));
                            world.put(cell + 4, &[distance]);
                            world.u64(cell + 8, key);
                            world.u64(cell + 16, object);
                            (key, object, distance) = (old_key, old_object, stored);
                        }
                        index += 1;
                        distance += 1;
                        assert!(
                            index < capacity + overflow && usize::from(distance) <= overflow + 1
                        );
                    }
                }
                for i in 0..count {
                    let (army, stat, member) =
                        (armies + i * ARMY, stats + i * STAT, members + i * MEMBER);
                    let handle = (i as u64 + 1) << 32 | 0x33;
                    world.u64(army, (base + carmy::VTABLE_RVA) as u64);
                    world.u64(army + cunit::HANDLE, handle);
                    world.u64(army + cunit::STAT_BLOCK, stat as u64);
                    world.u64(
                        army + carmy::ORDERS_GROUP,
                        (groups + (i % GROUPS) * 0x100) as u64,
                    );
                    world.u64(army + NAME_GROUP_MEMBER, member as u64);
                    world.u64(army + carmy::STRENGTH_FACTOR, 100_000);
                    world.u64(stat, (base + csub_unit_definition::VTABLE_RVA) as u64);
                    world.u64(stat + ARMOUR, 5 << 20);
                    world.u64(member, (base + name::VTABLE_RVA) as u64);
                    // Odd multiples of 13 have no template, so a null group.
                    if i % 26 != 13 {
                        world.u64(
                            member + name::TEMPLATE_REFERENCE,
                            (((i % template_count) as u64) + 1) << 32 | 52,
                        );
                    }
                    world.put(member + name::ORDER, &(i as i32 + 1).to_le_bytes());
                    world.put(member + name::FLAG_A, &[1, u8::from(i % 13 == 0)]);
                    world.u64(member + name::OVERRIDE_NAME + 24, 15);
                    // Custom names: some inline, some on the heap; programmatic
                    // copies hold generated text, which the game shows (and
                    // the reader takes) only for a null group (1409B5640).
                    if i % 20 == 0 || i % 13 == 0 {
                        world.string(member + name::OVERRIDE_NAME, b"Guards", 0);
                    } else if i % 97 == 0 {
                        world.string(
                            member + name::OVERRIDE_NAME,
                            b"Guards Rifle Division of the Line",
                            names + i * HEAP_NAME,
                        );
                    }
                    world.armies.push((army, handle));
                }
                world
            }
        }

        struct Counted<M> {
            inner: M,
            calls: Cell<usize>,
            bytes: Cell<usize>,
        }
        impl<M: Memory> Memory for Counted<M> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                self.bytes.set(self.bytes.get() + destination.len());
                self.inner.copy(address, destination)
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.calls.set(self.calls.get() + 1);
                self.bytes
                    .set(self.bytes.get() + destinations.iter().map(|d| d.len()).sum::<usize>());
                self.inner.copy_scattered(addresses, destinations)
            }
            fn copy_scattered_prefetched<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.copy_scattered(addresses, destinations)
            }
        }

        /// Naming operands with their text, as the replay row projects them.
        #[derive(Debug, PartialEq, Eq)]
        struct Plain {
            kind: i32,
            order: i32,
            ordered: u8,
            programmatic: u8,
            custom: crate::msvc_string::Text<256>,
            group: crate::msvc_string::Text<256>,
        }

        /// The pre-batching naming reader: one guarded read per field, the
        /// full registry walk and the group identity on every division; like
        /// the original, only the key text is reused within a pass.
        fn reference_naming(
            memory: &impl Memory,
            base: usize,
            member: usize,
            keys: &mut std::collections::BTreeMap<usize, crate::msvc_string::Text<256>>,
        ) -> Result<Plain, &'static str> {
            let order = name::order(memory, base, member)?;
            let kind = read::<i32>(memory, member + name::TYPE).ok_or("type")?;
            let [ordered, programmatic] =
                read::<[u8; 2]>(memory, member + name::FLAG_A).ok_or("flags")?;
            let mut row = Plain {
                kind,
                order,
                ordered,
                programmatic,
                custom: Default::default(),
                group: Default::default(),
            };
            if programmatic == 0 {
                let (source, len, _, _) = name::custom(memory, base, member)?;
                row.custom.len = len;
                name::copy_custom(memory, source, len, &mut row.custom.bytes, 0)?;
            }
            if kind == 0 && row.custom.len == 0 {
                let template =
                    read::<u64>(memory, member + name::TEMPLATE_REFERENCE).ok_or("template")?;
                let mut null = true;
                if template != 0
                    && let Some(object) = registry::resolve(memory, base, template)?
                {
                    assert_eq!(
                        crate::creferenced_division_template::identity(memory, base, object)?,
                        template
                    );
                    let group = read::<u64>(memory, object + name::TEMPLATE_NAME_GROUP)
                        .ok_or("group")? as usize;
                    if group != 0 {
                        null = false;
                        assert_eq!(
                            read::<u64>(memory, group),
                            Some((base + name::GROUP_VTABLE_RVA) as u64)
                        );
                        row.group = match keys.get(&group) {
                            Some(key) => *key,
                            None => {
                                let key =
                                    crate::msvc_string::read_text(memory, group + name::GROUP_KEY)
                                        .ok_or("key")?;
                                *keys.entry(group).or_insert(key)
                            }
                        };
                    }
                }
                // 1409B5640 shows a programmatic stored name for a null group.
                if null && programmatic != 0 {
                    let (source, len, _, _) = name::custom(memory, base, member)?;
                    row.custom.len = len;
                    name::copy_custom(memory, source, len, &mut row.custom.bytes, 0)?;
                }
            }
            Ok(row)
        }

        /// One pass's naming context, as the collector owns it.
        struct Pass {
            groups: name::GroupKeys,
            bytes: Vec<u8>,
            sources: crate::combat_sources::Sources,
        }
        impl Pass {
            fn new(world: &World) -> Self {
                Self::with(world, world.armies.len(), 1024)
            }
            fn with(world: &World, templates: usize, groups: usize) -> Self {
                Self {
                    groups: name::GroupKeys::with_capacity(templates, groups),
                    bytes: vec![0; world.armies.len() * 64 + 4096],
                    sources: crate::combat_sources::Sources::new(),
                }
            }
            fn plain(&self, row: name::Inputs) -> Plain {
                let text = |(start, len): (usize, usize)| {
                    let mut text = crate::msvc_string::Text::<256>::default();
                    text.bytes[..len].copy_from_slice(&self.bytes[start..start + len]);
                    text.len = len;
                    text
                };
                Plain {
                    kind: row.kind,
                    order: row.order,
                    ordered: row.ordered,
                    programmatic: row.programmatic,
                    custom: text(row.custom),
                    group: text(row.group),
                }
            }
        }

        /// Every division of one pass, committing each player's name as the
        /// collector does on admission. Returns the named divisions.
        fn pass(memory: &impl Memory, world: &World, context: &mut Pass) -> usize {
            context.groups.reset();
            let mut names = name::Names {
                bytes: &mut context.bytes,
                len: 0,
                required: 0,
            };
            let mut named = 0;
            for &(army, handle) in &world.armies {
                let row = capture_stats(
                    memory,
                    world.base(),
                    &mut context.sources,
                    army,
                    handle,
                    &mut context.groups,
                    &mut names,
                )
                .unwrap();
                let naming = row.naming.unwrap();
                names.len += naming.custom.1;
                named += usize::from(naming.group.1 != 0 || naming.custom.1 != 0);
            }
            named
        }

        /// Every division's naming text, read with `context` over two passes
        /// so the second runs on stale, generation-stamped memo entries.
        fn read_all(live: &impl Memory, world: &World, context: &mut Pass) -> Vec<Plain> {
            let mut rows = Vec::new();
            for _ in 0..2 {
                context.groups.reset();
                rows.clear();
                let mut names = name::Names {
                    bytes: &mut context.bytes,
                    len: 0,
                    required: 0,
                };
                let mut spans = Vec::new();
                for &(army, handle) in &world.armies {
                    let row = capture_stats(
                        live,
                        world.base(),
                        &mut context.sources,
                        army,
                        handle,
                        &mut context.groups,
                        &mut names,
                    )
                    .unwrap()
                    .naming
                    .unwrap();
                    names.len += row.custom.1;
                    spans.push(row);
                }
                rows.extend(spans.into_iter().map(|row| context.plain(row)));
            }
            rows
        }

        #[test]
        fn batched_naming_matches_the_per_field_reader_over_guarded_memory() {
            for templates in [64, 500] {
                let world = World::new(2_000, templates);
                // SAFETY: the synthetic graph is immutable, owned storage.
                let live = unsafe { crate::memory::LiveMemory::new() };
                let mut keys = Default::default();
                let expected: Vec<_> = world
                    .armies
                    .iter()
                    .map(|&(army, _)| {
                        let member = read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize;
                        reference_naming(&live, world.base(), member, &mut keys).unwrap()
                    })
                    .collect();
                // An unbounded memo, the collector's default sizing and a
                // memo of one slot per table, which is full after the first
                // template: all three name every division alike.
                for (templates, groups) in [(world.armies.len(), 1024), (1 << 16, 1 << 16), (1, 1)]
                {
                    let mut context = Pass::with(&world, templates, groups);
                    assert_eq!(
                        read_all(&live, &world, &mut context),
                        expected,
                        "{templates}/{groups}"
                    );
                }
                let mut kinds = [0; 4];
                for row in &expected {
                    kinds[match (row.programmatic, row.custom.len) {
                        (0, 0) => 0,
                        (0, _) => 1,
                        (_, 0) => 2,
                        _ => 3,
                    }] += 1;
                }
                // Historical, custom (inline and heap) and programmatic
                // members, with a group and with a null one.
                assert!(kinds.iter().all(|count| *count > 0), "{kinds:?}");
            }
        }

        #[test]
        fn a_key_that_does_not_fit_is_counted_once_per_pass() {
            // Divisions 1 to 3 are historical members of the one template.
            let world = World::new(4, 1);
            // SAFETY: the synthetic graph is immutable, owned storage.
            let live = unsafe { crate::memory::LiveMemory::new() };
            let mut groups = name::GroupKeys::with_capacity(4, 4);
            for pass in 0..2 {
                groups.reset();
                let mut names = name::Names {
                    bytes: &mut [0; 2],
                    len: 0,
                    required: 0,
                };
                for &(army, _) in &world.armies[1..] {
                    let member = read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize;
                    assert_eq!(
                        name::inputs(&live, world.base(), member, &mut groups, &mut names),
                        Ok(name::Naming::DoesNotFit),
                        "pass {pass}"
                    );
                }
                // "GRP_0": five bytes, however many divisions share it.
                assert_eq!((names.len, names.required), (0, 5), "pass {pass}");
            }
        }

        #[test]
        fn a_second_division_of_a_template_costs_only_the_member_copy() {
            let world = World::new(3, 1);
            // SAFETY: the synthetic graph is immutable, owned storage.
            let counted = Counted {
                inner: unsafe { crate::memory::LiveMemory::new() },
                calls: Cell::new(0),
                bytes: Cell::new(0),
            };
            let mut context = Pass::new(&world);
            let mut names = name::Names {
                bytes: &mut context.bytes,
                len: 0,
                required: 0,
            };
            let mut costs = Vec::new();
            // Divisions 1 and 2 are historical members of the one template.
            for &(army, _) in &world.armies[1..] {
                let member = read::<u64>(&counted, army + NAME_GROUP_MEMBER).unwrap() as usize;
                let before = counted.calls.get();
                let row = name::inputs(
                    &counted,
                    world.base(),
                    member,
                    &mut context.groups,
                    &mut names,
                );
                assert!(matches!(row, Ok(name::Naming::Row(row)) if row.group.1 != 0));
                costs.push(counted.calls.get() - before);
            }
            assert!(costs[0] > 1);
            assert_eq!(costs[1], 1);
        }

        #[test]
        #[ignore = "timing benchmark: cargo test -p engine --release scale -- --ignored --nocapture"]
        fn division_stats_and_naming_capture_cost_at_scale() {
            for (count, templates) in [(5_000, 64), (5_000, 1_250), (10_000, 64), (10_000, 2_500)] {
                let world = World::new(count, templates);
                let members: Vec<usize> = world
                    .armies
                    .iter()
                    .map(|&(army, _)| {
                        // SAFETY: the synthetic graph is immutable, owned storage.
                        let live = unsafe { crate::memory::LiveMemory::new() };
                        read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize
                    })
                    .collect();
                let context = std::cell::RefCell::new(Pass::new(&world));
                // Three passes over the same graph: the whole stats/name read,
                // the batched name read alone and the per-field name reader.
                // Timing runs through the counting adapter, which forwards
                // every copy to LiveMemory's guarded entries.
                type Run<'a> = (
                    &'a str,
                    &'a dyn Fn(&Counted<crate::memory::LiveMemory>) -> usize,
                );
                let runs: [Run<'_>; 3] = [
                    (
                        "capture_stats (identity, one 12-span scatter, commander, name)",
                        &|m| pass(m, &world, &mut context.borrow_mut()),
                    ),
                    ("batched name read", &|m| {
                        let mut context = context.borrow_mut();
                        let context = &mut *context;
                        context.groups.reset();
                        let mut names = name::Names {
                            bytes: &mut context.bytes,
                            len: 0,
                            required: 0,
                        };
                        let mut named = 0;
                        for &member in &members {
                            if let Ok(name::Naming::Row(row)) = name::inputs(
                                m,
                                world.base(),
                                member,
                                &mut context.groups,
                                &mut names,
                            ) {
                                names.len += row.custom.1;
                                named += 1;
                            }
                        }
                        named
                    }),
                    ("per-field name reader (before)", &|m| {
                        let mut keys = Default::default();
                        members
                            .iter()
                            .map(|&member| {
                                usize::from(
                                    reference_naming(m, world.base(), member, &mut keys).is_ok(),
                                )
                            })
                            .sum()
                    }),
                ];
                for (label, run) in runs {
                    // SAFETY: the synthetic graph is immutable, owned storage.
                    let counted = Counted {
                        inner: unsafe { crate::memory::LiveMemory::new() },
                        calls: Cell::new(0),
                        bytes: Cell::new(0),
                    };
                    assert_eq!(run(&counted), count, "{label}");
                    let (calls, bytes) = (counted.calls.get(), counted.bytes.get());
                    let mut best = u128::MAX;
                    for _ in 0..7 {
                        let started = std::time::Instant::now();
                        std::hint::black_box(run(&counted));
                        best = best.min(started.elapsed().as_nanos());
                    }
                    println!(
                        "{count} divisions, {templates} templates, {label}: {:.3} ms/pass best of 7, {:.0} ns/division, {:.2} guarded calls and {:.0} bytes per division",
                        best as f64 / 1e6,
                        best as f64 / count as f64,
                        calls as f64 / count as f64,
                        bytes as f64 / count as f64,
                    );
                }
            }
        }
    }
}
