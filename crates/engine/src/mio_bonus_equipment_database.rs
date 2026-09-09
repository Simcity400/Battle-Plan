//! Equipment database reached from the MIO organisation; actual engine class name unknown.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_bonus_hook.rs; gameplay ownership and hourly safety remain unproven.

/// MIO bonus database +0x28 optional initial-trait source pointer; archived hook:384 tests its presence.
pub const OPTIONAL_INITIAL_TRAIT: usize = 0x28;
/// MIO bonus database +0x98 equipment-group array pointer; archived hook:397 walks 0x28-byte records.
pub const GROUPS: usize = 0x98;
/// MIO bonus database +0xa4 equipment-group count, native i32; archived hook:393 bounds the walk.
pub const GROUP_COUNT: usize = 0xa4;
