//! Observed target-country slot in CCountryOperationTokenManager's map.
//!
//! The entry's C++ type name is unknown. Its value is a vector of native u32 token
//! ids; the old reader's 256-token ceiling is a capacity policy, not engine layout.

/// Operation-token target map slot stride; old diplomacy_intel_operatives_live/src/extractor.rs:95,10788 advances by 0x28 bytes.
pub const STRIDE: usize = 0x28;
/// Operation-token target slot occupancy byte; old diplomacy_intel_operatives_live/src/extractor.rs:10789 skips zero and accepts any nonzero value.
pub const OCCUPIED: usize = 0x04;
/// Operation-token target country tag database id, u32; old diplomacy_intel_operatives_live/src/extractor.rs:10792 checks it against accepted country roots.
pub const TARGET_TAG_ID: usize = 0x08;
/// Operation-token vector data pointer, u64; old diplomacy_intel_operatives_live/src/extractor.rs:10793-10794 reads the slot's +0x10 pointer.
pub const TOKENS: usize = 0x10;
/// Operation-token vector element count, i32; old diplomacy_intel_operatives_live/src/extractor.rs:10795-10796 requires a positive value before reading.
pub const TOKEN_COUNT: usize = 0x1c;
/// Operation-token vector element width, native u32 ids; old diplomacy_intel_operatives_live/src/extractor.rs:10805-10808 reads four bytes per token.
pub const TOKEN_STRIDE: usize = 0x04;
