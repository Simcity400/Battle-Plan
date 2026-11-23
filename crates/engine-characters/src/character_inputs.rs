
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_tree_node::Cursor as RoleTree,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::{cadvisor, ccharacter::{self, at, manager::Roster}, cunit_leader};
use engine_native_politics::{ccountry_leader, character_leader_node};
use engine_native_parts::{
    character_advisor_node,
    creference_registry,
    cunit_leader_skill,
    cunit_leader_trait,
    null_unit_leader_skill,
};
use crate::cscientist;

fn site(base: usize, key: engine_sites::Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("character address overflow")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    CountryLeader(i32),
    Advisor(u32),
    #[default]
    Scientist,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Trait {
    pub token: u32,
    pub ordinal: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TraitRow {
    pub character: u32,
    pub role: Role,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Leader {
    pub category: i32,
    pub owner: u32,
    pub skill_level: Option<i32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OperativeRow {
    pub character: u32,
    pub reference: u64,
    pub leader: Option<Leader>,
    pub start: usize,
    pub len: usize,
    pub name: Option<engine_native::cname_group_member::Span>,
    pub codename: Option<engine_native::cname_group_member::Inputs>,
    pub name_reason: Option<&'static str>,
    pub codename_reason: Option<&'static str>,
    pub codename_heap: [bool; 2],
}

pub struct Buffers<'a> {
    pub traits: &'a mut Vec<TraitRow>,
    pub trait_values: &'a mut Vec<Trait>,
    pub operatives: &'a mut Vec<OperativeRow>,
    pub definitions: &'a mut Vec<u32>,
    pub bytes: &'a mut Vec<u8>,
}
struct Storage<'a> {
    traits: engine_native_memory::walk::Buffer<'a, TraitRow>,
    trait_values: engine_native_memory::walk::Buffer<'a, Trait>,
    operatives: engine_native_memory::walk::Buffer<'a, OperativeRow>,
    definitions: engine_native_memory::walk::Buffer<'a, u32>,
    bytes: engine_native_memory::walk::Buffer<'a, u8>,
}
struct Names<'a> {
    bytes: engine_native_memory::walk::Buffer<'a, u8>,
    len: usize,
    required: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub traits: Outcome,
    pub operatives: Outcome,
    pub required_trait_values: usize,
    pub required_definitions: usize,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

const PAGE: usize = 64;
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
const TRAITS_PER_READ: usize = 32;
const MAPS: [usize; 2] = [ccharacter::LEADER_MAP_HEAD, ccharacter::ADVISOR_MAP_HEAD];
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

struct Table<'a, R, V> {
    rows: engine_native_memory::walk::Buffer<'a, R>,
    values: engine_native_memory::walk::Buffer<'a, V>,
    len: usize,
    values_len: usize,
    required: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl<'a, R: Default, V: Default> Table<'a, R, V> {
    fn new(rows: engine_native_memory::walk::Buffer<'a, R>, values: engine_native_memory::walk::Buffer<'a, V>) -> Self {
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

    fn reserve(&mut self, count: usize) -> Result<Option<usize>, &'static str> {
        self.required = self
            .required
            .checked_add(count)
            .ok_or("character input value count overflow")?;
        self.rows.ensure(
            self.len
                .checked_add(1)
                .ok_or("character input row count overflow")?,
        )?;
        self.values.ensure(
            self.values_len
                .checked_add(count)
                .ok_or("character input value count overflow")?,
        )?;
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

    fn run(&mut self, step: impl FnOnce(&mut Self) -> Result<(), &'static str>) {
        if self.failure.is_none()
            && let Err(reason) = step(self)
        {
            self.failure = Some(reason);
        }
    }

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

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: Buffers<'_>,
) -> Walk {
    walk_storage(
        memory,
        base,
        game_state,
        Storage {
            traits: engine_native_memory::walk::Buffer::Growing(buffers.traits),
            trait_values: engine_native_memory::walk::Buffer::Growing(buffers.trait_values),
            operatives: engine_native_memory::walk::Buffer::Growing(buffers.operatives),
            definitions: engine_native_memory::walk::Buffer::Growing(buffers.definitions),
            bytes: engine_native_memory::walk::Buffer::Growing(buffers.bytes),
        },
    )
}
pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_storage(
        memory,
        base,
        game_state,
        Storage {
            traits: engine_native_memory::walk::Buffer::Count,
            trait_values: engine_native_memory::walk::Buffer::Count,
            operatives: engine_native_memory::walk::Buffer::Count,
            definitions: engine_native_memory::walk::Buffer::Count,
            bytes: engine_native_memory::walk::Buffer::Count,
        },
    )
}

fn walk_storage(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: Storage<'_>,
) -> Walk {
    let mut traits = Table::new(buffers.traits, buffers.trait_values);
    let mut operatives = Table::new(buffers.operatives, buffers.definitions);
    let mut resolver = creference_registry::HeaderResolver::default();
    let mut names = Names {
        bytes: buffers.bytes,
        len: 0,
        required: 0,
    };
    let mut page_pointers = [0_usize; PAGE];
    let mut page_records = [0_u8; PAGE * ROW];
    let mut page_heads = [0_usize; PAGE * MAPS.len()];
    let mut page_sentinels = [0_u8; PAGE * MAPS.len() * SENTINEL];
    let result = (|| {
        let character_vtable = site(base, ccharacter::VTABLE_RVA)? as u64;
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
                            let object = resolver.resolve(memory, base, reference)?;
                            operative(
                                memory, base, object, character, reference, table, &mut names,
                            )
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
        bytes_len: names.len,
        required_bytes: names.required,
    }
}

pub const CENSUS_PAGE: usize = PAGE;
pub const MAP_SENTINEL: usize = SENTINEL;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Occupant {
    pub pointer: usize,
    pub character: u32,
    pub leaders: [u8; 16],
    pub advisors: [u8; 16],
    pub scientist: u64,
    pub operative: u64,
}

pub struct CensusWalk<'a> {
    traits: Table<'a, TraitRow, Trait>,
    operatives: Table<'a, OperativeRow, u32>,
    resolver: creference_registry::HeaderResolver,
    names: Names<'a>,
}

impl<'a> CensusWalk<'a> {
    pub fn new(buffers: Buffers<'a>) -> Self {
        Self::storage(Storage {
            traits: engine_native_memory::walk::Buffer::Growing(buffers.traits),
            trait_values: engine_native_memory::walk::Buffer::Growing(buffers.trait_values),
            operatives: engine_native_memory::walk::Buffer::Growing(buffers.operatives),
            definitions: engine_native_memory::walk::Buffer::Growing(buffers.definitions),
            bytes: engine_native_memory::walk::Buffer::Growing(buffers.bytes),
        })
    }

    fn storage(buffers: Storage<'a>) -> Self {
        Self {
            traits: Table::new(buffers.traits, buffers.trait_values),
            operatives: Table::new(buffers.operatives, buffers.definitions),
            resolver: creference_registry::HeaderResolver::default(),
            names: Names {
                bytes: buffers.bytes,
                len: 0,
                required: 0,
            },
        }
    }

    pub fn sentinels(&mut self, memory: &impl Memory, headers: &[[u8; 16]], output: &mut [u8]) {
        self.traits.run(|_| {
            if headers.len() > PAGE * MAPS.len() || output.len() != headers.len() * SENTINEL {
                return Err("character role map sentinel batch exceeds its page");
            }
            if headers.is_empty() {
                return Ok(());
            }
            let mut heads = [0_usize; PAGE * MAPS.len()];
            for (head, header) in heads.iter_mut().zip(headers) {
                *head = RoleTree::header(*header)?.0;
            }
            let heads = &heads[..headers.len()];
            if memory.copy_records(RecordSources::Pointers(heads), &SENTINEL_LAYOUT, output)
                != heads.len()
            {
                return Err("character role map sentinel unreadable");
            }
            Ok(())
        });
    }

    pub fn roles(
        &mut self,
        memory: &impl Memory,
        base: usize,
        occupant: &Occupant,
        sentinels: [[u8; SENTINEL]; 2],
    ) {
        let maps = [
            (occupant.leaders, sentinels[0]),
            (occupant.advisors, sentinels[1]),
        ];
        self.traits.run(|table| {
            map_role_traits(
                memory,
                base,
                occupant.pointer,
                occupant.character,
                maps,
                table,
            )
        });
    }

    pub fn carry(&mut self, rows: &[TraitRow], values: &[Trait]) {
        self.traits.run(|table| {
            for row in rows {
                let source = row
                    .start
                    .checked_add(row.len)
                    .and_then(|end| values.get(row.start..end))
                    .ok_or("carried character role traits outside their arena")?;
                if row.len == 0 {
                    return Err("carried character role row has no traits");
                }
                let Some(start) = table.reserve(row.len)? else {
                    continue;
                };
                table.values[start..start + row.len].copy_from_slice(source);
                table.push(TraitRow { start, ..*row });
            }
            Ok(())
        });
    }

    pub fn scientist(&mut self, memory: &impl Memory, base: usize, occupant: &Occupant) {
        self.traits.run(|table| {
            scientist_traits(memory, base, occupant.scientist, occupant.character, table)
        });
    }

    pub fn operative(&mut self, memory: &impl Memory, base: usize, occupant: &Occupant) {
        if occupant.operative == 0 {
            return;
        }
        if let Some(object) = self.resolve_operative(memory, base, occupant.operative) {
            self.operative_resolved(memory, base, occupant, object);
        }
    }

    pub fn resolve_operative(
        &mut self,
        memory: &impl Memory,
        base: usize,
        reference: u64,
    ) -> Option<Option<usize>> {
        let Self {
            operatives,
            resolver,
            ..
        } = self;
        let mut resolved = None;
        operatives.run(|_| {
            resolved = Some(resolver.resolve(memory, base, reference)?);
            Ok(())
        });
        resolved
    }

    pub fn operative_resolved(
        &mut self,
        memory: &impl Memory,
        base: usize,
        occupant: &Occupant,
        object: Option<usize>,
    ) {
        let Self {
            operatives, names, ..
        } = self;
        operatives.run(|table| {
            operative(
                memory,
                base,
                object,
                occupant.character,
                occupant.operative,
                table,
                names,
            )
        });
    }

    pub fn operatives_stored(&self) -> usize {
        self.operatives.len
    }

    pub fn last_operative(&self, text: &mut Vec<u8>) -> Option<(OperativeRow, &[u32])> {
        let len = self.operatives.len;
        let row = *self.operatives.rows.get(len.checked_sub(1)?)?;
        let definitions = self.operatives.values.get(row.start..row.start + row.len)?;
        let bytes = self.names.bytes.get(..self.names.len)?;
        let spans = row.name.into_iter().chain(
            row.codename
                .into_iter()
                .flat_map(|inputs| [inputs.group, inputs.custom]),
        );
        text.clear();
        crate::allocation::reserve(text, spans.map(|(_, len)| len).sum()).ok()?;
        let mut take = |(at, len): engine_native::cname_group_member::Span| {
            let moved = text.len();
            text.extend_from_slice(bytes.get(at..at + len)?);
            Some((moved, len))
        };
        let name = match row.name {
            Some(span) => Some(take(span)?),
            None => None,
        };
        let codename = match row.codename {
            Some(mut inputs) => {
                inputs.group = take(inputs.group)?;
                if engine_native::cname_group_member::custom_copied(&inputs) {
                    inputs.custom = take(inputs.custom)?;
                }
                Some(inputs)
            }
            None => None,
        };
        Some((
            OperativeRow {
                start: 0,
                name,
                codename,
                ..row
            },
            definitions,
        ))
    }

    pub fn carry_operative(&mut self, row: &OperativeRow, definitions: &[u32], text: &[u8]) {
        let Self {
            operatives, names, ..
        } = self;
        operatives.run(|table| {
            let held = |(at, len): engine_native::cname_group_member::Span| {
                text.get(at..at + len)
                    .ok_or("carried operative text out of range")
            };
            let mut pending = Names {
                bytes: names.bytes.suffix(0),
                len: names.len,
                required: names.required,
            };
            let mut carried = *row;
            if let Some(span) = row.name {
                (carried.name, carried.name_reason) = match place(&mut pending, held(span)?) {
                    Ok(Some(at)) => (Some((at, span.1)), None),
                    Ok(None) => (None, Some("operative name byte capacity exceeded")),
                    Err(reason) => (None, Some(reason)),
                };
            }
            if let Some(inputs) = row.codename {
                use engine_native::cname_group_member::{self as naming, Naming};
                let codename_start = pending.len;
                let group = held(inputs.group)?;
                let custom = if naming::custom_copied(&inputs) {
                    Some(held(inputs.custom)?)
                } else {
                    None
                };
                let mut arena = naming::Names {
                    bytes: pending.bytes.suffix(0),
                    len: pending.len,
                    required: pending.required,
                };
                let placed =
                    naming::carry_codename(&mut arena, inputs, group, custom, row.codename_heap);
                (pending.len, pending.required) = (arena.len, arena.required);
                (carried.codename, carried.codename_reason) = match placed {
                    Ok(Naming::Row(inputs)) => {
                        pending.len += inputs.custom.1;
                        (Some(inputs), None)
                    }
                    Ok(Naming::DoesNotFit) => {
                        (None, Some("operative codename byte capacity exceeded"))
                    }
                    Err(reason) => (None, Some(reason)),
                };
                if carried.codename.is_none() {
                    pending.len = codename_start;
                }
            }
            let (bytes_len, required_bytes) = (pending.len, pending.required);
            names.required = required_bytes;
            let Some(start) = table.reserve(definitions.len())? else {
                return Ok(());
            };
            table.values[start..start + definitions.len()].copy_from_slice(definitions);
            names.len = bytes_len;
            table.push(OperativeRow { start, ..carried });
            Ok(())
        });
    }

    pub fn stored(&self) -> (usize, usize) {
        (self.traits.len, self.traits.values_len)
    }

    pub fn stored_since(&self, from: (usize, usize)) -> (&[TraitRow], &[Trait]) {
        (
            &self.traits.rows[from.0.min(self.traits.len)..self.traits.len],
            &self.traits.values[..self.traits.values_len],
        )
    }

    pub fn traits_complete(&self) -> bool {
        self.traits.failure.is_none() && self.traits.unread_rows == 0
    }

    pub fn finish(self) -> Walk {
        Walk {
            traits: self.traits.outcome(None),
            operatives: self.operatives.outcome(None),
            required_trait_values: self.traits.required,
            required_definitions: self.operatives.required,
            bytes_len: self.names.len,
            required_bytes: self.names.required,
        }
    }
}

fn map_header(record: &[u8], map: usize) -> [u8; 16] {
    let at = IDENTITY + map - ccharacter::LEADER_MAP_HEAD;
    record[at..at + 16].try_into().unwrap()
}

fn role_traits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    maps: [([u8; 16], [u8; SENTINEL]); 2],
    field: &impl Fn(usize) -> u64,
    table: &mut Table<'_, TraitRow, Trait>,
) -> Result<(), &'static str> {
    map_role_traits(memory, base, pointer, character, maps, table)?;
    scientist_traits(memory, base, field(ccharacter::SCIENTIST), character, table)
}

fn map_role_traits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    [leaders, advisors]: [([u8; 16], [u8; SENTINEL]); 2],
    table: &mut Table<'_, TraitRow, Trait>,
) -> Result<(), &'static str> {
    let country_leader_trait = site(base, ccountry_leader::TRAIT_VTABLE_RVA)? as u64;
    let mut leaders = RoleTree::from_parts(leaders.0, leaders.1)?;
    let mut advisors = RoleTree::from_parts(advisors.0, advisors.1)?;
    if leaders.count() != 0 {
        let vtable = site(base, engine_native_parts::ccountry_leader_layout::VTABLE_RVA)? as u64;
        while let Some(node) = leaders.next(memory)? {
            let value = read::<[u8; 16]>(memory, at(node, character_leader_node::IDEOLOGY)?)
                .ok_or("country leader ideology unreadable")?;
            let ideology = i32::from_le_bytes(value[..4].try_into().unwrap());
            let role = non_null(u64_at(&value, 8), "country leader role unreadable or null")?;
            let header =
                read::<[u8; 16]>(memory, role).ok_or("country leader role header unreadable")?;
            if u64_at(&header, 0) != vtable
                || u64_at(&header, engine_native_parts::ccountry_leader_layout::CHARACTER) != pointer as u64
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
        let vtable = site(base, cadvisor::VTABLE_RVA)? as u64;
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
    Ok(())
}

fn scientist_traits(
    memory: &impl Memory,
    base: usize,
    scientist: u64,
    character: u32,
    table: &mut Table<'_, TraitRow, Trait>,
) -> Result<(), &'static str> {
    if scientist != 0 {
        let scientist = non_null(scientist, "scientist pointer overflow")?;
        if read::<u64>(memory, scientist) != Some(site(base, cscientist::VTABLE_RVA)? as u64) {
            return Err("scientist vtable mismatch");
        }
        let array = Array::read(memory, at(scientist, cscientist::TRAIT_DATA)?, 8)
            .ok_or("scientist trait array unreadable or invalid")?;
        push_traits(
            memory,
            array,
            site(base, cscientist::TRAIT_VTABLE_RVA)? as u64,
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

fn place(names: &mut Names<'_>, bytes: &[u8]) -> Result<Option<usize>, &'static str> {
    const OVERFLOW: &str = "unit leader name byte count overflow";
    names.required = names.required.checked_add(bytes.len()).ok_or(OVERFLOW)?;
    let start = names.len;
    let end = start.checked_add(bytes.len()).ok_or(OVERFLOW)?;
    if !names.bytes.ensure(end)? {
        return Ok(None);
    }
    names.bytes[start..end].copy_from_slice(bytes);
    names.len = end;
    Ok(Some(start))
}

fn operative(
    memory: &impl Memory,
    base: usize,
    object: Option<usize>,
    character: u32,
    reference: u64,
    table: &mut Table<'_, OperativeRow, u32>,
    names: &mut Names<'_>,
) -> Result<(), &'static str> {
    let Some(object) = object else {
        if let Some(start) = table.reserve(0)? {
            table.push(OperativeRow {
                character,
                reference,
                leader: None,
                start,
                len: 0,
                name: None,
                codename: None,
                name_reason: None,
                codename_reason: None,
                codename_heap: [false; 2],
            });
        }
        return Ok(());
    };
    let (kind, _, _) = cunit_leader::identity(memory, base, object)?;
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
        let vtable = u64::from_le_bytes(vtable);
        let skills = [
            cunit_leader_skill::VTABLE_RVA,
            null_unit_leader_skill::VTABLE_RVA,
        ];
        if usize::try_from(vtable)
            .ok()
            .and_then(|vtable| engine_sites::which(base, vtable, &skills))
            .is_none()
        {
            return Err("operative skill vtable mismatch");
        }
        Some(i32::from_le_bytes(level))
    };
    let array = Array::from_header(traits, 8).ok_or("operative trait array invalid")?;
    let len = array.count as usize;
    let mut pending = Names {
        bytes: names.bytes.suffix(0),
        len: names.len,
        required: names.required,
    };
    let (name, name_reason) = match cunit_leader::read_name_buffer(
        memory,
        object,
        pending.bytes.suffix(0),
        &mut pending.len,
        &mut pending.required,
    ) {
        Ok(Some(span)) => (Some(span), None),
        Ok(None) => (None, Some("operative name byte capacity exceeded")),
        Err(reason) => (None, Some(reason)),
    };
    let category = i32::from_le_bytes(category);
    let codename_start = pending.len;
    let mut codename_heap = [false; 2];
    let (codename, codename_reason) = if cunit_leader::has_codename(kind) {
        match cunit_leader::read_codename_buffer(
            memory,
            base,
            object,
            pending.bytes.suffix(0),
            &mut pending.len,
            &mut pending.required,
            &mut codename_heap,
        ) {
            Ok(engine_native::cname_group_member::Naming::Row(row)) => (Some(row), None),
            Ok(engine_native::cname_group_member::Naming::DoesNotFit) => {
                pending.len = codename_start;
                (None, Some("operative codename byte capacity exceeded"))
            }
            Err(reason) => {
                pending.len = codename_start;
                (None, Some(reason))
            }
        }
    } else {
        (None, None)
    };
    let (bytes_len, required_bytes) = (pending.len, pending.required);
    names.required = required_bytes;
    let Some(start) = table.reserve(len)? else {
        return Ok(());
    };
    let trait_vtable = site(base, cunit_leader_trait::VTABLE_RVA)? as u64;
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
    names.len = bytes_len;
    table.push(OperativeRow {
        character,
        reference,
        leader: Some(Leader {
            category,
            owner: u32::from_le_bytes(owner),
            skill_level,
        }),
        start,
        len,
        name,
        codename,
        name_reason,
        codename_reason,
        codename_heap,
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
    engine_native_memory::cpdx_array::pointers(
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
    use engine_native::{ccharacter::manager as ccharacter_manager, cgame_state};
        use engine_native_memory::msvc_tree_node as node;
    use engine_native_parts::reference_bucket as bucket;
    use std::collections::BTreeMap;

    const GS: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const A: usize = 0x3000;
    const B: usize = 0x3400;
    const C: usize = 0x3800;
    const TABLE: usize = 0x8000;
    const ENTRIES: usize = 0x8100;
    const OPERATIVE: usize = 0xa000;
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
            self.u64(at, ccharacter::VTABLE_RVA.0 as u64);
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

    fn world() -> Sparse {
        let mut m = Sparse::default();
        m.u64(GS + cgame_state::CHARACTER_MANAGER, MANAGER as u64);
        m.u64(MANAGER, ccharacter_manager::VTABLE_RVA.0 as u64);
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
        let leader_trait = ccountry_leader::TRAIT_VTABLE_RVA.0;
        m.object(0x6000, leader_trait, 101);
        m.object(0x6100, leader_trait, 102);
        m.u32(0x6000 + ccountry_leader::TRAIT_ORDINAL, 1);
        m.u32(0x6100 + ccountry_leader::TRAIT_ORDINAL, 2);
        m.object(0x6200, cscientist::TRAIT_VTABLE_RVA.0, 201);
        m.one_node(A, ccharacter::LEADER_MAP_HEAD, 0x4000, 0x4040);
        m.u64(0x4040 + character_leader_node::IDEOLOGY, 5);
        m.u64(0x4040 + character_leader_node::LEADER, 0x5000);
        m.object(0x5000, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0, A as u64);
        m.array(0x5000 + ccountry_leader::TRAITS, 0x5100, &[0x6000, 0x6100]);
        m.one_node(A, ccharacter::ADVISOR_MAP_HEAD, 0x4100, 0x4140);
        m.u64(0x4140 + character_advisor_node::ADVISOR, 0x5200);
        m.object(0x5200, cadvisor::VTABLE_RVA.0, 0);
        m.u64(0x5200 + cadvisor::CHARACTER, A as u64);
        m.array(0x5200 + cadvisor::TRAITS, 0x5300, &[0x6100]);
        m.u64(A + ccharacter::SCIENTIST, 0x5400);
        m.u64(0x5400, cscientist::VTABLE_RVA.0 as u64);
        m.array(0x5400 + cscientist::TRAIT_DATA, 0x5500, &[0x6200]);
        m.u64(B + ccharacter::OPERATIVE, RESOLVED);
        m.u64(C + ccharacter::OPERATIVE, DANGLING);
        m.u64(creference_registry::HIGH_ROOT_RVA.0, TABLE as u64);
        m.u64(TABLE + creference_registry::ENTRIES, ENTRIES as u64);
        m.u32(TABLE + creference_registry::COUNT, 1);
        m.u32(TABLE + creference_registry::MASK, 7);
        m.put(TABLE + creference_registry::OVERFLOW_DISTANCE, &[5]);
        m.put(ENTRIES, &[0; 14 * bucket::STRIDE]);
        m.put(ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        m.bucket(RESOLVED, OPERATIVE);
        m.u64(OPERATIVE, engine_native_parts::coperative_leader_layout::VTABLE_RVA.0 as u64);
        m.u32(OPERATIVE + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
        m.u32(OPERATIVE + cunit_leader::ID, 77);
        m.u32(OPERATIVE + cunit_leader::OWNER, 9);
        m.array(OPERATIVE + cunit_leader::TRAIT_DATA, 0x7100, &[0x7200]);
        m.u64(OPERATIVE + cunit_leader::SKILL, 0x7300);
        m.u32(OPERATIVE + cunit_leader::CATEGORY, 3);
        m.put(OPERATIVE + cunit_leader::NAME, &[0; 32]);
        m.put(OPERATIVE + cunit_leader::NAME, b"Alias");
        m.u64(OPERATIVE + cunit_leader::NAME + 16, 5);
        m.u64(OPERATIVE + cunit_leader::NAME + 24, 15);
        let member = OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME;
        m.put(member, &[0; engine_native::cname_group_member::INPUT_SPAN]);
        m.u64(member, engine_native::cname_group_member::VTABLE_RVA.0 as u64);
        m.u32(member + engine_native::cname_group_member::TYPE, 2);
        m.put(member + 0x10, b"GROUP");
        m.u64(member + 0x20, 5);
        m.u64(member + 0x28, 15);
        m.u64(member + 0xa0, 15);
        m.put(member + 0xa8, &[1]);
        m.u64(0x7200, cunit_leader_trait::VTABLE_RVA.0 as u64);
        m.u32(0x7200 + cunit_leader_trait::ORDINAL, 3);
        m.u64(0x7300, cunit_leader_skill::VTABLE_RVA.0 as u64);
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
        let mut bytes = vec![0; 64];
        let walk = walk(
            memory,
            0,
            GS,
            Buffers {
                traits: &mut traits,
                trait_values: &mut values,
                operatives: &mut operatives,
                definitions: &mut definitions,
                bytes: &mut bytes,
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
                        skill_level: Some(4),
                    }),
                    vec![3],
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

    fn census(
        memory: &Sparse,
        carry: &[u32],
        held: &(Vec<TraitRow>, Vec<Trait>),
    ) -> (Output, Vec<TraitRow>, Vec<Trait>) {
        let occupants: Vec<Occupant> = [(A, 11), (B, 12), (C, 13)]
            .into_iter()
            .map(|(pointer, character)| {
                let field = |offset: usize| read::<[u8; 16]>(memory, pointer + offset).unwrap();
                Occupant {
                    pointer,
                    character,
                    leaders: field(ccharacter::LEADER_MAP_HEAD),
                    advisors: field(ccharacter::ADVISOR_MAP_HEAD),
                    scientist: read(memory, pointer + ccharacter::SCIENTIST).unwrap_or(0),
                    operative: read(memory, pointer + ccharacter::OPERATIVE).unwrap_or(0),
                }
            })
            .collect();
        let (mut traits, mut values) = (Vec::new(), Vec::new());
        let (mut operatives, mut definitions, mut bytes) = (Vec::new(), Vec::new(), Vec::new());
        let mut walk = CensusWalk::new(Buffers {
            traits: &mut traits,
            trait_values: &mut values,
            operatives: &mut operatives,
            definitions: &mut definitions,
            bytes: &mut bytes,
        });
        let fresh: Vec<_> = occupants
            .iter()
            .filter(|occupant| !carry.contains(&occupant.character))
            .collect();
        let headers: Vec<[u8; 16]> = fresh
            .iter()
            .flat_map(|occupant| [occupant.leaders, occupant.advisors])
            .collect();
        let mut sentinels = vec![0; headers.len() * MAP_SENTINEL];
        walk.sentinels(memory, &headers, &mut sentinels);
        let mut next = 0;
        for occupant in &occupants {
            if carry.contains(&occupant.character) {
                let rows: Vec<_> = held
                    .0
                    .iter()
                    .copied()
                    .filter(|row| row.character == occupant.character)
                    .collect();
                walk.carry(&rows, &held.1);
            } else {
                let at = next * 2 * MAP_SENTINEL;
                let gathered = |map: usize| {
                    sentinels[at + map * MAP_SENTINEL..][..MAP_SENTINEL]
                        .try_into()
                        .unwrap()
                };
                walk.roles(memory, 0, occupant, [gathered(0), gathered(1)]);
                next += 1;
            }
            walk.scientist(memory, 0, occupant);
            walk.operative(memory, 0, occupant);
        }
        let result = walk.finish();
        let output = Output {
            walk: result,
            traits: traits[..result.traits.len()]
                .iter()
                .map(|row| {
                    let values = values[row.start..row.start + row.len].to_vec();
                    (row.character, row.role, values)
                })
                .collect(),
            operatives: operatives[..result.operatives.len()]
                .iter()
                .map(|row| {
                    let traits = definitions[row.start..row.start + row.len].to_vec();
                    (row.character, row.reference, row.leader, traits)
                })
                .collect(),
        };
        (output, traits, values)
    }

    #[test]
    fn the_census_walk_reads_and_carries_exactly_what_the_roster_walk_reads() {
        let memory = world();
        let roster = run(&memory, 8);
        let (fresh, rows, values) = census(&memory, &[], &(Vec::new(), Vec::new()));
        assert_eq!(fresh.walk, roster.walk);
        assert_eq!(fresh.traits, roster.traits);
        assert_eq!(fresh.operatives, roster.operatives);
        let held = (rows[..2].to_vec(), values);
        let (carried, ..) = census(&memory, &[11], &held);
        assert_eq!(carried.walk, roster.walk);
        assert_eq!(carried.traits, roster.traits);
        let mut gone = world();
        gone.0.retain(|&address, _| {
            !(0x4000..0x4200).contains(&address) && !(0x5000..0x5400).contains(&address)
        });
        let (carried, ..) = census(&gone, &[11], &held);
        assert_eq!(carried.traits, roster.traits);
        let (fresh, ..) = census(&gone, &[], &(Vec::new(), Vec::new()));
        assert!(matches!(fresh.walk.traits, Outcome::Unavailable { .. }));
    }

    #[test]
    fn resolved_name_and_codename_are_owned_and_dangling_references_stay_absent() {
        let mut memory = world();
        let mut rows = vec![OperativeRow::default(); 2];
        let mut definitions = vec![0; 1];
        let mut bytes = vec![0; 10];
        fn capture(
            memory: &Sparse,
            rows: &mut Vec<OperativeRow>,
            definitions: &mut Vec<u32>,
            bytes: &mut Vec<u8>,
        ) -> Walk {
            walk(
                memory,
                0,
                GS,
                Buffers {
                    traits: &mut Vec::new(),
                    trait_values: &mut Vec::new(),
                    operatives: rows,
                    definitions,
                    bytes,
                },
            )
        }
        let result = capture(&memory, &mut rows, &mut definitions, &mut bytes);
        assert_eq!(result.operatives, Outcome::Rows { len: 2 });
        assert_eq!((result.required_bytes, result.bytes_len), (10, 10));
        assert_eq!(rows[0].name, Some((0, 5)));
        assert_eq!(&bytes[..5], b"Alias");
        let codename = rows[0].codename.unwrap();
        assert_eq!((codename.kind, codename.group), (2, (5, 5)));
        assert_eq!(&bytes[5..], b"GROUP");
        assert_eq!((rows[1].name, rows[1].codename), (None, None));
        let before = rows.clone();
        memory.put(OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME + 0x60, &[b'Z'; 32]);
        capture(&memory, &mut rows, &mut definitions, &mut bytes);
        assert_eq!(rows, before);
        memory.u32(OPERATIVE + cunit_leader::CATEGORY, 0);
        capture(&memory, &mut rows, &mut definitions, &mut bytes);
        assert!(rows[0].codename.is_some());
        memory.u32(OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME + 8, 1);
        let wrong_kind = capture(&memory, &mut rows, &mut definitions, &mut bytes);
        assert_eq!(wrong_kind.operatives, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].codename, None);
        assert_eq!(
            rows[0].codename_reason,
            Some("operative codename member is not type two")
        );
        memory.u32(OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME + 8, 2);
        memory.u64(OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME, 0);
        assert_eq!(
            capture(&memory, &mut rows, &mut definitions, &mut bytes).operatives,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows[0].codename_reason,
            Some("division name vtable mismatch")
        );
    }

    #[test]
    fn a_carried_operative_stores_exactly_what_a_fresh_read_stores() {
        use engine_native::cname_group_member::{FLAG_B, OVERRIDE_NAME};
        let mut memory = world();
        let occupant = Occupant {
            pointer: B,
            character: 12,
            leaders: [0; 16],
            advisors: [0; 16],
            scientist: 0,
            operative: RESOLVED,
        };
        let member = OPERATIVE + engine_native_parts::coperative_leader_layout::CODENAME;
        let (group_header, custom_header) = (member + 0x10, member + OVERRIDE_NAME);
        let string = |memory: &mut Sparse, header: usize, text: &[u8], heap: Option<usize>| {
            match heap {
                Some(at) => {
                    memory.put(at, text);
                    memory.put(at + text.len(), &[0]);
                    memory.u64(header, at as u64);
                    memory.u64(header + 24, (text.len() as u64).max(31));
                }
                None => {
                    memory.put(header, &[0; 16]);
                    memory.put(header, text);
                    memory.u64(header + 24, 15);
                }
            }
            memory.u64(header + 16, text.len() as u64);
        };
        let long: Vec<u8> = (0..300_u32)
            .map(|index| b'a' + (index % 26) as u8)
            .collect();
        let groups: [(&[u8], Option<usize>); 3] = [
            (b"GROUP", None),
            (b"GROUP-KEY-ON-THE-HEAP", Some(0x2_0000)),
            (&long, Some(0x2_0000)),
        ];
        let customs: [(u8, &[u8], Option<usize>); 6] = [
            (0, b"", None),
            (0, b"Code", None),
            (0, b"", Some(0x2_1000)),
            (0, b"Code", Some(0x2_1000)),
            (0, &long, Some(0x2_1000)),
            (1, b"", None),
        ];
        for (group, group_heap) in groups {
            string(&mut memory, group_header, group, group_heap);
            for (programmatic, name, custom_heap) in customs {
                memory.put(member + FLAG_B, &[programmatic]);
                string(&mut memory, custom_header, name, custom_heap);
                let (mut traits, mut values) = (Vec::new(), Vec::new());
                let (mut operatives, mut definitions, mut bytes) =
                    (Vec::new(), Vec::new(), Vec::new());
                let mut held = CensusWalk::new(Buffers {
                    traits: &mut traits,
                    trait_values: &mut values,
                    operatives: &mut operatives,
                    definitions: &mut definitions,
                    bytes: &mut bytes,
                });
                held.operative(&memory, 0, &occupant);
                let mut text = Vec::new();
                let (row, carried_definitions) = held.last_operative(&mut text).unwrap();
                let carried_definitions = carried_definitions.to_vec();
                assert_eq!(carried_definitions, [3]);
                assert_eq!(row.name_reason.or(row.codename_reason), None);
                assert_eq!(
                    row.codename_heap,
                    [group_heap.is_some(), custom_heap.is_some()]
                );
                assert_eq!(text, [b"Alias".as_slice(), group, name].concat());
                if programmatic == 1 {
                    assert_eq!(row.codename.unwrap().custom, (0, 0));
                }
                let heap = (group_heap.is_some(), custom_heap.is_some());
                let whole = 2 * text.len();
                {
                    for capacity in (0..=whole)
                        .filter(|&capacity| whole < 64 || capacity % 7 == 0 || capacity + 4 > whole)
                    {
                        for rows in 0..=2 {
                            for slots in 0..=2 {
                                let store = |carry: bool| {
                                    let mut operatives = vec![OperativeRow::default(); rows];
                                    let mut definitions = vec![0; slots];
                                    let mut bytes = vec![0; capacity];
                                    let (mut traits, mut values): (Vec<TraitRow>, Vec<Trait>) =
                                        (Vec::new(), Vec::new());
                                    let mut walk = CensusWalk::new(Buffers {
                                        traits: &mut traits,
                                        trait_values: &mut values,
                                        operatives: &mut operatives,
                                        definitions: &mut definitions,
                                        bytes: &mut bytes,
                                    });
                                    walk.operative(&memory, 0, &occupant);
                                    if carry {
                                        walk.carry_operative(&row, &carried_definitions, &text);
                                    } else {
                                        walk.operative(&memory, 0, &occupant);
                                    }
                                    (walk.finish(), operatives, definitions, bytes)
                                };
                                assert_eq!(
                                    store(true),
                                    store(false),
                                    "heap {heap:?}, programmatic {programmatic}, custom {} bytes, \
                                     {capacity} bytes, {rows} rows, \
                                     {slots} definitions",
                                    name.len()
                                );
                            }
                        }
                    }
                }
            }
        }
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
        let mut memory = world();
        memory.u64(OPERATIVE + cunit_leader::SKILL, 0);
        assert_eq!(run(&memory, 8).operatives[0].2.unwrap().skill_level, None);
    }

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
