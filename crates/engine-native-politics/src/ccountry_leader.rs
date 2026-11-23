pub use engine_native_parts::ccountry_leader_layout::{CHARACTER, VTABLE_RVA};

use engine_sites::Rva;
pub const LEADER_ID: usize = 0x148;
pub const TRAITS: usize = 0x50;
pub const TRAIT_VTABLE_RVA: Rva = Rva(0x0286_78c8);
pub const TRAIT_TOKEN: usize = 0x08;
pub const TRAIT_ORDINAL: usize = 0x1a0;

pub const CURRENT_IDEOLOGY: usize = 0x140;
pub const IDEOLOGY_VTABLE_RVA: Rva = Rva(0x029b_a2d8);
pub const NULL_IDEOLOGY_VTABLE_RVA: Rva = Rva(0x029b_a558);

pub fn current_ideology(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    role: usize,
) -> Result<Option<u32>, &'static str> {
    let pointer = engine_native_memory::read::<u64>(memory, engine_native::ccharacter::at(role, CURRENT_IDEOLOGY)?)
        .ok_or("country leader current ideology pointer unreadable")?;
    let pointer =
        usize::try_from(pointer).map_err(|_| "country leader current ideology pointer overflow")?;
    if pointer == 0 {
        return Ok(None);
    }
    let header = engine_native_memory::read::<[u8; 12]>(memory, pointer)
        .ok_or("country leader current ideology unreadable")?;
    let expected = engine_sites::at(base, IDEOLOGY_VTABLE_RVA).ok_or("module address overflow")?;
    let null_expected =
        engine_sites::at(base, NULL_IDEOLOGY_VTABLE_RVA).ok_or("module address overflow")?;
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    if vtable != expected as u64 && vtable != null_expected as u64 {
        return Err("country leader current ideology vtable mismatch");
    }
    Ok(Some(u32::from_le_bytes(header[8..].try_into().unwrap())))
}

pub(crate) fn role_key(
    memory: &impl engine_native_memory::Memory,
    character: usize,
    leader: usize,
) -> Result<i32, &'static str> {
    use engine_native::ccharacter;
    use crate::character_leader_node;
    use engine_native_memory::{msvc_tree_node::Cursor, read};
    let mut tree = Cursor::read(memory, character, ccharacter::LEADER_MAP_HEAD)?;
    let mut previous = None;
    let mut found = None;
    while let Some(node) = tree.next(memory)? {
        let value = read::<[u8; 16]>(
            memory,
            ccharacter::at(node, character_leader_node::IDEOLOGY)?,
        )
        .ok_or("country leader role node unreadable")?;
        let key = i32::from_le_bytes(value[..4].try_into().unwrap());
        if previous.is_some_and(|previous| previous >= key) {
            return Err("country leader ideology map order invalid");
        }
        previous = Some(key);
        let pointer = u64::from_le_bytes(value[8..].try_into().unwrap());
        if pointer == 0 {
            return Err("country leader role pointer null");
        }
        if pointer == leader as u64 && found.replace(key).is_some() {
            return Err("country leader occurs in multiple role nodes");
        }
    }
    found.ok_or("party leader missing from character role map")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub character: u32,
    pub country: u32,
    pub ideology: i32,
    pub current_ideology: Option<Option<u32>>,
    pub current_ideology_failure: Option<&'static str>,
}

pub fn walk(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> engine_native_memory::walk::Outcome {
    use engine_native::ccharacter::manager as ccharacter_manager;
    use engine_native_memory::walk::Outcome;
    let result = (|| {
        let mut len = 0;
        let mut unread_rows = 0_u32;
        ccharacter_manager::visit(memory, base, game_state, |entry| {
            match walk_entry(memory, base, entry, output.suffix(len)) {
                Outcome::Rows { len: added } => len += added,
                Outcome::Truncated {
                    len: added,
                    unread_rows: unread,
                } => {
                    len += added;
                    unread_rows = unread_rows
                        .checked_add(unread)
                        .ok_or("country leader row count overflow")?;
                }
                Outcome::Unavailable { reason } => return Err(reason),
            }
            Ok(())
        })?;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn walk_entry(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    entry: engine_native::ccharacter::manager::Entry,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> engine_native_memory::walk::Outcome {
    use engine_native::ccharacter::{self, at};
    use crate::character_leader_node;
    use engine_native_memory::{msvc_tree_node::Cursor as RoleTree, read, walk::Outcome};
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let mut len = 0;
        let mut unread_rows = 0_u32;
        {
            let (character_pointer, character, country) = (entry.pointer, entry.id, entry.country);
            let mut tree = RoleTree::read(memory, character_pointer, ccharacter::LEADER_MAP_HEAD)?;
            output.ensure(tree.count() as usize)?;
            if len == output.len() {
                return Ok(if tree.count() == 0 {
                    Outcome::Rows { len: 0 }
                } else {
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: tree.count(),
                    }
                });
            }
            let mut previous = None;
            let mut ordinal = 0;
            while let Some(node) = tree.next(memory)? {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(tree.count() - ordinal)
                        .ok_or("country leader row count overflow")?;
                    break;
                }
                let value = read::<[u8; 16]>(memory, at(node, character_leader_node::IDEOLOGY)?)
                    .ok_or("country leader ideology unreadable")?;
                let ideology = i32::from_le_bytes(value[..4].try_into().unwrap());
                if previous.is_some_and(|key| key >= ideology) {
                    return Err("country leader ideology map order invalid");
                }
                previous = Some(ideology);
                let role = usize::try_from(u64::from_le_bytes(value[8..].try_into().unwrap()))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("country leader role unreadable or null")?;
                let header = read::<[u8; 16]>(memory, role)
                    .ok_or("country leader role header unreadable")?;
                if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
                    || u64::from_le_bytes(header[CHARACTER..CHARACTER + 8].try_into().unwrap())
                        != character_pointer as u64
                {
                    return Err("country leader role vtable or character parent mismatch");
                }
                let current = current_ideology(memory, base, role);
                output[len] = Row {
                    character,
                    country,
                    ideology,
                    current_ideology: current.as_ref().ok().copied(),
                    current_ideology_failure: current.err(),
                };
                len += 1;
                ordinal += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn unreadable_current_ideology_keeps_the_existing_role_row() {
        use engine_native::ccharacter::{self, manager::Entry};
        use crate::character_leader_node;
        use engine_native_memory::msvc_tree_node;
        let mut bytes = vec![0; 0x400];
        let word = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        word(&mut bytes, ccharacter::LEADER_MAP_HEAD, 0x180);
        word(&mut bytes, ccharacter::LEADER_MAP_HEAD + 8, 1);
        for offset in [
            msvc_tree_node::LEFT,
            msvc_tree_node::PARENT,
            msvc_tree_node::RIGHT,
        ] {
            word(&mut bytes, 0x180 + offset, 0x200);
            word(&mut bytes, 0x200 + offset, 0x180);
        }
        bytes[0x180 + msvc_tree_node::NIL] = 1;
        bytes[0x200 + character_leader_node::IDEOLOGY..0x204 + character_leader_node::IDEOLOGY]
            .copy_from_slice(&(-7_i32).to_le_bytes());
        word(&mut bytes, 0x200 + character_leader_node::LEADER, 0x300);
        word(&mut bytes, 0x300, VTABLE_RVA.0 as u64);
        word(&mut bytes, 0x300 + CHARACTER, 0);
        let entry = Entry {
            pointer: 0,
            id: 7,
            country: 5000,
            roster: 0,
            ordinal: 0,
        };
        let mut rows = [Row::default()];
        assert_eq!(
            walk_entry(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                entry,
                engine_native_memory::walk::Buffer::Exact(&mut rows)
            ),
            engine_native_memory::walk::Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].character, 7);
        assert_eq!(rows[0].ideology, -7);
        assert_eq!(rows[0].current_ideology, None);
        assert_eq!(
            rows[0].current_ideology_failure,
            Some("country leader current ideology pointer unreadable")
        );
        bytes.resize(0x450, 0);
        assert_eq!(
            walk_entry(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                entry,
                engine_native_memory::walk::Buffer::Exact(&mut rows)
            ),
            engine_native_memory::walk::Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].current_ideology, Some(None));
        assert_eq!(rows[0].current_ideology_failure, None);
        word(&mut bytes, 0x300 + CURRENT_IDEOLOGY, 0x480);
        bytes.resize(0x490, 0);
        word(&mut bytes, 0x480, IDEOLOGY_VTABLE_RVA.0 as u64);
        assert_eq!(
            walk_entry(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                entry,
                engine_native_memory::walk::Buffer::Exact(&mut rows)
            ),
            engine_native_memory::walk::Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].current_ideology, Some(Some(0)));
        assert_eq!(rows[0].current_ideology_failure, None);
    }

    #[test]
    fn native_null_ideology_retains_the_unconditional_writer_token() {
        let mut bytes = vec![0; 0x300];
        bytes[CURRENT_IDEOLOGY..CURRENT_IDEOLOGY + 8].copy_from_slice(&0x200_u64.to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
        for vtable in [NULL_IDEOLOGY_VTABLE_RVA.0, IDEOLOGY_VTABLE_RVA.0] {
            bytes[0x200..0x208].copy_from_slice(&(vtable as u64).to_le_bytes());
            assert_eq!(
                current_ideology(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    0,
                    0
                ),
                Ok(Some(0xffff_fffe))
            );
        }
        bytes[0x200..0x208].copy_from_slice(&0x1234_u64.to_le_bytes());
        assert_eq!(
            current_ideology(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0
            ),
            Err("country leader current ideology vtable mismatch")
        );
    }

    #[test]
    fn current_ideology_is_a_token_independent_of_the_role_map_key() {
        let mut bytes = vec![0; 0x300];
        bytes[CURRENT_IDEOLOGY..CURRENT_IDEOLOGY + 8].copy_from_slice(&0x200_u64.to_le_bytes());
        bytes[0x200..0x208].copy_from_slice(&(IDEOLOGY_VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x208..0x20c].copy_from_slice(&0xffff_1234_u32.to_le_bytes());
        assert_eq!(
            current_ideology(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0
            ),
            Ok(Some(0xffff_1234))
        );
        bytes[CURRENT_IDEOLOGY..CURRENT_IDEOLOGY + 8].fill(0);
        assert_eq!(
            current_ideology(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0
            ),
            Ok(None)
        );
        assert_eq!(
            current_ideology(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..CURRENT_IDEOLOGY + 7]
                },
                0,
                0
            ),
            Err("country leader current ideology pointer unreadable")
        );
    }
}
