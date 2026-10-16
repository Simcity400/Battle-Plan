//! Scientist specialization records reached through scientist +0x110.
//! The +0x11c count determines the population and order. Each 0x20-byte
//! record contains a u32 token, signed i64 experience and signed i32 level.
//! Experience and level are serialized only while strictly positive.
//! Reporting token values never gate rows or impose an ordering.

/// Specialization record stride.
pub const STRIDE: usize = 0x20;
/// Full specialization token, u32.
pub const TOKEN: usize = 0x00;
/// Signed i64 experience at +0x10; serialized only when positive.
pub const EXPERIENCE: usize = 0x10;
/// Signed i32 level at +0x18; serialized only when positive.
pub const LEVEL: usize = 0x18;
/// Skill-level save writer with independent strictly-positive guards.
pub const SERIALIZE_RVA: usize = 0x0144_e480;
/// Nuclear specialization token low half, for reporting only.
pub const NUCLEAR_TOKEN_PREFIX: u16 = 0x56c6;
/// Naval specialization token low half, for reporting only.
pub const NAVAL_TOKEN_PREFIX: u16 = 0x56c7;
/// Air specialization token low half, for reporting only.
pub const AIR_TOKEN_PREFIX: u16 = 0x56c8;
/// Land specialization token low half, for reporting only.
pub const LAND_TOKEN_PREFIX: u16 = 0x56c9;
