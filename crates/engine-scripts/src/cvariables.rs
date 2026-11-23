
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native::{
    ccharacter::{self, manager as ccharacter_manager},
    ccountry,
    cgame_state,
    cstate,
};
use crate::variable_hash_node;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x0285_3318);
pub const VTABLE: usize = 0;
pub const SEED: usize = 8;
pub const STORAGE: usize = 0x18;
pub const ENTRY_COUNT: usize = 0x20;
pub const BUCKET_MASK: usize = 0x24;
pub const PROBE_TAIL: usize = 0x28;
pub const EMPTY_STORAGE_RVA: Rva = Rva(0x030f_d1a0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub name_start: usize,
    pub name_len: usize,
    pub value_raw: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Global,
    Country(u32),
    State(u32),
    Character(u32),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScopeRow {
    pub scope: Scope,
    pub entries_start: usize,
    pub entries_len: usize,
    pub seed: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeWalk {
    pub outcome: Outcome,
    pub entries_len: usize,
    pub required_entries: usize,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub const HEADER: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    pub scope: Scope,
    pub container: usize,
    pub header: [u8; HEADER],
    pub seed: Option<u32>,
}

pub trait VariableCarry {
    fn plan(&mut self, store: &Store) -> (usize, Option<(usize, usize)>);
    fn carry(&mut self, token: usize, entries: &mut [Row], names: &mut [u8]);
    fn read(&mut self, token: usize, outcome: Outcome, entries: &[Row], names: &[u8]);
}

pub struct Fresh;
impl VariableCarry for Fresh {
    fn plan(&mut self, _: &Store) -> (usize, Option<(usize, usize)>) {
        (0, None)
    }
    fn carry(&mut self, _: usize, _: &mut [Row], _: &mut [u8]) {}
    fn read(&mut self, _: usize, _: Outcome, _: &[Row], _: &[u8]) {}
}

pub fn count_scopes(memory: &impl Memory, base: usize, game_state: usize) -> ScopeWalk {
    walk_scopes_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_scopes(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<ScopeRow>,
    entries: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> ScopeWalk {
    walk_scopes_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(entries),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_scopes_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, ScopeRow>,
    entries: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> ScopeWalk {
    walk_scopes_carried(memory, base, game_state, output, entries, names, &mut Fresh)
}

pub fn walk_scopes_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, ScopeRow>,
    mut entries: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    carry: &mut impl VariableCarry,
) -> ScopeWalk {
    let result = (|| {
        let mut len = 0_usize;
        let mut missing = 0_u32;
        let mut entries_len = 0_usize;
        let mut required_entries = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut scope = |identity, container, head: Option<Head>| -> Result<(), &'static str> {
            if container == 0 {
                return Err("script variable scope root is null");
            }
            let head = head.unwrap_or_else(|| read_head(memory, container));
            output.ensure(
                len.checked_add(1)
                    .ok_or("variable scope row demand overflow")?,
            )?;
            let can_emit = len < output.len();
            let captured = if can_emit {
                store_rows(
                    memory,
                    base,
                    identity,
                    container,
                    head,
                    entries.suffix(entries_len),
                    names.suffix(bytes_len),
                    carry,
                )?
            } else {
                walk_head_buffer(
                    memory,
                    base,
                    head,
                    engine_native_memory::walk::Buffer::Count,
                    engine_native_memory::walk::Buffer::Count,
                )
            };
            let (count, omitted) = match captured.outcome {
                Outcome::Rows { len } => (len, 0),
                Outcome::Truncated { len, unread_rows } => (len, unread_rows),
                Outcome::Unavailable { reason } => return Err(reason),
            };
            required_entries = required_entries
                .checked_add(count)
                .and_then(|n| n.checked_add(omitted as usize))
                .ok_or("variable entry count overflow")?;
            required_bytes = required_bytes
                .checked_add(captured.required_bytes)
                .ok_or("variable name byte count overflow")?;
            if omitted != 0 || !can_emit {
                missing = missing
                    .checked_add(1)
                    .ok_or("variable omitted scope count overflow")?;
                return Ok(());
            }
            for entry in &mut entries[entries_len..entries_len + count] {
                entry.name_start = entry
                    .name_start
                    .checked_add(bytes_len)
                    .ok_or("variable name arena offset overflow")?;
            }
            output[len] = ScopeRow {
                scope: identity,
                entries_start: entries_len,
                entries_len: count,
                seed: head.seed(),
            };
            len += 1;
            entries_len += count;
            bytes_len += captured.bytes_len;
            Ok(())
        };
        scope(
            Scope::Global,
            scope_pointer(memory, game_state, cgame_state::GLOBAL_VARIABLES)?,
            None,
        )?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    Scope::Country(country.id),
                    scope_pointer(memory, country.pointer, ccountry::VARIABLES)?,
                    None,
                )?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        let states = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("variable state scope array unreadable")?;
        if states.count < 2 {
            return Err("state table has no state slots");
        }
        let expected =
            engine_sites::at(base, cstate::VTABLE_RVA).ok_or("state vtable overflow")? as u64;
        let mut found_state = false;
        let mut first = 1;
        let mut pointers = [0_u8; 64 * 8];
        while first < states.count {
            let count = ((states.count - first) as usize).min(64);
            let address = states
                .element(first, 8)
                .ok_or("variable state scope address overflow")?;
            if !read_slice(memory, address, &mut pointers[..count * 8]) {
                return Err("variable state scope pointers unreadable");
            }
            for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
                let state = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| "variable state scope pointer overflow")?;
                if state == 0 {
                    continue;
                }
                let mut vtable = [0_u8; 8];
                let mut id = [0_u8; 4];
                let mut container = [0_u8; 8];
                if memory.copy_scattered(
                    [
                        state,
                        state
                            .checked_add(cstate::ID)
                            .ok_or("variable state id address overflow")?,
                        state
                            .checked_add(cstate::VARIABLES)
                            .ok_or("variable state root address overflow")?,
                    ],
                    &mut [&mut vtable, &mut id, &mut container],
                ) != 3
                {
                    return Err("variable state scope fields unreadable");
                }
                if u64::from_le_bytes(vtable) != expected {
                    return Err("variable state vtable mismatch");
                }
                let id = u32::from_le_bytes(id);
                if id != first + index as u32 {
                    return Err("variable state id differs from slot");
                }
                let container = usize::try_from(u64::from_le_bytes(container))
                    .map_err(|_| "variable scope pointer overflow")?;
                found_state = true;
                scope(Scope::State(id), container, None)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        let mut characters = ccharacter_manager::Roster::read(memory, base, game_state)?.cursor();
        while let Some(character) = characters.next_occurrence(memory)? {
            let mut head = Head::default();
            let (id, copied) = ccharacter::identity_with_spans(
                memory,
                base,
                character,
                [VTABLE, STORAGE, SEED].map(|offset| ccharacter::VARIABLES + offset),
                [&mut head.vtable, &mut head.header, &mut head.seed],
            )?;
            let container = character
                .checked_add(ccharacter::VARIABLES)
                .ok_or("character variables address overflow")?;
            head.copied = copied;
            scope(Scope::Character(id), container, Some(head))?;
        }
        Ok(ScopeWalk {
            outcome: if missing == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: missing,
                }
            },
            entries_len,
            required_entries,
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| ScopeWalk {
        outcome: Outcome::Unavailable { reason },
        entries_len: 0,
        required_entries: 0,
        bytes_len: 0,
        required_bytes: 0,
    })
}

fn scope_pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("script variable scope root unreadable")
}

#[allow(clippy::too_many_arguments)]
fn store_rows(
    memory: &impl Memory,
    base: usize,
    scope: Scope,
    container: usize,
    head: Head,
    mut entries: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    carry: &mut impl VariableCarry,
) -> Result<NameWalk, &'static str> {
    let plan = head
        .store(base, scope, container)
        .map(|store| carry.plan(&store));
    if let Some((token, Some((count, bytes)))) = plan
        && entries.ensure(count)?
        && names.ensure(bytes)?
    {
        carry.carry(token, &mut entries[..count], &mut names[..bytes]);
        return Ok(NameWalk {
            outcome: Outcome::Rows { len: count },
            bytes_len: bytes,
            required_bytes: bytes,
        });
    }
    let captured = walk_head_buffer(memory, base, head, entries.suffix(0), names.suffix(0));
    if let Some((token, _)) = plan {
        let count = captured.outcome.len();
        carry.read(
            token,
            captured.outcome,
            &entries[..count],
            &names[..captured.bytes_len],
        );
    }
    Ok(captured)
}

pub fn read_store_carried(
    memory: &impl Memory,
    base: usize,
    scope: Scope,
    container: usize,
    entries: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
    carry: &mut impl VariableCarry,
) -> Result<(NameWalk, Option<u32>), &'static str> {
    if container == 0 {
        return Err("script variable scope root is null");
    }
    let head = read_head(memory, container);
    let rows = store_rows(memory, base, scope, container, head, entries, names, carry)?;
    Ok((rows, head.seed()))
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    container: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        container,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    container: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    walk_head_buffer(memory, base, read_head(memory, container), output, names)
}

#[derive(Clone, Copy, Default)]
struct Head {
    vtable: [u8; 8],
    header: [u8; HEADER],
    seed: [u8; 4],
    copied: usize,
    overflow: bool,
}

impl Head {
    fn seed(&self) -> Option<u32> {
        (self.copied == 3).then(|| u32::from_le_bytes(self.seed))
    }

    fn store(&self, base: usize, scope: Scope, container: usize) -> Option<Store> {
        let expected = engine_sites::at(base, VTABLE_RVA)?;
        (self.copied >= 2 && !self.overflow && u64::from_le_bytes(self.vtable) == expected as u64)
            .then_some(Store {
                scope,
                container,
                header: self.header,
                seed: self.seed(),
            })
    }
}

fn read_head(memory: &impl Memory, container: usize) -> Head {
    let mut head = Head::default();
    match container.checked_add(STORAGE) {
        Some(header) => {
            head.copied = memory.copy_scattered(
                [container, header, container + SEED],
                &mut [&mut head.vtable, &mut head.header, &mut head.seed],
            );
        }
        None => {
            head.overflow = true;
            head.copied = usize::from(read_slice(memory, container, &mut head.vtable));
        }
    }
    head
}

fn walk_head_buffer(
    memory: &impl Memory,
    base: usize,
    head: Head,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        if head.copied == 0 || u64::from_le_bytes(head.vtable) != expected as u64 {
            return Err("variables vtable mismatch");
        }
        if head.overflow {
            return Err("variables address overflow");
        }
        if head.copied == 1 {
            return Err("variables header unreadable");
        }
        let header = head.header;
        let storage = u64::from_le_bytes(header[..8].try_into().unwrap());
        let storage = usize::try_from(storage).map_err(|_| "variables storage overflow")?;
        let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let tail = header[16];
        let empty = engine_sites::at(base, EMPTY_STORAGE_RVA).ok_or("module address overflow")?;
        if storage == empty {
            if count != 0 || mask != 0 || tail != 0 {
                return Err("variables empty sentinel header invalid");
            }
            return Ok(NameWalk {
                outcome: Outcome::Rows { len: 0 },
                bytes_len: 0,
                required_bytes: 0,
            });
        }
        let buckets = mask
            .checked_add(1)
            .ok_or("variables bucket count overflow")?;
        if storage == 0
            || !buckets.is_power_of_two()
            || !(2..=1 << 30).contains(&buckets)
            || u32::from(tail) != buckets.trailing_zeros() + 2
            || count > mask
        {
            return Err("variables hash header invalid");
        }
        let slots = (buckets as usize)
            .checked_add(usize::from(tail))
            .ok_or("variables slot count overflow")?;
        let sentinel = slots
            .checked_mul(variable_hash_node::STRIDE)
            .and_then(|span| storage.checked_add(span))
            .and_then(|p| p.checked_add(variable_hash_node::DISTANCE))
            .ok_or("variables storage address overflow")?;
        if read::<u8>(memory, sentinel) != Some(u8::MAX) {
            return Err("variables end sentinel invalid or unreadable");
        }
        output.ensure(count as usize)?;
        let mut len = 0_usize;
        let mut occupied = 0_u32;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        const BLOCK_NODES: usize = 16;
        let mut block = [0_u8; BLOCK_NODES * variable_hash_node::STRIDE];
        for index in 0..slots {
            let pointer = index
                .checked_mul(variable_hash_node::STRIDE)
                .and_then(|offset| storage.checked_add(offset))
                .ok_or("variables node address overflow")?;
            let block_index = index % BLOCK_NODES;
            if block_index == 0 {
                let block_len = (slots - index).min(BLOCK_NODES) * variable_hash_node::STRIDE;
                if !read_slice(memory, pointer, &mut block[..block_len]) {
                    return Err("variables node block unreadable");
                }
            }
            let node = &block[block_index * variable_hash_node::STRIDE
                ..(block_index + 1) * variable_hash_node::STRIDE];
            let distance = node[variable_hash_node::DISTANCE];
            if distance == 0 {
                continue;
            }
            if distance > tail {
                return Err("variables probe distance invalid");
            }
            let hash = u32::from_le_bytes(node[..4].try_into().unwrap());
            let home = (hash & mask) as usize;
            if home.checked_add(usize::from(distance) - 1) != Some(index) {
                return Err("variables probe location invalid");
            }
            occupied = occupied
                .checked_add(1)
                .ok_or("variables entry count overflow")?;
            if occupied > count {
                return Err("variables occupied count mismatch");
            }
            let name_len = u64::from_le_bytes(node[0x18..0x20].try_into().unwrap());
            let capacity = u64::from_le_bytes(node[0x20..0x28].try_into().unwrap());
            if capacity < 15 || name_len > capacity {
                return Err("variable name header invalid");
            }
            let name_len =
                usize::try_from(name_len).map_err(|_| "variable name length overflow")?;
            required_bytes = required_bytes
                .checked_add(name_len)
                .ok_or("variable name byte count overflow")?;
            let source = if capacity == 15 {
                pointer
                    .checked_add(variable_hash_node::KEY)
                    .ok_or("variable name address overflow")?
            } else {
                usize::try_from(u64::from_le_bytes(node[8..16].try_into().unwrap()))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("variable name pointer invalid")?
            };
            let heap = capacity != 15;
            if !heap && node[variable_hash_node::KEY + name_len] != 0 {
                return Err("variable name terminator invalid or unreadable");
            }
            let terminator = if heap {
                source
                    .checked_add(name_len)
                    .ok_or("variable name address overflow")?
            } else {
                0
            };
            let gathered = heap && required_bytes <= names.len();
            if heap && !gathered && read::<u8>(memory, terminator) != Some(0) {
                return Err("variable name terminator invalid or unreadable");
            }
            names.ensure(required_bytes)?;
            let fits = len < output.len() && name_len <= names.len() - bytes_len;
            if gathered {
                let mut end = [1_u8];
                let payload: &mut [u8] = if fits {
                    &mut names[bytes_len..bytes_len + name_len]
                } else {
                    &mut []
                };
                let copied = memory.copy_scattered([terminator, source], &mut [&mut end, payload]);
                if copied == 0 || end[0] != 0 {
                    return Err("variable name terminator invalid or unreadable");
                }
                if fits && copied != 2 {
                    return Err("variable name bytes unreadable");
                }
            }
            if !fits {
                unread_rows += 1;
                continue;
            }
            if !heap {
                names[bytes_len..bytes_len + name_len].copy_from_slice(&node[8..8 + name_len]);
            } else if !gathered
                && !read_slice(memory, source, &mut names[bytes_len..bytes_len + name_len])
            {
                return Err("variable name bytes unreadable");
            }
            output[len] = Row {
                name_start: bytes_len,
                name_len,
                value_raw: i64::from_le_bytes(
                    node[variable_hash_node::VALUE..].try_into().unwrap(),
                ),
            };
            bytes_len += name_len;
            len += 1;
        }
        if occupied != count {
            return Err("variables occupied count mismatch");
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

#[cfg(test)]
mod tests;
