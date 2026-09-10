//! CCountryLeader role object, reached through CCharacter's ideology map or a political party vector.

/// CCountryLeader vtable; build_profile.rs:192 and the pinned executable's MSVC RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x0296_a720;
/// CCountryLeader parent CCharacter*; extractor.rs:113 and read_vtable_and_character_ptr validate this link.
pub const CHARACTER: usize = 0x08;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub character: u32,
    pub country: u32,
    /// The native signed key of CCharacter's ideology-to-role map.
    pub ideology: i32,
}

/// Reads every persistent country-leader role, including roles that are not the
/// active ruler. Ruling-party membership is a separate political-state join.
/// Native order is retained; sorting and row-key validation belong off-thread.
pub fn walk(
    memory: &impl crate::Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
) -> crate::walk::Outcome {
    use crate::{ccharacter_manager, walk::Outcome};
    let result = (|| {
        let mut len = 0;
        let mut unread_rows = 0_u32;
        ccharacter_manager::visit(memory, base, game_state, |entry| {
            match walk_entry(memory, base, entry, &mut output[len..]) {
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

/// Walks one authenticated character from the current shared roster pass.
pub fn walk_entry(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
) -> crate::walk::Outcome {
    use crate::{
        ccharacter::{self, RoleTree, at},
        character_leader_node, read,
        walk::Outcome,
    };
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut len = 0;
        let mut unread_rows = 0_u32;
        {
            let (character_pointer, character, country) = (entry.pointer, entry.id, entry.country);
            let mut tree = RoleTree::read(memory, character_pointer, ccharacter::LEADER_MAP_HEAD)?;
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
                output[len] = Row {
                    character,
                    country,
                    ideology,
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
