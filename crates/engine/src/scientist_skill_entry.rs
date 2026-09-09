//! Entry in the array reached through `CScientist +0x110`; native class name unknown.
//!
//! Sources: `scientists_live_lane/src/lib.rs:22-24` and archived collector
//! `read_project_scientist_skills`. The old reader assumes four entries in
//! nuclear, naval, air, land order, guarded by the build-specific token ids.

/// Scientist skill entry stride 0x20; independently used by collector and `scientists_live_lane`.
pub const STRIDE: usize = 0x20;
/// Scientist skill entry +0x00 specialization token prefix u16; full token width is not proved by the old reader.
pub const TOKEN_PREFIX: usize = 0x00;
/// Scientist skill entry +0x18 level i32; both old readers retain the raw integer.
pub const LEVEL: usize = 0x18;
/// Nuclear skill token prefix 0x56c6; pinned-build observation, not a portable specialization ordering rule.
pub const NUCLEAR_TOKEN_PREFIX: u16 = 0x56c6;
/// Naval skill token prefix 0x56c7; old collector checks base 0x56c6 plus array index 1.
pub const NAVAL_TOKEN_PREFIX: u16 = 0x56c7;
/// Air skill token prefix 0x56c8; old collector checks base 0x56c6 plus array index 2.
pub const AIR_TOKEN_PREFIX: u16 = 0x56c8;
/// Land skill token prefix 0x56c9; old collector checks base 0x56c6 plus array index 3.
pub const LAND_TOKEN_PREFIX: u16 = 0x56c9;
