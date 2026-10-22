//! Unit-leader base state, preserving full signed experience and skill widths.

use crate::{
    Memory, carmy_leader, ccharacter, cnavy_leader, coperative_leader, cpdx_array::Array,
    cunit_leader_skill, cunit_leader_trait, msvc_string, read as read_native, read_slice,
    unit_leader_trait_experience_entry, walk::Outcome,
};

/// CUnitLeader base vtable.
pub const VTABLE_RVA: usize = 0x0293_f420;
/// CUnitLeader reference type u32.
pub const REFERENCE_TYPE: u32 = 4713;
/// CUnitLeader reference type u32.
pub const TYPE: usize = 0x08;
/// CUnitLeader instance ID u32.
pub const ID: usize = 0x0c;
/// CUnitLeader MSVC string (`TOKEN`).
pub const TOKEN: usize = 0x20;
/// CUnitLeader inherited MSVC string (`NAME`).
pub const NAME: usize = 0x40;
/// Raw saved description token; localized into a separate client-language cache.
pub const DESCRIPTION_TOKEN: usize = 0x80;
/// CUnitLeader assigned CUnitLeaderTrait* array.
pub const TRAIT_DATA: usize = 0xdc8;
/// CUnitLeader assigned-trait capacity i32.
pub const TRAIT_CAPACITY: usize = 0xdd0;
/// CUnitLeader assigned-trait count i32.
pub const TRAIT_COUNT: usize = 0xdd4;
/// Timed-trait vector entries contain a definition and signed remaining days.
/// Trait assignment and daily updates mutate it; first-match queries retain
/// its stored order.
pub const TIMED_TRAIT_DATA: usize = 0xde0;
pub const TIMED_TRAIT_STRIDE: usize = 0x10;

/// CUnitLeader trait-experience entry array.
pub const TRAIT_EXPERIENCE_DATA: usize = 0xdf8;
/// CUnitLeader trait-experience count i32.
#[cfg(any(test, feature = "test-support"))]
pub const TRAIT_EXPERIENCE_COUNT: usize = 0xe04;
/// Ordered per-trait XP-factor overrides, independent of acquired traits and progress.
pub const TRAIT_EXPERIENCE_FACTORS: usize = 0xe10;
/// CUnitLeader CUnitLeaderSkill pointer.
pub const SKILL: usize = 0xe60;
/// CUnitLeader experience i64 /100000.
pub const EXPERIENCE: usize = 0xe68;
/// CUnitLeader skill-category i32.
pub const CATEGORY: usize = 0xe7c;
/// Raw country reference used by portrait-directory and visibility selection.
/// Capture does not change it.
pub const OWNER: usize = 0x120;
/// Stored signed exile-country reference used by the exile-trait flag builder.
pub const EXILE_COUNTRY: usize = 0xed4;
/// Visibility-choice pointer; its predicate is evaluated when non-null.
pub const VISIBILITY_CHOICE: usize = 0x18;
/// Template-visibility byte; when set and no direct choice exists, visibility
/// evaluates the resolved character template's choice.
pub const TEMPLATE_VISIBILITY: usize = 0xf50;
/// CUnitLeader copied character ID u32.
pub const CHARACTER_ID: usize = 0xf4c;
/// CUnitLeader copied character-reference type u32.
pub const CHARACTER_TYPE: usize = 0xf48;
/// CUnitLeader base constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00bf_ccb0;
/// CUnitLeader experience update.
pub const ADD_EXPERIENCE_RVA: usize = 0x00c0_5400;
/// CUnitLeader name/token updater.
pub const SET_NAME_RVA: usize = 0x00c1_6fb0;

/// Raw army-category formation reference used by late-join scaling and communications.
/// The caller supplies the authenticated kind and category from the established
/// field prefix. Other categories never access the army extension. Zero is a
/// known absent reference; no registry lookup or derived formation count occurs.
pub fn read_formation_reference(
    memory: &impl Memory,
    pointer: usize,
    kind: u8,
    category: i32,
) -> Result<Option<u64>, &'static str> {
    if kind != KIND_ARMY || category as u32 >= 2 {
        return Ok(None);
    }
    read_native(memory, at(pointer, carmy_leader::FORMATION_REFERENCE)?)
        .map(Some)
        .ok_or("unit leader formation reference unreadable")
}

/// Replay-side class discriminator; only the inherited fields exist for base and operative rows.
pub const KIND_BASE: u8 = 0;
/// Replay-side CArmyLeader discriminator, authenticated by its primary vtable.
pub const KIND_ARMY: u8 = 1;
/// Replay-side CNavyLeader discriminator, authenticated by its primary vtable.
pub const KIND_NAVY: u8 = 2;
/// Replay-side COperativeLeader discriminator, authenticated by its primary vtable.
pub const KIND_OPERATIVE: u8 = 3;

/// Definition identity and exact signed countdown of one `TIMED_TRAIT_DATA` entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TimedTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub remaining_days: i32,
}

/// Reads an already validated timed-trait header in stored order, including
/// duplicates and negative days. Queries select the first matching definition;
/// serialization preserves that order.
pub fn read_timed_traits(
    memory: &impl Memory,
    base: usize,
    array: Array,
    mut visit: impl FnMut(TimedTrait),
) -> Result<(), &'static str> {
    let expected = at(base, cunit_leader_trait::VTABLE_RVA)? as u64;
    let mut batch = [0_u8; 64 * TIMED_TRAIT_STRIDE];
    for first in (0..array.count).step_by(64) {
        let count = (array.count - first).min(64) as usize;
        let entries = &mut batch[..count * TIMED_TRAIT_STRIDE];
        let address = array
            .element(first, TIMED_TRAIT_STRIDE)
            .ok_or("unit leader timed trait address overflow")?;
        if !read_slice(memory, address, entries) {
            return Err("unit leader timed trait entries unreadable");
        }
        for entry in entries.chunks_exact(TIMED_TRAIT_STRIDE) {
            let pointer = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                .map_err(|_| "unit leader timed trait pointer overflow")?;
            let mut identity = [0; 12];
            let mut ordinal = [0; 4];
            let complete = memory.copy_scattered(
                [pointer, at(pointer, cunit_leader_trait::ORDINAL)?],
                &mut [&mut identity, &mut ordinal],
            );
            if complete == 0 {
                return Err("unit leader timed trait identity unreadable");
            }
            if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
                return Err("unit leader timed trait vtable mismatch");
            }
            if complete != 2 {
                return Err("unit leader timed trait identity unreadable");
            }
            visit(TimedTrait {
                token_id: i32::from_le_bytes(identity[8..].try_into().unwrap()),
                definition_ordinal: i32::from_le_bytes(ordinal),
                remaining_days: i32::from_le_bytes(entry[8..12].try_into().unwrap()),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub reference_type: u32,
    pub id: u32,
    pub character_type: u32,
    pub character_id: u32,
    pub category: i32,
    pub experience: i64,
    pub kind: u8,
    /// 0: null pointer, 1: native invalid definition, 2: native valid definition.
    pub skill_status: u8,
    /// Exact native valid byte, retained separately from the presence discriminator.
    pub skill_valid: u8,
    pub skill_level: i32,
    pub skill_required_experience: i32,
    pub skill_category: i32,
    /// Army: attack, defense, planning, logistics. Navy: attack, defense,
    /// maneuvering, coordination. Absent for base and operative kinds.
    pub components: [i32; 4],
    /// [`OWNER`], the raw country reference.
    pub owner: u32,
    /// [`carmy_leader::CAPTURED`]; zero for the other kinds, which have none.
    pub captured: u8,
    /// [`VISIBILITY_CHOICE`] is non-null.
    pub visibility_choice: u8,
    /// [`TEMPLATE_VISIBILITY`], as stored.
    pub template_visibility: u8,
    pub exile_country: i32,
}

/// Reads one CCharacter-owned role. Null role pointers are handled by the caller;
/// a non-null role with unreadable fields is never reported as absent. The base
/// and operative classes retain their shared state without reading army/navy extensions.
pub fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_character_id: u32,
) -> Result<Row, &'static str> {
    let (kind, reference_type, id) = identity(memory, base, pointer)?;
    let mut character = [0; 8];
    let mut category = [0; 4];
    let mut skill = [0; 8];
    let mut exile = [0; 4];
    if memory.copy_scattered(
        [
            at(pointer, CHARACTER_TYPE)?,
            at(pointer, CATEGORY)?,
            at(pointer, SKILL)?,
            at(pointer, EXILE_COUNTRY)?,
        ],
        &mut [&mut character, &mut category, &mut skill, &mut exile],
    ) != 4
    {
        return Err("unit leader state unreadable");
    }
    let character_type = u32::from_le_bytes(character[..4].try_into().unwrap());
    let character_id = u32::from_le_bytes(character[4..].try_into().unwrap());
    if character_type != ccharacter::REFERENCE_TYPE || character_id != expected_character_id {
        return Err("unit leader character reference mismatch");
    }
    let experience = if matches!(kind, KIND_ARMY | KIND_NAVY) {
        read_native::<i64>(memory, at(pointer, EXPERIENCE)?)
            .ok_or("unit leader experience unreadable")?
    } else {
        0
    };
    let mut row = Row {
        reference_type,
        id,
        character_type,
        character_id,
        category: i32::from_le_bytes(category),
        experience,
        kind,
        exile_country: i32::from_le_bytes(exile),
        ..Row::default()
    };
    let skill = usize::try_from(u64::from_le_bytes(skill))
        .map_err(|_| "unit leader skill pointer overflow")?;
    if skill != 0 {
        let expected = at(base, cunit_leader_skill::VTABLE_RVA)?;
        let mut vtable = [0; 8];
        let mut valid = [0; 1];
        let mut scalars = [0; 12];
        let completed = memory.copy_scattered(
            [
                skill,
                at(skill, cunit_leader_skill::VALID)?,
                at(skill, cunit_leader_skill::LEVEL)?,
            ],
            &mut [&mut vtable, &mut valid, &mut scalars],
        );
        if completed == 0 || u64::from_le_bytes(vtable) != expected as u64 {
            return Err("unit leader skill vtable mismatch");
        }
        if completed != 3 {
            return Err("unit leader skill fields unreadable");
        }
        row.skill_valid = valid[0];
        row.skill_status = if valid[0] == 0 { 1 } else { 2 };
        row.skill_level = i32::from_le_bytes(scalars[..4].try_into().unwrap());
        row.skill_required_experience = i32::from_le_bytes(scalars[4..8].try_into().unwrap());
        row.skill_category = i32::from_le_bytes(scalars[8..].try_into().unwrap());
    }
    let offsets = match kind {
        KIND_ARMY => Some([
            carmy_leader::ATTACK,
            carmy_leader::DEFENSE,
            carmy_leader::PLANNING,
            carmy_leader::LOGISTICS,
        ]),
        KIND_NAVY => Some([
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]),
        _ => None,
    };
    if let Some(offsets) = offsets {
        let mut a = [0; 4];
        let mut b = [0; 4];
        let mut c = [0; 4];
        let mut d = [0; 4];
        if memory.copy_scattered(
            [
                at(pointer, offsets[0])?,
                at(pointer, offsets[1])?,
                at(pointer, offsets[2])?,
                at(pointer, offsets[3])?,
            ],
            &mut [&mut a, &mut b, &mut c, &mut d],
        ) != 4
        {
            return Err("unit leader skill components unreadable");
        }
        row.components = [a, b, c, d].map(i32::from_le_bytes);
    }
    // Owner and visibility inputs, with the army leader's captured byte.
    let mut owner = [0; 4];
    let mut choice = [0; 8];
    let mut template = [0; 1];
    let mut captured = [0; 1];
    let (owner_at, choice_at, template_at) = (
        at(pointer, OWNER)?,
        at(pointer, VISIBILITY_CHOICE)?,
        at(pointer, TEMPLATE_VISIBILITY)?,
    );
    let complete = if kind == KIND_ARMY {
        memory.copy_scattered(
            [
                owner_at,
                choice_at,
                template_at,
                at(pointer, carmy_leader::CAPTURED)?,
            ],
            &mut [&mut owner, &mut choice, &mut template, &mut captured],
        ) == 4
    } else {
        memory.copy_scattered(
            [owner_at, choice_at, template_at],
            &mut [&mut owner, &mut choice, &mut template],
        ) == 3
    };
    if !complete {
        return Err("unit leader owner or visibility inputs unreadable");
    }
    row.owner = u32::from_le_bytes(owner);
    row.visibility_choice = u8::from(u64::from_le_bytes(choice) != 0);
    row.template_visibility = template[0];
    row.captured = captured[0];
    Ok(row)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRead {
    pub outcome: Outcome,
    /// Arena offset of the native `NAME`, which the unit-leader serializer does not emit.
    pub name_start: usize,
    pub name_len: usize,
    /// Arena offset of the native `TOKEN`, serialized as name. No script-token claim is made.
    pub token_start: usize,
    pub token_len: usize,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

/// Copies both exact byte strings after `read` authenticates the role. Native `TOKEN` is the saved
/// name; `NAME` remains a separate unsaved inherited string. Insufficient storage omits the whole
/// row; callers can grow the arena off-thread.
pub fn read_text(memory: &impl Memory, pointer: usize, arena: &mut [u8]) -> TextRead {
    let result = (|| {
        let name = literal(memory, at(pointer, NAME)?)?;
        let token = literal(memory, at(pointer, TOKEN)?)?;
        let (name_len, token_len) = (name.len, token.len);
        let required_bytes = name_len
            .checked_add(token_len)
            .ok_or("unit leader text length overflow")?;
        if required_bytes > arena.len() {
            return Ok(TextRead {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                name_start: 0,
                name_len,
                token_start: name_len,
                token_len,
                bytes_len: 0,
                required_bytes,
            });
        }
        if !name.copy(memory, &mut arena[..name_len])
            || !token.copy(memory, &mut arena[name_len..required_bytes])
        {
            return Err("unit leader text bytes unreadable");
        }
        Ok(TextRead {
            outcome: Outcome::Rows { len: 1 },
            name_start: 0,
            name_len,
            token_start: name_len,
            token_len,
            bytes_len: required_bytes,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| TextRead {
        outcome: Outcome::Unavailable { reason },
        name_start: 0,
        name_len: 0,
        token_start: 0,
        token_len: 0,
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// Exact inherited string bytes, including unsaved client-language text.
/// The caller has authenticated the leader. Commits a successful span.
pub fn read_name(
    memory: &impl Memory,
    pointer: usize,
    names: &mut crate::cname_group_member::Names<'_>,
) -> Result<Option<crate::cname_group_member::Span>, &'static str> {
    read_name_buffer(
        memory,
        pointer,
        crate::walk::Buffer::Fixed(names.bytes),
        &mut names.len,
        &mut names.required,
    )
}

pub fn read_name_buffer(
    memory: &impl Memory,
    pointer: usize,
    mut bytes: crate::walk::Buffer<'_, u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<crate::cname_group_member::Span>, &'static str> {
    let value = literal(memory, at(pointer, NAME)?)?;
    *required = required
        .checked_add(value.len)
        .ok_or("unit leader name byte count overflow")?;
    let end = len
        .checked_add(value.len)
        .ok_or("unit leader name byte count overflow")?;
    if !bytes.ensure(end)? {
        return Ok(None);
    }
    let start = *len;
    if !value.copy(memory, &mut bytes[start..end]) {
        return Err("unit leader name bytes unreadable");
    }
    *len = end;
    Ok(Some((start, value.len)))
}

/// Embedded operative codename operands; serialization and cached naming use
/// this member. Its descriptor sizes worker storage once.
pub fn read_codename(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    names: &mut crate::cname_group_member::Names<'_>,
) -> Result<crate::cname_group_member::Naming, &'static str> {
    read_codename_buffer(
        memory,
        base,
        pointer,
        crate::walk::Buffer::Fixed(names.bytes),
        &mut names.len,
        &mut names.required,
        &mut [false; 2],
    )
}

pub fn read_codename_buffer(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    bytes: crate::walk::Buffer<'_, u8>,
    len: &mut usize,
    required: &mut usize,
    heap: &mut [bool; 2],
) -> Result<crate::cname_group_member::Naming, &'static str> {
    use crate::cname_group_member::Naming;
    let result = crate::cname_group_member::codename_inputs_buffer(
        memory,
        base,
        at(pointer, coperative_leader::CODENAME)?,
        bytes,
        len,
        required,
        heap,
    )?;
    if let Naming::Row(row) = result {
        *len += row.custom.1;
    }
    Ok(result)
}

/// Operative leaders construct an embedded naming member of type two.
/// Category is independent of this layout.
pub const fn has_codename(kind: u8) -> bool {
    kind == KIND_OPERATIVE
}

/// UnitLeaders' complete live text demand, including kind-three naming
/// operands. The header-only capture uses no row or byte storage.
pub fn text_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let roster = crate::ccharacter_manager::Roster::read(memory, base, game_state)?;
    let mut cursor = roster.cursor();
    let mut required = 0_usize;
    while let Some(view) = cursor.next(memory, [false, false, true, false])? {
        let (character, _) = view.reference();
        let pointer = read_native::<u64>(&view, at(character, crate::ccharacter::UNIT_LEADER)?)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("unit leader pointer unreadable")?;
        if pointer == 0 {
            continue;
        }
        let (kind, _, _) = identity(memory, base, pointer)?;
        let text = read_text(memory, pointer, &mut []);
        if let Outcome::Unavailable { reason } = text.outcome {
            return Err(reason);
        }
        required = required.saturating_add(text.required_bytes);
        if has_codename(kind) {
            let mut names = crate::cname_group_member::Names {
                bytes: &mut [],
                len: 0,
                required: 0,
            };
            // Appended naming refusals must not suppress the old text demand.
            let _ = read_codename(memory, base, pointer, &mut names);
            required = required.saturating_add(names.required);
        }
    }
    Ok(required)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TraitRow {
    pub leader_id: u32,
    pub ordinal: u32,
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub token_start: usize,
    pub token_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub struct TraitExperienceRow {
    pub trait_row: TraitRow,
    /// Exact stored native i64 /100000 amount. Missing vector entries are not emitted as zero.
    pub experience: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraitWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    /// Exact for a complete walk. On truncation this is a lower bound: the
    /// copied prefix plus the first whole token that did not fit the arena.
    /// At row capacity no later token headers are read just to size the arena.
    pub required_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TraitDemand {
    pub rows: usize,
    pub bytes: usize,
}

/// Header-only demand for one complete vector, preserving repeated definitions.
/// Assigned-trait entries are pointers; trait-experience entries also carry a
/// signed fixed5 amount. Sizing inspects every token header after row capacity.
pub fn trait_demand(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    leader_id: u32,
    has_experience: bool,
) -> Result<TraitDemand, &'static str> {
    if identity(memory, base, pointer)?.2 != leader_id {
        return Err("unit leader trait parent identity mismatch");
    }
    let (offset, stride) = if has_experience {
        (
            TRAIT_EXPERIENCE_DATA,
            unit_leader_trait_experience_entry::STRIDE,
        )
    } else {
        (TRAIT_DATA, 8)
    };
    let array = Array::read(memory, at(pointer, offset)?, stride)
        .ok_or("unit leader trait vector invalid or unreadable")?;
    let mut bytes = 0_usize;
    for ordinal in 0..array.count {
        let definition = array
            .element(ordinal, stride)
            .and_then(|entry| read_native::<u64>(memory, entry))
            .and_then(|definition| usize::try_from(definition).ok())
            .ok_or("unit leader trait pointer unreadable")?;
        let (_, _, token) = trait_definition(memory, base, definition)?;
        token.validate_terminator(memory)?;
        bytes = bytes
            .checked_add(token.len)
            .ok_or("unit leader trait byte count overflow")?;
    }
    Ok(TraitDemand {
        rows: array.count as usize,
        bytes,
    })
}

fn trait_definition(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<(i32, i32, Literal), &'static str> {
    let expected = at(base, cunit_leader_trait::VTABLE_RVA)?;
    let mut header = [0; 12];
    let mut ordinal = [0; 4];
    let mut token_header = [0; 32];
    let completed = memory.copy_scattered(
        [
            definition,
            at(definition, cunit_leader_trait::ORDINAL)?,
            at(definition, cunit_leader_trait::TOKEN)?,
        ],
        &mut [&mut header, &mut ordinal, &mut token_header],
    );
    if completed == 0 {
        return Err("unit leader trait identity unreadable");
    }
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("unit leader trait vtable mismatch");
    }
    if completed != 3 {
        return Err("unit leader trait identity unreadable");
    }
    Ok((
        i32::from_le_bytes(header[8..].try_into().unwrap()),
        i32::from_le_bytes(ordinal),
        literal_metadata(at(definition, cunit_leader_trait::TOKEN)?, token_header)?,
    ))
}

/// Preserves the assigned vector order, including repeated definitions. Capacity
/// failures stop before the remaining rows and count them from the native header.
pub fn walk_traits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    leader_id: u32,
    output: &mut [TraitRow],
    arena: &mut [u8],
) -> TraitWalk {
    walk_definitions(
        memory,
        base,
        pointer,
        leader_id,
        TRAIT_DATA,
        output,
        arena,
        |row, _| row,
    )
}

/// Visits every native training entry in stored order, including repeated definitions.
/// The caller has authenticated this leader and its character backlink. Definitions
/// supply identity only; names, thresholds and effects resolve through content.
pub fn visit_trait_progress(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    visit: impl FnMut(i32, i32, i64),
) -> Result<usize, &'static str> {
    visit_training(memory, base, pointer, TRAIT_EXPERIENCE_DATA, visit)
}

/// Visits the complete ordered runtime XP-factor override vector.
pub fn visit_trait_factors(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    visit: impl FnMut(i32, i32, i64),
) -> Result<usize, &'static str> {
    visit_training(memory, base, pointer, TRAIT_EXPERIENCE_FACTORS, visit)
}
fn visit_training(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    offset: usize,
    mut visit: impl FnMut(i32, i32, i64),
) -> Result<usize, &'static str> {
    let array = Array::read(memory, at(pointer, offset)?, 16)
        .ok_or("unit leader training vector invalid or unreadable")?;
    let expected = at(base, cunit_leader_trait::VTABLE_RVA)? as u64;
    for ordinal in 0..array.count {
        let address = array
            .element(ordinal, 16)
            .ok_or("unit leader training entry address overflow")?;
        let entry = read_native::<[u8; 16]>(memory, address)
            .ok_or("unit leader training entry unreadable")?;
        let definition = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
            .map_err(|_| "unit leader training definition pointer overflow")?;
        let identity = read_native::<[u8; 12]>(memory, definition)
            .ok_or("unit leader training definition unreadable")?;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
            return Err("unit leader training definition vtable mismatch");
        }
        let definition_ordinal =
            read_native::<i32>(memory, at(definition, cunit_leader_trait::ORDINAL)?)
                .ok_or("unit leader training definition ordinal unreadable")?;
        visit(
            i32::from_le_bytes(identity[8..].try_into().unwrap()),
            definition_ordinal,
            i64::from_le_bytes(entry[8..].try_into().unwrap()),
        );
    }
    Ok(array.count as usize)
}

/// Reads the native trait-XP vector independently of the assigned-trait vector
/// and any selected UI. Definition identity and source order remain explicit.
#[cfg(test)]
pub fn walk_trait_experience(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    leader_id: u32,
    output: &mut [TraitExperienceRow],
    arena: &mut [u8],
) -> TraitWalk {
    walk_definitions(
        memory,
        base,
        pointer,
        leader_id,
        TRAIT_EXPERIENCE_DATA,
        output,
        arena,
        |trait_row, experience| TraitExperienceRow {
            trait_row,
            experience,
        },
    )
}

// All storage belongs to the caller. The only difference between these native
// containers is whether their element also carries the signed XP qword.
#[allow(clippy::too_many_arguments)]
fn walk_definitions<R>(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    leader_id: u32,
    offset: usize,
    output: &mut [R],
    arena: &mut [u8],
    make: impl Fn(TraitRow, i64) -> R,
) -> TraitWalk {
    let result = (|| {
        if identity(memory, base, pointer)?.2 != leader_id {
            return Err("unit leader trait parent identity mismatch");
        }
        let has_experience = offset == TRAIT_EXPERIENCE_DATA;
        let stride = if has_experience {
            unit_leader_trait_experience_entry::STRIDE
        } else {
            8
        };
        let array = Array::read(memory, at(pointer, offset)?, stride)
            .ok_or("unit leader trait vector invalid or unreadable")?;
        let mut len = 0;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0;
        for ordinal in 0..array.count {
            if len == output.len() {
                unread_rows = array.count - ordinal;
                break;
            }
            let entry = array
                .element(ordinal, stride)
                .ok_or("unit leader trait entry address overflow")?;
            let (definition, experience) = if has_experience {
                let bytes = read_native::<[u8; 16]>(memory, entry)
                    .ok_or("unit leader trait experience entry unreadable")?;
                (
                    u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                    i64::from_le_bytes(bytes[8..].try_into().unwrap()),
                )
            } else {
                (
                    read_native::<u64>(memory, entry)
                        .ok_or("unit leader trait pointer unreadable")?,
                    0,
                )
            };
            let definition =
                usize::try_from(definition).map_err(|_| "unit leader trait pointer overflow")?;
            // Exact fields share one guard, with the identity first. Do not
            // follow the string's heap pointer until this identity is checked.
            let (token_id, definition_ordinal, token) = trait_definition(memory, base, definition)?;
            let token_len = token.len;
            required_bytes = required_bytes
                .checked_add(token_len)
                .ok_or("unit leader trait byte count overflow")?;
            if token_len > arena.len() - bytes_len {
                token.validate_terminator(memory)?;
                unread_rows = array.count - ordinal;
                break;
            }
            token.copy_checked(memory, &mut arena[bytes_len..bytes_len + token_len])?;
            output[len] = make(
                TraitRow {
                    leader_id,
                    ordinal,
                    token_id,
                    definition_ordinal,
                    token_start: bytes_len,
                    token_len,
                },
                experience,
            );
            bytes_len += token_len;
            len += 1;
        }
        Ok(TraitWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| TraitWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("unit leader address overflow")
}

/// Authenticates any unit-leader subclass and returns its full stored handle,
/// also used by orders-group serialization. Callers needing only identity
/// avoid the full row walk.
pub fn handle(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    let (_, reference_type, id) = identity(memory, base, pointer)?;
    Ok(u64::from(reference_type) | (u64::from(id) << 32))
}

/// [`handle`] of every pointer of `pointers` into `output`, in order: a null
/// pointer is handle zero. The leaders are scattered objects, so their
/// identity headers are copied in one guarded, prefetched batch whose misses
/// overlap; they are then checked in order, so the first fault is that of a
/// read per leader.
pub fn handles(
    memory: &impl Memory,
    base: usize,
    pointers: &[usize],
    output: &mut [u64],
) -> Result<(), &'static str> {
    const BATCH: usize = crate::cpdx_array::POINTER_PAGE;
    let mut headers = [[0_u8; 16]; BATCH];
    for (pointers, output) in pointers.chunks(BATCH).zip(output.chunks_mut(BATCH)) {
        let count = pointers.len().min(output.len());
        let sources: [usize; BATCH] =
            std::array::from_fn(|index| pointers.get(index).copied().unwrap_or(0));
        let mut slots = headers.iter_mut().enumerate();
        let identified = memory
            .copy_scattered_prefetched(
                sources,
                &mut std::array::from_fn(|_| {
                    let (index, header) = slots.next().unwrap();
                    &mut header[..if index < count && sources[index] != 0 {
                        16
                    } else {
                        0
                    }]
                }),
            )
            .min(count);
        for (index, handle) in output[..count].iter_mut().enumerate() {
            *handle = if sources[index] == 0 {
                0
            } else if index >= identified {
                return Err("unit leader identity unreadable");
            } else {
                let (_, reference_type, id) = identity_from_header(base, headers[index])?;
                u64::from(reference_type) | (u64::from(id) << 32)
            };
        }
    }
    Ok(())
}

pub fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u8, u32, u32), &'static str> {
    let header =
        read_native::<[u8; 16]>(memory, pointer).ok_or("unit leader identity unreadable")?;
    identity_from_header(base, header)
}

/// [`identity`] over its 16 header bytes, already copied.
fn identity_from_header(base: usize, header: [u8; 16]) -> Result<(u8, u32, u32), &'static str> {
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    let relative = usize::try_from(vtable)
        .ok()
        .and_then(|value| value.checked_sub(base));
    let kind = match relative {
        Some(VTABLE_RVA) => KIND_BASE,
        Some(carmy_leader::VTABLE_RVA) => KIND_ARMY,
        Some(cnavy_leader::VTABLE_RVA) => KIND_NAVY,
        Some(coperative_leader::VTABLE_RVA) => KIND_OPERATIVE,
        _ => return Err("unit leader vtable mismatch"),
    };
    let reference_type = u32::from_le_bytes(header[TYPE..ID].try_into().unwrap());
    let id = u32::from_le_bytes(header[ID..].try_into().unwrap());
    if reference_type != REFERENCE_TYPE {
        return Err("unit leader reference type mismatch");
    }
    Ok((kind, reference_type, id))
}

struct Literal {
    header: [u8; 32],
    source: usize,
    len: usize,
    inline: bool,
}

impl Literal {
    fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> bool {
        if self.inline {
            output.copy_from_slice(&self.header[..self.len]);
            true
        } else {
            read_slice(memory, self.source, output)
        }
    }

    fn validate_terminator(&self, memory: &impl Memory) -> Result<(), &'static str> {
        let terminated = if self.inline {
            self.header[self.len] == 0
        } else {
            read_native::<u8>(memory, at(self.source, self.len)?) == Some(0)
        };
        if terminated {
            Ok(())
        } else {
            Err("unit leader string terminator invalid or unreadable")
        }
    }

    fn copy_checked(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        if self.inline {
            self.validate_terminator(memory)?;
            output.copy_from_slice(&self.header[..self.len]);
            return Ok(());
        }
        let mut terminator = [0; 1];
        let completed = memory.copy_scattered(
            [at(self.source, self.len)?, self.source],
            &mut [&mut terminator, output],
        );
        // A failed first span never visits payload memory. A readable nonzero
        // terminator retains precedence over a fault in the second span.
        if completed == 0 || terminator[0] != 0 {
            return Err("unit leader string terminator invalid or unreadable");
        }
        if completed != 2 {
            return Err("unit leader trait token bytes unreadable");
        }
        Ok(())
    }
}

fn literal(memory: &impl Memory, pointer: usize) -> Result<Literal, &'static str> {
    let header =
        read_native::<[u8; 32]>(memory, pointer).ok_or("unit leader string header unreadable")?;
    literal_header(memory, pointer, header)
}

fn literal_header(
    memory: &impl Memory,
    pointer: usize,
    header: [u8; 32],
) -> Result<Literal, &'static str> {
    let literal = literal_metadata(pointer, header)?;
    literal.validate_terminator(memory)?;
    Ok(literal)
}

fn literal_metadata(pointer: usize, header: [u8; 32]) -> Result<Literal, &'static str> {
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("unit leader string header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "unit leader string length overflow")?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    let source = if inline {
        pointer
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&value| value != 0)
            .ok_or("unit leader string pointer null or invalid")?
    };
    Ok(Literal {
        header,
        source,
        len,
        inline,
    })
}

#[cfg(test)]
mod tests;

/// CUnitLeader raw country reference.
pub const COUNTRY: usize = 0x120;
/// Serialization can omit these strings, and portrait fallback can consume
/// them when character resolution fails. Keep their exact bytes independently.
pub const PORTRAIT_STRINGS: [usize; 3] = [0xc0, 0xe0, 0x100];
#[cfg(test)]
#[path = "cunit_leader/hq_portrait_tests.rs"]
mod hq_portrait_tests;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortraitRow<T = crate::msvc_string::Text<256>> {
    pub leader: u64,
    pub character: u64,
    pub strings: [T; 3],
}
pub struct PortraitWalk {
    pub outcome: Outcome,
    pub sources: Outcome,
}

/// Fresh portrait inputs for every possible land HQ, independent of camera or
/// viewing country. `roots` must be a complete authenticated orders census
/// from this capture. A selected HQ requires both its unit and leader. `sources`
/// receives the distinct selected leaders in handle order for
/// [`walk_portrait_paths_memo`]; insufficient storage reports its demand.
pub fn walk_hq_portraits(
    memory: &impl Memory,
    base: usize,
    roots: &[crate::corders_group::Root],
    sources: &mut [(u64, usize)],
    output: &mut [PortraitRow],
) -> PortraitWalk {
    walk_hq_portraits_buffer(
        memory,
        base,
        roots,
        crate::walk::Buffer::Fixed(sources),
        crate::walk::Buffer::Fixed(output),
    )
}

pub fn walk_hq_portraits_growing(
    memory: &impl Memory,
    base: usize,
    roots: &[crate::corders_group::Root],
    sources: &mut Vec<(u64, usize)>,
    output: &mut Vec<PortraitRow<crate::capture_text::Owned>>,
) -> PortraitWalk {
    walk_hq_portraits_buffer(
        memory,
        base,
        roots,
        crate::walk::Buffer::Growing(sources),
        crate::walk::Buffer::Growing(output),
    )
}

fn walk_hq_portraits_buffer<T: crate::capture_text::Value>(
    memory: &impl Memory,
    base: usize,
    roots: &[crate::corders_group::Root],
    mut sources: crate::walk::Buffer<'_, (u64, usize)>,
    mut output: crate::walk::Buffer<'_, PortraitRow<T>>,
) -> PortraitWalk {
    use crate::corders_group::{HQ_PROVIDER, LEADER};
    let unavailable = |reason| PortraitWalk {
        outcome: Outcome::Unavailable { reason },
        sources: Outcome::Unavailable { reason },
    };
    // Select at most one leader per authenticated orders root.
    if let Err(reason) = sources.ensure(roots.len()) {
        return unavailable(reason);
    }
    let mut count = 0;
    for root in roots {
        let selected = (|| {
            let provider = read_native::<u64>(memory, at(root.pointer(), HQ_PROVIDER)?)
                .ok_or("HQ portrait provider unreadable")?;
            if provider == 0 {
                return Ok(None);
            }
            let pointer = read_native::<u64>(memory, at(root.pointer(), LEADER)?)
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("HQ portrait leader unreadable")?;
            if pointer == 0 {
                return Ok(None);
            }
            Ok(Some((handle(memory, base, pointer)?, pointer)))
        })();
        match selected {
            Err(reason) => return unavailable(reason),
            Ok(None) => {}
            Ok(Some(source)) => {
                if let Some(slot) = sources.get_mut(count) {
                    *slot = source;
                }
                count += 1;
            }
        }
    }
    if count > sources.len() {
        return PortraitWalk {
            outcome: Outcome::Unavailable {
                reason: "HQ portrait source census incomplete",
            },
            sources: Outcome::Truncated {
                len: sources.len(),
                unread_rows: (count - sources.len()) as u32,
            },
        };
    }
    // One leader may head several groups; one identity at two addresses is not native.
    sources[..count].sort_unstable();
    let mut unique = 0;
    for index in 0..count {
        let source = sources[index];
        if unique != 0 && sources[unique - 1].0 == source.0 {
            if sources[unique - 1].1 != source.1 {
                return unavailable("HQ leader identity has multiple pointers");
            }
            continue;
        }
        sources[unique] = source;
        unique += 1;
    }
    if let Err(reason) = output.ensure(unique) {
        return unavailable(reason);
    }
    let len = unique.min(output.len());
    let acquired = sources[..len]
        .iter()
        .zip(output.iter_mut())
        .try_for_each(|(&(leader, pointer), row)| read_portrait(memory, pointer, leader, row));
    PortraitWalk {
        outcome: match acquired {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(()) if len == unique => Outcome::Rows { len },
            Ok(()) => Outcome::Truncated {
                len,
                unread_rows: (unique - len) as u32,
            },
        },
        sources: Outcome::Rows { len: unique },
    }
}

fn read_portrait<T: crate::capture_text::Value>(
    memory: &impl Memory,
    pointer: usize,
    leader: u64,
    row: &mut PortraitRow<T>,
) -> Result<(), &'static str> {
    let character = read_native(memory, at(pointer, CHARACTER_TYPE)?)
        .ok_or("leader portrait character unreadable")?;
    for (lane, offset) in PORTRAIT_STRINGS.into_iter().enumerate() {
        row.strings[lane] = T::read(memory, at(pointer, offset)?, true).map_err(|_| {
            if T::GROWING {
                "leader portrait string invalid"
            } else {
                "leader portrait string invalid or exceeds256 bytes"
            }
        })?;
    }
    row.leader = leader;
    row.character = character;
    Ok(())
}

/// Picture-directory inputs: raw country reference, original-tag reference and
/// tag bytes. An unlinked or null country yields zero and an empty tag.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortraitPathRow<T = crate::msvc_string::Text<64>> {
    pub leader: u64,
    pub country: u32,
    pub tag_reference: u32,
    /// Country tag strings are a few bytes; longer text fails instead of truncating.
    pub tag: T,
}

type TagCell<T> = Option<(crate::ccountry::Country, u32, T)>;

/// Caller-owned per-walk tag memo, one cell per canonical country slot. Cells
/// are valid only for the walk that wrote them (epoch stamp), so nothing
/// carries across captures and no per-walk clearing is needed. A memo
/// smaller than the live country table records the demand and falls back to
/// the ordinary reads for slots it cannot hold; `grow` runs off-thread.
#[derive(Debug, Default)]
pub struct PortraitTagMemo<T = crate::msvc_string::Text<64>> {
    cells: Vec<(u32, TagCell<T>)>,
    epoch: u32,
    required: usize,
}

impl<T: crate::capture_text::Value> PortraitTagMemo<T> {
    /// Country-table slots observed by the latest walk, including slot zero.
    pub fn required(&self) -> usize {
        self.required
    }
    /// Resize to the observed demand. Allocates; call off the game thread.
    pub fn grow(&mut self) -> std::io::Result<()> {
        if self.cells.len() < self.required {
            crate::allocation::resize_default(&mut self.cells, self.required)?;
        }
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.cells.capacity() * std::mem::size_of::<(u32, TagCell<T>)>()
            + self
                .cells
                .iter()
                .filter_map(|(_, cell)| cell.as_ref().map(|(_, _, tag)| tag.allocated_bytes()))
                .sum::<usize>()
    }
    fn begin(&mut self, slots: u32) {
        self.required = self.required.max(slots as usize);
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.cells.fill_with(|| (0, None));
            self.epoch = 1;
        }
    }
    fn cell(&mut self, slot: u32) -> Option<&mut TagCell<T>> {
        let epoch = self.epoch;
        let (stamp, cell) = self.cells.get_mut(slot as usize)?;
        if *stamp != epoch {
            *stamp = epoch;
            *cell = None;
        }
        Some(cell)
    }
}

/// Test entry point with a memo already sized for 256 country slots, so read
/// counts match a recycled production capture.
#[cfg(test)]
pub fn walk_portrait_paths(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[(u64, usize)],
    output: &mut [PortraitPathRow],
) -> Outcome {
    let mut memo = PortraitTagMemo {
        required: 256,
        ..Default::default()
    };
    memo.grow().unwrap();
    walk_portrait_paths_memo(memory, base, game_state, sources, output, &mut memo)
}

/// Picture-directory inputs of `sources`, the distinct leaders a complete
/// [`walk_hq_portraits`] selected in this same quiescent capture.
pub fn walk_portrait_paths_memo(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[(u64, usize)],
    output: &mut [PortraitPathRow],
    memo: &mut PortraitTagMemo,
) -> Outcome {
    walk_portrait_paths_memo_buffer(
        memory,
        base,
        game_state,
        sources,
        crate::walk::Buffer::Fixed(output),
        memo,
    )
}

pub fn walk_portrait_paths_memo_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[(u64, usize)],
    output: &mut Vec<PortraitPathRow<crate::capture_text::Owned>>,
    memo: &mut PortraitTagMemo<crate::capture_text::Owned>,
) -> Outcome {
    walk_portrait_paths_memo_buffer(
        memory,
        base,
        game_state,
        sources,
        crate::walk::Buffer::Growing(output),
        memo,
    )
}

fn walk_portrait_paths_memo_buffer<T: crate::capture_text::Value>(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[(u64, usize)],
    mut output: crate::walk::Buffer<'_, PortraitPathRow<T>>,
    memo: &mut PortraitTagMemo<T>,
) -> Outcome {
    if sources.is_empty() {
        return Outcome::Rows { len: 0 };
    }
    let countries = match crate::ccountry::Reader::new(memory, base, game_state) {
        Ok(reader) => reader,
        Err(reason) => return Outcome::Unavailable { reason },
    };
    let Some(tags) = game_state
        .checked_add(crate::cgame_state::COUNTRY_TAGS)
        .and_then(|p| crate::cpdx_array::Array::read(memory, p, 0x20))
    else {
        return Outcome::Unavailable {
            reason: "country tag strings unreadable or invalid",
        };
    };
    // Tags are shared only within this quiescent walk. Keep every leader's
    // raw country/link authentication; remember only a fully acquired tag in
    // the country's own canonical-slot cell, keyed by the complete
    // authenticated country rather than its address.
    memo.begin(countries.count());
    if output.is_growing() {
        if let Err(reason) = output.ensure(sources.len()) {
            return Outcome::Unavailable { reason };
        }
        if memo.grow().is_err() {
            return Outcome::Unavailable {
                reason: "portrait tag memo allocation failed",
            };
        }
    }
    let mut visit = |expected, pointer, row: &mut PortraitPathRow<T>| {
        if handle(memory, base, pointer)? != expected {
            return Err("leader portrait path identity mismatch");
        }
        let raw = read_native::<u32>(memory, at(pointer, COUNTRY)?)
            .ok_or("leader portrait country unreadable")?;
        *row = PortraitPathRow {
            leader: expected,
            country: raw,
            ..Default::default()
        };
        let slot = if raw == 0 { None } else { countries.link(raw)? };
        if let Some(slot) = slot
            && let Some(country) = countries.at(slot)?
        {
            let cached = memo.cell(country.canonical_slot);
            if let Some(Some((known, reference, tag))) = cached.as_deref()
                && *known == country
            {
                row.tag_reference = *reference;
                row.tag = tag.copy_owned()?;
            } else {
                let original = read_native::<i32>(
                    memory,
                    country
                        .pointer
                        .checked_add(crate::ccountry::ORIGINAL_TAG)
                        .ok_or("leader portrait country address overflow")?,
                )
                .ok_or("leader portrait original tag unreadable")?;
                let reference = if original > 0 {
                    original as u32
                } else {
                    country.id
                };
                let address = tags
                    .element(reference, 0x20)
                    .ok_or("leader portrait tag reference outside tag strings")?;
                row.tag_reference = reference;
                row.tag = T::read(memory, address, false).map_err(|_| {
                    if T::GROWING {
                        "leader portrait tag string invalid"
                    } else {
                        "leader portrait tag string invalid or exceeds 64 bytes"
                    }
                })?;
                if let Some(cell) = cached {
                    *cell = Some((country, row.tag_reference, row.tag.copy_owned()?));
                }
            }
        }
        Ok(())
    };
    let len = sources.len().min(output.len());
    let result = sources
        .iter()
        .zip(output.iter_mut())
        .try_for_each(|(&(expected, pointer), row)| visit(expected, pointer, row));
    if let Err(reason) = result {
        return Outcome::Unavailable { reason };
    }
    output[..len].sort_unstable_by_key(|r| r.leader);
    if output[..len].windows(2).any(|p| p[0].leader == p[1].leader) {
        return Outcome::Unavailable {
            reason: "duplicate leader portrait path identity",
        };
    }
    if len == sources.len() {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: (sources.len() - len) as u32,
        }
    }
}
