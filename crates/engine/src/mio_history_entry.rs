//! Organisation history hash-table entry; actual engine class name unknown.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_history_hook.rs; this is the entry, not the organisation object.

/// MIO history entry stride; archived hook:624/638 walks the retained 0x40-byte layout.
pub const STRIDE: usize = 0x40;
/// MIO history entry +0x04 signed i8 probe distance; archived hook:631 treats nonzero as occupied.
pub const DISTANCE: usize = 0x04;
/// MIO history entry +0x08 pair of u32 object-reference words; archived hook:480/556 resolves row identity.
pub const ITEM_REFERENCE: usize = 0x08;
/// MIO history entry +0x30 active flag, native u8; archived hook:484 filters variant candidates.
pub const ACTIVE: usize = 0x30;
/// MIO history entry +0x38 production count, native i32; archived hook:558 requires a positive value.
pub const PRODUCTION_COUNT: usize = 0x38;
