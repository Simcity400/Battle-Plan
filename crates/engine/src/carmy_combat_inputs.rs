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
mod tests;
