//! COperation, the authored espionage-operation definition.

use crate::{Memory, read};

/// COperation primary vtable; build_profile.rs:516 and pinned a729 RTTI confirm the class, subobject offset zero.
pub const VTABLE_RVA: usize = 0x0292_8c98;
/// COperation embedded preparation CMeanTimeToHappen; A6C9F0 calls 544BA0 on +0x10, whose constructor installs that type's vtable.
pub const PREPARATION_TIME: usize = 0x10;
/// COperation native LexerToken; 1189390 resolves this key before constructing an instance.
pub const TOKEN_ID: usize = 0x08;

/// Reads the exact definition token. +0x10 is an embedded timing object, not a
/// string; authored names are resolved separately through the token dictionary.
pub(crate) fn definition(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u32, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("operation module address overflow")?;
    let head =
        read::<[u8; 12]>(memory, pointer).ok_or("operation definition identity unreadable")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation definition vtable mismatch");
    }
    Ok(u32::from_le_bytes(head[TOKEN_ID..].try_into().unwrap()))
}
