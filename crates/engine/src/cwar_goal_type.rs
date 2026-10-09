//! CWarGoalType: the naming override is content's `war_name`, not a war label.

use crate::{Memory, read};

/// Primary vtable, installed by the type constructor; RTTI at VA1429258A0.
pub const VTABLE_RVA: usize = 0x0292_58a0;
/// Lexer type token; CWarGoalType::Read@140A2C950 and SerializeData@1415CE460.
pub const TOKEN: usize = 8;
/// MSVC string loaded from `war_name` (token 0x2C18), read by 140D0BCA0.
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
