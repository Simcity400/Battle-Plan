//! Inline sorted modifier accumulator at CCountry+0x5b8.
//! These readers do not establish the concrete C++ template/class name.

/// Modifier accumulator data pointer; old build_profile.rs:98 and compact_contract/tactic_weight.rs:23.
pub const DATA: usize = 0x00;
/// Modifier accumulator capacity u32; old compact_contract/tactic_weight.rs:24 and tactic-weight reader.
pub const CAPACITY: usize = 0x08;
/// Modifier accumulator count; old D57/D74 read i32 and tactic-weight reads u32 at the same +0xc word.
pub const COUNT: usize = 0x0c;
pub const VALUE_RVA: usize = 0x0055_1820;
