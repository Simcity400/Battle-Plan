//! Equipment-group record used by the MIO bonus UI; actual engine class name unknown.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_bonus_hook.rs.

/// MIO bonus equipment-group record stride; archived hook:578 advances by 0x28 bytes.
pub const STRIDE: usize = 0x28;
/// MIO bonus group +0x20 semantic key, native u32; archived hook:581 reads the row identity.
pub const SEMANTIC_KEY: usize = 0x20;
