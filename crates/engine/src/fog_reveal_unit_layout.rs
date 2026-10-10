//! Fog-skip receiver layout; its primary/body relationship is unspecified.
//! Matching the standard-template offset does not authenticate that identity.

/// Nullable skip-record pointer; null supplies zero, otherwise reads the skip byte.
pub const SKIP_RECORD_POINTER: usize = 0x3b8;
