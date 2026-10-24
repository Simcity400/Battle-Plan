//! CWarGoalType: the naming override is content's `war_name`, not a war label.

use crate::{Memory, read};

/// Primary type vtable.
pub const VTABLE_RVA: usize = 0x0293_bea0;
/// Declared lexer type token.
pub const TOKEN: usize = 8;
/// Authored war-name override string used by naming.
pub const WAR_NAME: usize = 0x2a8;

pub fn token(memory: &impl Memory, base: usize, pointer: usize) -> Result<u32, &'static str> {
    if pointer == 0
        || read::<u64>(memory, pointer) != base.checked_add(VTABLE_RVA).map(|p| p as u64)
    {
        return Err("war goal type vtable mismatch");
    }
    pointer
        .checked_add(TOKEN)
        .and_then(|p| read(memory, p))
        .ok_or("war goal type token unreadable")
}
