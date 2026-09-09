//! Sprite lookup on CGraphics' embedded hash table; the exact template class is unproved.
//! Function metadata only: the old event acquisition hook is excluded.

pub const LOOKUP_RVA: usize = 0x00a2_d0d0;
/// Sprite lookup entry bytes; fired_event_popups_live/src/a729_inline_hook.rs:106-108 checked the pinned executable.
pub const LOOKUP_SENTINEL: [u8; 15] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
];
