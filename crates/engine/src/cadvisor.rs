//! CAdvisor role object; membership in active political ideas determines appointment, not the roster alone.

/// CAdvisor vtable; build_profile.rs:194 and the pinned executable's MSVC RTTI confirm the class.
pub const VTABLE_RVA: usize = 0x0299_0e60;
/// CAdvisor parent CCharacter*; extractor.rs:115 and read_advisor_raw_fields validate this link.
pub const CHARACTER: usize = 0x10;
/// CAdvisor idea-token MSVC string; extractor.rs:116 and attach_log.rs:87338 read its bytes.
pub const IDEA_TOKEN: usize = 0x30;
/// CAdvisor slot MSVC string; extractor.rs:117 and attach_log.rs:87386 read its bytes.
pub const SLOT: usize = 0x80;

// These excluded mutator sites affect advisor roles; their receiver types are not established by the gate.
/// ADVISOR_ERASE_EFFECT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_ERASE_EFFECT_RVA: usize = 0x0035_C330;
/// ADVISOR_ERASE_EFFECT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_ERASE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];

/// ADVISOR_REPLACE_EFFECT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_REPLACE_EFFECT_RVA: usize = 0x0034_7DF0;
/// ADVISOR_REPLACE_EFFECT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_REPLACE_EFFECT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

/// ADVISOR_BULK_COPY function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_BULK_COPY_RVA: usize = 0x00F8_B8A0;
/// ADVISOR_BULK_COPY function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_BULK_COPY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];

/// ADVISOR_INSERT function, named and byte-checked by d47_character_change_gate/src/hooks_windows.rs:21-95; excluded gate hook.
pub const ADVISOR_INSERT_RVA: usize = 0x00F8_D380;
/// ADVISOR_INSERT function prefix; d47_character_change_gate/src/hooks_windows.rs:32-73 checked these bytes before the excluded hook.
pub const ADVISOR_INSERT_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub character: u32,
    pub country: u32,
    /// Position in this character's native string-keyed role map.
    pub ordinal: u32,
    pub key_start: usize,
    pub key_len: usize,
    pub idea_start: usize,
    pub idea_len: usize,
    pub slot_start: usize,
    pub slot_len: usize,
}

/// Captures persistent advisor roles, with exact map key, idea token and slot.
/// A role's existence does not assert appointment. Appointment requires joining
/// the same-hour political active-idea vector; its ordinal supplies cabinet order.
/// Native order is retained; sorting and row-key validation belong off-thread.
pub fn walk(
    memory: &impl crate::Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    use crate::{ccharacter::NameWalk, ccharacter_manager, walk::Outcome};
    let result = (|| {
        let mut len = 0;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        ccharacter_manager::visit(memory, base, game_state, |entry| {
            let end = if unread_rows == 0 { output.len() } else { len };
            let result = walk_entry(
                memory,
                base,
                entry,
                &mut output[len..end],
                &mut bytes[bytes_len..],
            );
            let (added, unread) = match result.outcome {
                Outcome::Rows { len } => (len, 0),
                Outcome::Truncated { len, unread_rows } => (len, unread_rows),
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for row in &mut output[len..len + added] {
                row.key_start += bytes_len;
                row.idea_start += bytes_len;
                row.slot_start += bytes_len;
            }
            len += added;
            bytes_len += result.bytes_len;
            required_bytes = required_bytes
                .checked_add(result.required_bytes)
                .ok_or("advisor string size overflow")?;
            unread_rows = unread_rows
                .checked_add(unread)
                .ok_or("advisor row count overflow")?;
            Ok(())
        })?;
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

/// Walks one authenticated character. Native keys use unsigned hash order,
/// with a locale-sensitive comparator only on hash collisions.
pub fn walk_entry(
    memory: &impl crate::Memory,
    base: usize,
    entry: crate::ccharacter_manager::Entry,
    output: &mut [Row],
    bytes: &mut [u8],
) -> crate::ccharacter::NameWalk {
    use crate::{
        ccharacter::{self, Literal, NameWalk, RoleTree, at},
        character_advisor_node,
        walk::Outcome,
    };
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut len = 0;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        {
            let (character_pointer, character, country) = (entry.pointer, entry.id, entry.country);
            let mut tree = RoleTree::read(memory, character_pointer, ccharacter::ADVISOR_MAP_HEAD)?;
            let mut ordinal = 0;
            let mut previous_hash = None;
            if len == output.len() {
                return Ok(NameWalk {
                    outcome: if tree.count() == 0 {
                        Outcome::Rows { len: 0 }
                    } else {
                        Outcome::Truncated {
                            len: 0,
                            unread_rows: tree.count(),
                        }
                    },
                    bytes_len: 0,
                    required_bytes: 0,
                });
            }
            while let Some(node) = tree.next(memory)? {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(tree.count() - ordinal)
                        .ok_or("advisor row count overflow")?;
                    break;
                }
                let mut role_pointer = [0; 8];
                let mut key_header = [0; 32];
                let mut hash = [0; 4];
                if memory.copy_scattered(
                    [
                        at(node, character_advisor_node::ADVISOR)?,
                        at(node, character_advisor_node::KEY)?,
                        at(node, character_advisor_node::KEY_HASH)?,
                    ],
                    &mut [&mut role_pointer, &mut key_header, &mut hash],
                ) != 3
                {
                    return Err("advisor role node unreadable");
                }
                let role = usize::try_from(u64::from_le_bytes(role_pointer))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("advisor role unreadable or null")?;
                let mut vtable = [0; 8];
                let mut parent = [0; 8];
                let mut idea_header = [0; 32];
                let mut slot_header = [0; 32];
                if memory.copy_scattered(
                    [
                        role,
                        at(role, CHARACTER)?,
                        at(role, IDEA_TOKEN)?,
                        at(role, SLOT)?,
                    ],
                    &mut [&mut vtable, &mut parent, &mut idea_header, &mut slot_header],
                ) != 4
                {
                    return Err("advisor role fields unreadable");
                }
                if u64::from_le_bytes(vtable) != expected as u64
                    || u64::from_le_bytes(parent) != character_pointer as u64
                {
                    return Err("advisor role vtable or character parent mismatch");
                }
                let key = Literal::from_header(at(node, character_advisor_node::KEY)?, key_header)?;
                let hash = u32::from_le_bytes(hash);
                if previous_hash.is_some_and(|previous| previous > hash) {
                    return Err("advisor role map hash order invalid");
                }
                if previous_hash == Some(hash) {
                    return Err("advisor role hash collision needs native locale comparison");
                }
                previous_hash = Some(hash);
                let idea = Literal::from_header(at(role, IDEA_TOKEN)?, idea_header)?;
                let slot = Literal::from_header(at(role, SLOT)?, slot_header)?;
                Literal::validate(memory, [&key, &idea, &slot])?;
                let needed = key
                    .len
                    .checked_add(idea.len)
                    .and_then(|n| n.checked_add(slot.len))
                    .ok_or("advisor string size overflow")?;
                required_bytes = required_bytes
                    .checked_add(needed)
                    .ok_or("advisor string size overflow")?;
                if needed > bytes.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(tree.count() - ordinal)
                        .ok_or("advisor row count overflow")?;
                    break;
                }
                let key_start = bytes_len;
                let idea_start = key_start + key.len;
                let slot_start = idea_start + idea.len;
                Literal::copy_all(
                    memory,
                    [&key, &idea, &slot],
                    &mut bytes[key_start..slot_start + slot.len],
                )?;
                output[len] = Row {
                    character,
                    country,
                    ordinal,
                    key_start,
                    key_len: key.len,
                    idea_start,
                    idea_len: idea.len,
                    slot_start,
                    slot_len: slot.len,
                };
                len += 1;
                bytes_len += needed;
                ordinal += 1;
            }
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
