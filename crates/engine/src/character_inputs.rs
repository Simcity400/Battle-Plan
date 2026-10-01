//! The character-scope trigger inputs the role tables do not hold, from one
//! pass over the character roster (`ccharacter_manager`): the trait arrays of
//! every country-leader, advisor and scientist role (CharacterRoleTraits) and
//! the unit-leader reference with the operands of the object it resolves to
//! (CharacterOperatives).
//!
//! `CHasTraitTrigger::Evaluate@1403D7D20` searches a character's unit leader's
//! traits, then (for a country-leader trait) every advisor role's
//! `CAdvisor+0xC8` and every country-leader role's `CCountryLeader+0x50`, then
//! (for a scientist trait) `CScientist+0x30` of `CCharacter+0xC0`. Its unit
//! leader, like that of `tag` (`CTagTrigger::Evaluate@1403DCDF0`, which
//! compares `CUnitLeader+0x120`) and `skill`, is
//! `CCharacter::GetUnitLeader@140F8A840`: `+0xA8`, else `+0xC8` resolved
//! through the reference registry. The roster's pointers are copied in pages
//! and each page's identities and role fields in one guarded gather; the maps,
//! roles and trait arrays are read as the role walks read them.

use crate::{
    Memory, cadvisor,
    ccharacter::{self, at},
    ccharacter_manager::Roster,
    ccountry_leader, character_advisor_node, character_leader_node,
    cpdx_array::Array,
    creference_registry, cscientist, cunit_leader, cunit_leader_skill, cunit_leader_trait,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_tree_node::Cursor as RoleTree,
    null_unit_leader_skill, read, read_slice,
    walk::Outcome,
};

/// The role a trait array belongs to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A country-leader role, by its ideology key in `CCharacter+0x98`.
    CountryLeader(i32),
    /// An advisor role, by its position in `CCharacter+0xB0`.
    Advisor(u32),
    /// The scientist role at `CCharacter+0xC0`.
    #[default]
    Scientist,
}

/// One trait of a role's array: its lexer token (`+0x8`) and, for a
/// country-leader trait, its database ordinal (`CCountryLeaderTrait+0x1A0`),
/// which tells definitions of one name apart.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Trait {
    pub token: u32,
    pub ordinal: Option<u32>,
}

/// One role's nonempty trait array: `traits[start..start + len]` in array
/// order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TraitRow {
    pub character: u32,
    pub role: Role,
    pub start: usize,
    pub len: usize,
}

/// The stored operands of the unit leader a reference resolves to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Leader {
    /// `CUnitLeader+0xE7C`.
    pub category: i32,
    /// `CUnitLeader+0x120`, the raw country reference.
    pub owner: u32,
    /// `*(CUnitLeader+0xE60)+0x1B8`; `None` for a null skill object.
    pub skill_level: Option<i32>,
}

/// One character's nonzero `CCharacter+0xC8`. With a resolved `leader`, its
/// assigned traits' definition ordinals are `definitions[start..start + len]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OperativeRow {
    pub character: u32,
    pub reference: u64,
    pub leader: Option<Leader>,
    pub start: usize,
    pub len: usize,
}

/// Caller-owned storage; the walk never allocates.
pub struct Buffers<'a> {
    pub traits: &'a mut [TraitRow],
    pub trait_values: &'a mut [Trait],
    pub operatives: &'a mut [OperativeRow],
    pub definitions: &'a mut [u32],
}

/// Outcomes of both tables. Rows that do not fit are counted (a row whose
/// values do not fit is not stored), and `required_*` count every value,
/// stored or not, so an empty buffer yields the live demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub traits: Outcome,
    pub operatives: Outcome,
    pub required_trait_values: usize,
    pub required_definitions: usize,
}

/// Roster pointers per page.
const PAGE: usize = 64;
/// Per character: the identity header, then `LEADER_MAP_HEAD..OPERATIVE + 8`.
const IDENTITY: usize = 16;
const ROLE_SPAN: usize = ccharacter::OPERATIVE + 8 - ccharacter::LEADER_MAP_HEAD;
const ROW: usize = IDENTITY + ROLE_SPAN;
const _: () = assert!(
    ccharacter::LEADER_MAP_SIZE == ccharacter::LEADER_MAP_HEAD + 8
        && ccharacter::UNIT_LEADER == ccharacter::LEADER_MAP_HEAD + 16
        && ccharacter::ADVISOR_MAP_HEAD == ccharacter::LEADER_MAP_HEAD + 24
        && ccharacter::ADVISOR_MAP_SIZE == ccharacter::LEADER_MAP_HEAD + 32
        && ccharacter::SCIENTIST == ccharacter::LEADER_MAP_HEAD + 40
        && ccharacter::OPERATIVE == ccharacter::LEADER_MAP_HEAD + 48
        && ROW == 72
);
const FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: IDENTITY,
    },
    RecordField {
        source_offset: ccharacter::LEADER_MAP_HEAD,
        destination_offset: IDENTITY,
        len: ROLE_SPAN,
    },
];
const LAYOUT: RecordLayout<'static> = match RecordLayout::new(&FIELDS, ROW) {
    Some(layout) => layout,
    None => panic!("character input layout"),
};
/// Trait pointers copied per guarded read.
const TRAITS_PER_READ: usize = 32;
/// The two role maps of each character, in the order they are walked.
const MAPS: [usize; 2] = [ccharacter::LEADER_MAP_HEAD, ccharacter::ADVISOR_MAP_HEAD];
/// A map's head node up to its nil byte, which `RoleTree` checks.
const SENTINEL: usize = 26;
const SENTINEL_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: SENTINEL,
}];
const SENTINEL_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&SENTINEL_FIELDS, SENTINEL) {
    Some(layout) => layout,
    None => panic!("role map sentinel layout"),
};

/// One table's rows, value arena and its own first failure.
struct Table<'a, R, V> {
    rows: &'a mut [R],
    values: &'a mut [V],
    len: usize,
    values_len: usize,
    required: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl<'a, R, V> Table<'a, R, V> {
    fn new(rows: &'a mut [R], values: &'a mut [V]) -> Self {
        Self {
            rows,
            values,
            len: 0,
            values_len: 0,
            required: 0,
            unread_rows: 0,
            failure: None,
        }
    }

    /// Room for one more row with `count` values, which the demand counts
    /// either way. `None` counts the row as unread; once one row does not fit,
    /// no later row is stored.
    fn reserve(&mut self, count: usize) -> Result<Option<usize>, &'static str> {
        self.required = self
            .required
            .checked_add(count)
            .ok_or("character input value count overflow")?;
        if self.unread_rows == 0
            && self.len < self.rows.len()
            && count <= self.values.len() - self.values_len
        {
            let start = self.values_len;
            self.values_len += count;
            return Ok(Some(start));
        }
        self.unread_rows = self
            .unread_rows
            .checked_add(1)
            .ok_or("character input row count overflow")?;
        Ok(None)
    }

    fn push(&mut self, row: R) {
        self.rows[self.len] = row;
        self.len += 1;
    }

    /// Runs one step of this table, unless it already failed; a failure here
    /// fails this table alone.
    fn run(&mut self, step: impl FnOnce(&mut Self) -> Result<(), &'static str>) {
        if self.failure.is_none()
            && let Err(reason) = step(self)
        {
            self.failure = Some(reason);
        }
    }

    /// This table's outcome; `shared` is a failure of the walk both tables
    /// share, which a table that already failed on its own does not take.
    fn outcome(&self, shared: Option<&'static str>) -> Outcome {
        match self.failure.or(shared) {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread_rows == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread_rows,
            },
        }
    }
}

/// Walks the whole roster in native order (historical, then dynamic). An
/// unreadable roster, page or unauthenticated character fails both tables; a
/// role, trait or resolved leader that does not read fails its own table.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, buffers: Buffers<'_>) -> Walk {
    let mut traits = Table::new(buffers.traits, buffers.trait_values);
    let mut operatives = Table::new(buffers.operatives, buffers.definitions);
    let mut resolver = creference_registry::HeaderResolver::default();
    let mut page_pointers = [0_usize; PAGE];
    let mut page_records = [0_u8; PAGE * ROW];
    let mut page_heads = [0_usize; PAGE * MAPS.len()];
    let mut page_sentinels = [0_u8; PAGE * MAPS.len() * SENTINEL];
    let result = (|| {
        let character_vtable = at(base, ccharacter::VTABLE_RVA)? as u64;
        let roster = Roster::read(memory, base, game_state)?;
        for (array, _) in roster.sections() {
            let mut first = 0;
            while first < array.count {
                let count = ((array.count - first) as usize).min(PAGE);
                let pointers = &mut page_pointers[..count];
                copy_pointers(memory, array.slice(first, count as u32, 8)?, pointers)?;
                let records = &mut page_records[..count * ROW];
                if memory.copy_records(RecordSources::Pointers(pointers), &LAYOUT, records)
                    != count * FIELDS.len()
                {
                    return Err("character identity or role fields unreadable");
                }
                if records.chunks_exact(ROW).any(|record| {
                    u64_at(record, 0) != character_vtable
                        || u32_at(record, ccharacter::TYPE) != ccharacter::REFERENCE_TYPE
                }) {
                    return Err("character vtable or reference type mismatch");
                }
                // Every map is checked by its header and sentinel, the empty
                // ones included, as RoleTree::read checks it.
                let sentinels = &mut page_sentinels[..count * MAPS.len() * SENTINEL];
                traits.run(|_| {
                    let heads = &mut page_heads[..count * MAPS.len()];
                    for (index, head) in heads.iter_mut().enumerate() {
                        let record = &records[index / MAPS.len() * ROW..][..ROW];
                        *head = RoleTree::header(map_header(record, MAPS[index % MAPS.len()]))?.0;
                    }
                    if memory.copy_records(
                        RecordSources::Pointers(heads),
                        &SENTINEL_LAYOUT,
                        sentinels,
                    ) != heads.len()
                    {
                        return Err("character role map sentinel unreadable");
                    }
                    Ok(())
                });
                for (index, (&pointer, record)) in
                    pointers.iter().zip(records.chunks_exact(ROW)).enumerate()
                {
                    let character = u32_at(record, ccharacter::ID);
                    let field = |offset: usize| {
                        u64_at(record, IDENTITY + offset - ccharacter::LEADER_MAP_HEAD)
                    };
                    let maps: [([u8; 16], [u8; SENTINEL]); 2] = std::array::from_fn(|map| {
                        let at = (index * MAPS.len() + map) * SENTINEL;
                        (
                            map_header(record, MAPS[map]),
                            sentinels[at..at + SENTINEL].try_into().unwrap(),
                        )
                    });
                    traits.run(|table| {
                        role_traits(memory, base, pointer, character, maps, &field, table)
                    });
                    let reference = field(ccharacter::OPERATIVE);
                    if reference != 0 {
                        operatives.run(|table| {
                            operative(memory, base, &mut resolver, character, reference, table)
                        });
                    }
                }
                first += count as u32;
            }
        }
        Ok(())
    })();
    let shared = result.err();
    Walk {
        traits: traits.outcome(shared),
        operatives: operatives.outcome(shared),
        required_trait_values: traits.required,
        required_definitions: operatives.required,
    }
}

/// A role map's 16-byte header (head, size) in a gathered record.
fn map_header(record: &[u8], map: usize) -> [u8; 16] {
    let at = IDENTITY + map - ccharacter::LEADER_MAP_HEAD;
    record[at..at + 16].try_into().unwrap()
}

/// The trait arrays of one character's country-leader, advisor and scientist
/// roles, in that order. `maps` holds each map's gathered header and
/// sentinel, which are checked before an empty map is accepted; `field`
/// holds the scientist pointer. Each role is authenticated by its vtable and
/// parent character, as the role walks authenticate it.
fn role_traits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    [leaders, advisors]: [([u8; 16], [u8; SENTINEL]); 2],
    field: &impl Fn(usize) -> u64,
    table: &mut Table<'_, TraitRow, Trait>,
) -> Result<(), &'static str> {
    let country_leader_trait = at(base, ccountry_leader::TRAIT_VTABLE_RVA)? as u64;
    let mut leaders = RoleTree::from_parts(leaders.0, leaders.1)?;
    let mut advisors = RoleTree::from_parts(advisors.0, advisors.1)?;
    if leaders.count() != 0 {
        let vtable = at(base, ccountry_leader::VTABLE_RVA)? as u64;
        while let Some(node) = leaders.next(memory)? {
            let value = read::<[u8; 16]>(memory, at(node, character_leader_node::IDEOLOGY)?)
                .ok_or("country leader ideology unreadable")?;
            let ideology = i32::from_le_bytes(value[..4].try_into().unwrap());
            let role = non_null(u64_at(&value, 8), "country leader role unreadable or null")?;
            let header =
                read::<[u8; 16]>(memory, role).ok_or("country leader role header unreadable")?;
            if u64_at(&header, 0) != vtable
                || u64_at(&header, ccountry_leader::CHARACTER) != pointer as u64
            {
                return Err("country leader role vtable or character parent mismatch");
            }
            let array = Array::read(memory, at(role, ccountry_leader::TRAITS)?, 8)
                .ok_or("country leader trait array unreadable or invalid")?;
            push_traits(
                memory,
                array,
                country_leader_trait,
                true,
                table,
                character,
                Role::CountryLeader(ideology),
            )?;
        }
    }
    if advisors.count() != 0 {
        let vtable = at(base, cadvisor::VTABLE_RVA)? as u64;
        let mut ordinal = 0;
        while let Some(node) = advisors.next(memory)? {
            let role = read::<u64>(memory, at(node, character_advisor_node::ADVISOR)?)
                .ok_or("advisor role unreadable or null")?;
            let role = non_null(role, "advisor role unreadable or null")?;
            let header = read::<[u8; 24]>(memory, role).ok_or("advisor role fields unreadable")?;
            if u64_at(&header, 0) != vtable
                || u64_at(&header, cadvisor::CHARACTER) != pointer as u64
            {
                return Err("advisor role vtable or character parent mismatch");
            }
            let array = Array::read(memory, at(role, cadvisor::TRAITS)?, 8)
                .ok_or("advisor trait array unreadable or invalid")?;
            push_traits(
                memory,
                array,
                country_leader_trait,
                true,
                table,
                character,
                Role::Advisor(ordinal),
            )?;
            ordinal += 1;
        }
    }
    let scientist = field(ccharacter::SCIENTIST);
    if scientist != 0 {
        let scientist = non_null(scientist, "scientist pointer overflow")?;
        if read::<u64>(memory, scientist) != Some(at(base, cscientist::VTABLE_RVA)? as u64) {
            return Err("scientist vtable mismatch");
        }
        let array = Array::read(memory, at(scientist, cscientist::TRAIT_DATA)?, 8)
            .ok_or("scientist trait array unreadable or invalid")?;
        push_traits(
            memory,
            array,
            at(base, cscientist::TRAIT_VTABLE_RVA)? as u64,
            false,
            table,
            character,
            Role::Scientist,
        )?;
    }
    Ok(())
}

fn non_null(value: u64, reason: &'static str) -> Result<usize, &'static str> {
    usize::try_from(value)
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or(reason)
}

/// One nonempty trait array as a row: each element must be a non-null trait
/// of `vtable`, whose token is at `+0x8`; a country-leader trait's
/// (`ordinal`) database ordinal is read with it.
fn push_traits(
    memory: &impl Memory,
    array: Array,
    vtable: u64,
    ordinal: bool,
    table: &mut Table<'_, TraitRow, Trait>,
    character: u32,
    role: Role,
) -> Result<(), &'static str> {
    const _: () = assert!(ccountry_leader::TRAIT_TOKEN == 8 && cscientist::TRAIT_TOKEN == 8);
    if array.count == 0 {
        return Ok(());
    }
    let len = array.count as usize;
    let Some(start) = table.reserve(len)? else {
        return Ok(());
    };
    elements(
        memory,
        array,
        &mut table.values[start..start + len],
        |pointer| {
            let mut header = [0; 12];
            let mut definition = [0; 4];
            let read = if ordinal {
                memory.copy_scattered(
                    [pointer, at(pointer, ccountry_leader::TRAIT_ORDINAL)?],
                    &mut [&mut header, &mut definition],
                ) == 2
            } else {
                memory.copy(pointer, &mut header)
            };
            if !read {
                return Err("trait unreadable");
            }
            if u64_at(&header, 0) != vtable {
                return Err("trait vtable mismatch");
            }
            Ok(Trait {
                token: u32_at(&header, 8),
                ordinal: ordinal.then(|| u32::from_le_bytes(definition)),
            })
        },
    )?;
    table.push(TraitRow {
        character,
        role,
        start,
        len,
    });
    Ok(())
}

/// Each element pointer of `array`, copied in batches, through `value`.
fn elements<V>(
    memory: &impl Memory,
    array: Array,
    output: &mut [V],
    mut value: impl FnMut(usize) -> Result<V, &'static str>,
) -> Result<(), &'static str> {
    let mut pointers = [0_u8; TRAITS_PER_READ * 8];
    for (batch, output) in output.chunks_mut(TRAITS_PER_READ).enumerate() {
        let window = array.slice((batch * TRAITS_PER_READ) as u32, output.len() as u32, 8)?;
        let bytes = &mut pointers[..output.len() * 8];
        if !read_slice(memory, window.data, bytes) {
            return Err("trait pointers unreadable");
        }
        for (slot, pointer) in output.iter_mut().zip(bytes.chunks_exact(8)) {
            let pointer = u64::from_le_bytes(pointer.try_into().unwrap());
            *slot = value(non_null(pointer, "trait pointer null")?)?;
        }
    }
    Ok(())
}

/// One nonzero `CCharacter+0xC8` and, when the registry holds its object,
/// that unit leader's category, owner, skill level and assigned trait
/// definitions.
fn operative(
    memory: &impl Memory,
    base: usize,
    resolver: &mut creference_registry::HeaderResolver,
    character: u32,
    reference: u64,
    table: &mut Table<'_, OperativeRow, u32>,
) -> Result<(), &'static str> {
    let Some(object) = resolver.resolve(memory, base, reference)? else {
        if let Some(start) = table.reserve(0)? {
            table.push(OperativeRow {
                character,
                reference,
                leader: None,
                start,
                len: 0,
            });
        }
        return Ok(());
    };
    cunit_leader::identity(memory, base, object)?;
    let mut owner = [0; 4];
    let mut traits = [0; 16];
    let mut skill = [0; 8];
    let mut category = [0; 4];
    if memory.copy_scattered(
        [
            at(object, cunit_leader::OWNER)?,
            at(object, cunit_leader::TRAIT_DATA)?,
            at(object, cunit_leader::SKILL)?,
            at(object, cunit_leader::CATEGORY)?,
        ],
        &mut [&mut owner, &mut traits, &mut skill, &mut category],
    ) != 4
    {
        return Err("operative unit leader fields unreadable");
    }
    let skill = u64::from_le_bytes(skill);
    let skill_level = if skill == 0 {
        None
    } else {
        let skill = non_null(skill, "operative skill pointer overflow")?;
        let mut vtable = [0; 8];
        let mut level = [0; 4];
        if memory.copy_scattered(
            [skill, at(skill, cunit_leader_skill::LEVEL)?],
            &mut [&mut vtable, &mut level],
        ) != 2
        {
            return Err("operative skill unreadable");
        }
        // GetSkillLevel@140C0A4A0 reads the level of either class.
        let vtable = u64::from_le_bytes(vtable);
        if vtable != at(base, cunit_leader_skill::VTABLE_RVA)? as u64
            && vtable != at(base, null_unit_leader_skill::VTABLE_RVA)? as u64
        {
            return Err("operative skill vtable mismatch");
        }
        Some(i32::from_le_bytes(level))
    };
    let array = Array::from_header(traits, 8).ok_or("operative trait array invalid")?;
    let len = array.count as usize;
    let Some(start) = table.reserve(len)? else {
        return Ok(());
    };
    let trait_vtable = at(base, cunit_leader_trait::VTABLE_RVA)? as u64;
    elements(
        memory,
        array,
        &mut table.values[start..start + len],
        |pointer| {
            let mut vtable = [0; 8];
            let mut ordinal = [0; 4];
            if memory.copy_scattered(
                [pointer, at(pointer, cunit_leader_trait::ORDINAL)?],
                &mut [&mut vtable, &mut ordinal],
            ) != 2
            {
                return Err("operative trait unreadable");
            }
            if u64::from_le_bytes(vtable) != trait_vtable {
                return Err("operative trait vtable mismatch");
            }
            Ok(u32::from_le_bytes(ordinal))
        },
    )?;
    table.push(OperativeRow {
        character,
        reference,
        leader: Some(Leader {
            category: i32::from_le_bytes(category),
            owner: u32::from_le_bytes(owner),
            skill_level,
        }),
        start,
        len,
    });
    Ok(())
}

fn copy_pointers(
    memory: &impl Memory,
    array: Array,
    output: &mut [usize],
) -> Result<(), &'static str> {
    let source = array
        .slice(0, output.len() as u32, 8)
        .map_err(|_| "character roster pointers unreadable")?;
    // Validate in element order: an earlier null wins over a later unreadable pointer.
    crate::cpdx_array::pointers(
        memory,
        source,
        "character roster pointers unreadable",
        |index, pointer| {
            if pointer == 0 {
                return Err("character roster pointer unreadable or null");
            }
            output[index as usize] = pointer;
            Ok(())
        },
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ccharacter_manager, cgame_state, coperative_leader, msvc_tree_node as node,
        reference_bucket as bucket,
    };
    use std::collections::BTreeMap;

    const GS: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const A: usize = 0x3000;
    const B: usize = 0x3400;
    const C: usize = 0x3800;
    const TABLE: usize = 0x8000;
    const ENTRIES: usize = 0x8100;
    const OPERATIVE: usize = 0x7000;
    const RESOLVED: u64 = (77 << 32) | cunit_leader::REFERENCE_TYPE as u64;
    const DANGLING: u64 = (78 << 32) | cunit_leader::REFERENCE_TYPE as u64;

    #[derive(Default)]
    struct Sparse(BTreeMap<usize, u8>);

    impl Sparse {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, &b)| (at + i, b)));
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, pointers: &[usize]) {
            self.u64(at, data as u64);
            self.u32(at + 8, pointers.len() as u32);
            self.u32(at + 12, pointers.len() as u32);
            for (index, &pointer) in pointers.iter().enumerate() {
                self.u64(data + index * 8, pointer as u64);
            }
        }
        /// A map at `character + offset` holding one node.
        fn one_node(&mut self, character: usize, offset: usize, head: usize, entry: usize) {
            self.u64(character + offset, head as u64);
            self.u64(character + offset + 8, 1);
            for link in [node::LEFT, node::PARENT, node::RIGHT] {
                self.u64(head + link, entry as u64);
                self.u64(entry + link, head as u64);
            }
            self.put(head + node::COLOR, &[1, 1]);
            self.put(entry + node::COLOR, &[0, 0]);
        }
        /// An empty map at `character + offset`: the sentinel links itself.
        fn empty_map(&mut self, character: usize, offset: usize, head: usize) {
            self.u64(character + offset, head as u64);
            self.u64(character + offset + 8, 0);
            for link in [node::LEFT, node::PARENT, node::RIGHT] {
                self.u64(head + link, head as u64);
            }
            self.put(head + node::COLOR, &[1, 1]);
        }
        fn object(&mut self, at: usize, vtable: usize, token_or_parent: u64) {
            self.u64(at, vtable as u64);
            self.u64(at + 8, token_or_parent);
        }
        fn character(&mut self, at: usize, id: u32) {
            self.u64(at, ccharacter::VTABLE_RVA as u64);
            self.u32(at + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
            self.u32(at + ccharacter::ID, id);
            self.put(at + ccharacter::LEADER_MAP_HEAD, &[0; ROLE_SPAN]);
        }
        fn bucket(&mut self, handle: u64, object: usize) {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            let at = ENTRIES + index * bucket::STRIDE;
            self.put(at + bucket::DISTANCE, &[1]);
            self.u64(at + bucket::TYPE, handle);
            self.u64(at + bucket::OBJECT, object as u64);
        }
    }

    impl Memory for Sparse {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            destination.iter_mut().enumerate().all(|(i, byte)| {
                self.0
                    .get(&(address + i))
                    .map(|value| *byte = *value)
                    .is_some()
            })
        }
    }

    /// A (id 11): a country-leader role of ideology 5 with traits 101 and
    /// 102, an advisor role with trait 102 and a scientist with trait 201.
    /// B (12): an operative reference that resolves; C (13, dynamic): one that
    /// does not.
    fn world() -> Sparse {
        let mut m = Sparse::default();
        m.u64(GS + cgame_state::CHARACTER_MANAGER, MANAGER as u64);
        m.u64(MANAGER, ccharacter_manager::VTABLE_RVA as u64);
        m.array(
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            0x2100,
            &[A, B],
        );
        m.array(
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            0x2200,
            &[C],
        );
        for (index, (at, id)) in [(A, 11), (B, 12), (C, 13)].into_iter().enumerate() {
            m.character(at, id);
            for (map, offset) in MAPS.into_iter().enumerate() {
                m.empty_map(at, offset, 0x4800 + (index * 2 + map) * 0x40);
            }
        }
        let leader_trait = ccountry_leader::TRAIT_VTABLE_RVA;
        m.object(0x6000, leader_trait, 101);
        m.object(0x6100, leader_trait, 102);
        m.u32(0x6000 + ccountry_leader::TRAIT_ORDINAL, 1);
        m.u32(0x6100 + ccountry_leader::TRAIT_ORDINAL, 2);
        m.object(0x6200, cscientist::TRAIT_VTABLE_RVA, 201);
        m.one_node(A, ccharacter::LEADER_MAP_HEAD, 0x4000, 0x4040);
        m.u64(0x4040 + character_leader_node::IDEOLOGY, 5);
        m.u64(0x4040 + character_leader_node::LEADER, 0x5000);
        m.object(0x5000, ccountry_leader::VTABLE_RVA, A as u64);
        m.array(0x5000 + ccountry_leader::TRAITS, 0x5100, &[0x6000, 0x6100]);
        m.one_node(A, ccharacter::ADVISOR_MAP_HEAD, 0x4100, 0x4140);
        m.u64(0x4140 + character_advisor_node::ADVISOR, 0x5200);
        m.object(0x5200, cadvisor::VTABLE_RVA, 0);
        m.u64(0x5200 + cadvisor::CHARACTER, A as u64);
        m.array(0x5200 + cadvisor::TRAITS, 0x5300, &[0x6100]);
        m.u64(A + ccharacter::SCIENTIST, 0x5400);
        m.u64(0x5400, cscientist::VTABLE_RVA as u64);
        m.array(0x5400 + cscientist::TRAIT_DATA, 0x5500, &[0x6200]);
        m.u64(B + ccharacter::OPERATIVE, RESOLVED);
        m.u64(C + ccharacter::OPERATIVE, DANGLING);
        m.u64(creference_registry::HIGH_ROOT_RVA, TABLE as u64);
        m.u64(TABLE + creference_registry::ENTRIES, ENTRIES as u64);
        m.u32(TABLE + creference_registry::COUNT, 1);
        m.u32(TABLE + creference_registry::MASK, 7);
        m.put(TABLE + creference_registry::OVERFLOW_DISTANCE, &[5]);
        m.put(ENTRIES, &[0; 14 * bucket::STRIDE]);
        m.put(ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        m.bucket(RESOLVED, OPERATIVE);
        m.u64(OPERATIVE, coperative_leader::VTABLE_RVA as u64);
        m.u32(OPERATIVE + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
        m.u32(OPERATIVE + cunit_leader::ID, 77);
        m.u32(OPERATIVE + cunit_leader::OWNER, 9);
        m.array(OPERATIVE + cunit_leader::TRAIT_DATA, 0x7100, &[0x7200]);
        m.u64(OPERATIVE + cunit_leader::SKILL, 0x7300);
        m.u32(OPERATIVE + cunit_leader::CATEGORY, 3);
        m.u64(0x7200, cunit_leader_trait::VTABLE_RVA as u64);
        m.u32(0x7200 + cunit_leader_trait::ORDINAL, 3);
        m.u64(0x7300, cunit_leader_skill::VTABLE_RVA as u64);
        m.u32(0x7300 + cunit_leader_skill::LEVEL, 4);
        m
    }

    struct Output {
        walk: Walk,
        traits: Vec<(u32, Role, Vec<Trait>)>,
        operatives: Vec<(u32, u64, Option<Leader>, Vec<u32>)>,
    }

    fn run(memory: &Sparse, rows: usize) -> Output {
        let mut traits = vec![TraitRow::default(); rows];
        let mut values = vec![Trait::default(); 16];
        let mut operatives = vec![OperativeRow::default(); rows];
        let mut definitions = vec![0; 16];
        let walk = walk(
            memory,
            0,
            GS,
            Buffers {
                traits: &mut traits,
                trait_values: &mut values,
                operatives: &mut operatives,
                definitions: &mut definitions,
            },
        );
        Output {
            walk,
            traits: traits[..walk.traits.len()]
                .iter()
                .map(|row| {
                    (
                        row.character,
                        row.role,
                        values[row.start..row.start + row.len].to_vec(),
                    )
                })
                .collect(),
            operatives: operatives[..walk.operatives.len()]
                .iter()
                .map(|row| {
                    (
                        row.character,
                        row.reference,
                        row.leader,
                        definitions[row.start..row.start + row.len].to_vec(),
                    )
                })
                .collect(),
        }
    }

    fn leader(token: u32, ordinal: u32) -> Trait {
        Trait {
            token,
            ordinal: Some(ordinal),
        }
    }

    #[test]
    fn every_role_array_and_operative_link_is_read_in_roster_order() {
        let output = run(&world(), 8);
        assert_eq!(output.walk.traits, Outcome::Rows { len: 3 });
        assert_eq!(
            output.traits,
            [
                (
                    11,
                    Role::CountryLeader(5),
                    vec![leader(101, 1), leader(102, 2)]
                ),
                (11, Role::Advisor(0), vec![leader(102, 2)]),
                (
                    11,
                    Role::Scientist,
                    vec![Trait {
                        token: 201,
                        ordinal: None
                    }]
                ),
            ]
        );
        assert_eq!(output.walk.operatives, Outcome::Rows { len: 2 });
        assert_eq!(
            output.operatives,
            [
                (
                    12,
                    RESOLVED,
                    Some(Leader {
                        category: 3,
                        owner: 9,
                        skill_level: Some(4)
                    }),
                    vec![3]
                ),
                (13, DANGLING, None, vec![]),
            ]
        );
        assert_eq!(
            (
                output.walk.required_trait_values,
                output.walk.required_definitions
            ),
            (4, 1)
        );
    }

    #[test]
    fn rows_that_do_not_fit_are_counted_with_their_values() {
        let output = run(&world(), 1);
        assert_eq!(
            output.walk.traits,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        assert_eq!(
            output.walk.operatives,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(output.walk.required_trait_values, 4);
        let output = run(&world(), 0);
        assert_eq!(
            output.walk.traits,
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
    }

    #[test]
    fn a_role_or_leader_failure_fails_only_its_table() {
        let mut memory = world();
        memory.u64(0x5200, 1);
        let output = run(&memory, 8);
        assert!(matches!(output.walk.traits, Outcome::Unavailable { .. }));
        assert_eq!(output.walk.operatives, Outcome::Rows { len: 2 });
        let mut memory = world();
        memory.u64(0x7200, 1);
        let output = run(&memory, 8);
        assert_eq!(output.walk.traits, Outcome::Rows { len: 3 });
        assert!(matches!(
            output.walk.operatives,
            Outcome::Unavailable { .. }
        ));
        // A null skill object has no level.
        let mut memory = world();
        memory.u64(OPERATIVE + cunit_leader::SKILL, 0);
        assert_eq!(run(&memory, 8).operatives[0].2.unwrap().skill_level, None);
    }

    /// An empty map is accepted only with a valid header and sentinel; a bad
    /// one fails the trait table alone.
    #[test]
    fn empty_maps_are_checked_and_a_bad_one_fails_only_the_trait_table() {
        let b_leaders = 0x4800 + 2 * 0x40;
        for (what, change) in [
            (
                "null head",
                Box::new(|m: &mut Sparse| m.u64(B + ccharacter::LEADER_MAP_HEAD, 0))
                    as Box<dyn Fn(&mut Sparse)>,
            ),
            (
                "unreadable sentinel",
                Box::new(|m: &mut Sparse| m.u64(B + ccharacter::LEADER_MAP_HEAD, 0x9_0000)),
            ),
            (
                "nil byte clear",
                Box::new(move |m: &mut Sparse| m.put(b_leaders + node::NIL, &[0])),
            ),
            (
                "empty with foreign links",
                Box::new(move |m: &mut Sparse| m.u64(b_leaders + node::PARENT, 0x4000)),
            ),
            (
                "count with self links",
                Box::new(|m: &mut Sparse| m.u64(B + ccharacter::ADVISOR_MAP_SIZE, 1)),
            ),
        ] {
            let mut memory = world();
            change(&mut memory);
            let output = run(&memory, 8);
            assert!(
                matches!(output.walk.traits, Outcome::Unavailable { .. }),
                "{what}: {:?}",
                output.walk.traits
            );
            assert_eq!(output.walk.operatives, Outcome::Rows { len: 2 }, "{what}");
        }
    }

    /// A later failure of the walk both tables share does not replace a
    /// table's own earlier one.
    #[test]
    fn a_table_keeps_its_own_first_failure_before_a_shared_one() {
        let mut memory = world();
        memory.u64(0x5200, 1);
        memory.u32(C + ccharacter::TYPE, 74);
        let output = run(&memory, 8);
        assert_eq!(
            output.walk.traits,
            Outcome::Unavailable {
                reason: "advisor role vtable or character parent mismatch"
            }
        );
        assert_eq!(
            output.walk.operatives,
            Outcome::Unavailable {
                reason: "character vtable or reference type mismatch"
            }
        );
    }

    #[test]
    fn an_unauthenticated_character_fails_both_tables() {
        let mut memory = world();
        memory.u32(C + ccharacter::TYPE, 74);
        let output = run(&memory, 8);
        assert!(matches!(output.walk.traits, Outcome::Unavailable { .. }));
        assert!(matches!(
            output.walk.operatives,
            Outcome::Unavailable { .. }
        ));
    }
}
