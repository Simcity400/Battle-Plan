//! COperation, the authored espionage-operation definition.

use crate::{Memory, read};

/// Primary vtable at subobject offset zero.
pub const VTABLE_RVA: usize = 0x0292_8c98;
/// Embedded preparation mean-time-to-happen object.
pub const PREPARATION_TIME: usize = 0x10;
/// Lexer token resolved before constructing an operation instance.
pub const TOKEN_ID: usize = 0x08;

/// Reads the exact definition token. `PREPARATION_TIME` is an embedded timing object, not a string;
/// authored names are resolved separately through the token dictionary.
pub fn definition(memory: &impl Memory, base: usize, pointer: usize) -> Result<u32, &'static str> {
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
