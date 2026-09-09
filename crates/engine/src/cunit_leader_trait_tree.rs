//! CUnitLeaderTraitTree presentation collection, named by the archived A39 dossier.

/// CUnitLeaderTraitTree item-pointer vector, read by character_presentation_live/src/a729_hook.rs:2141.
pub const ITEMS: usize = 0x28;
/// Item-vector capacity (u32), checked against count in a729_hook.rs:2127.
pub const ITEM_CAPACITY: usize = 0x30;
/// Item-vector count (u32), read by a729_hook.rs:2122.
pub const ITEM_COUNT: usize = 0x34;
/// Vector element is one x64 CUnitLeaderTraitTreeItem pointer, walked by a729_hook.rs:2150.
pub const ITEM_STRIDE: usize = 8;
pub const APPEND_ITEM_RVA: usize = 0x016b_16c0;
