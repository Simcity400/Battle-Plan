//! COperationTokensDatabase root and definition map.
//!
//! The pinned executable contains this RTTI name. The old reader brackets the
//! global pointer and map bytes, which does not establish hourly consistency.

/// COperationTokensDatabase singleton pointer RVA; old diplomacy_intel_operatives_live/src/extractor.rs:89,10702-10707 reads a stable u64 pointer.
pub const SINGLETON_RVA: usize = 0x0331_6be8;
/// COperationTokensDatabase inline numeric-id map; old diplomacy_intel_operatives_live/src/extractor.rs:90,10708-10715 uses the operation_token_map header.
pub const DEFINITIONS: usize = 0x40;
