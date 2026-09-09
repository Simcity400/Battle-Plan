//! Observed slot in COperationTokensDatabase's numeric-id map.
//!
//! The old source does not recover this non-polymorphic entry's C++ type name.

/// Operation-token database map slot stride; old diplomacy_intel_operatives_live/src/extractor.rs:91,10720 advances by 0x38 bytes.
pub const STRIDE: usize = 0x38;
/// Operation-token database slot occupancy byte; old diplomacy_intel_operatives_live/src/extractor.rs:10721 skips zero and accepts any nonzero value.
pub const OCCUPIED: usize = 0x04;
/// Operation-token database numeric id, u32; old diplomacy_intel_operatives_live/src/extractor.rs:10724 uses it as the membership-vector lookup key.
pub const NUMERIC_ID: usize = 0x08;
/// Operation-token database definition pointer, u64; old diplomacy_intel_operatives_live/src/extractor.rs:10725-10730 follows it to COperationToken +0x50.
pub const DEFINITION: usize = 0x10;
