//! Object behind an operative action's +0x10 pointer; concrete C++ type is unproved.
//!
//! The old network walk interprets +0x18 as CCountryIntelNetwork*, while the
//! A30 mission reader requires CState* there for build/quiet-network missions.
//! This contradiction must be resolved before a state walker uses that field.

/// Operative mission implementation target country reference u32; old a30_non_operation_action_identity.rs:21 and read_mission_product_identity.
pub const TARGET_COUNTRY: usize = 0x10;
/// Operative implementation target pointer; old extractor.rs:8498 network interpretation conflicts with A30 mission CState interpretation at the same +18.
pub const TARGET_POINTER: usize = 0x18;
/// Operative boost-ideology implementation ideology pointer; old a30_non_operation_action_identity.rs:23, class-specific validity still needs proof.
pub const BOOST_IDEOLOGY: usize = 0x40;
