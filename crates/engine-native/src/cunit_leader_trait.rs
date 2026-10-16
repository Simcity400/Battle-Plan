//! CUnitLeaderTrait definitions are individually allocated; their database also holds sentinel objects.

/// CUnitLeaderTrait vtable.
pub const VTABLE_RVA: usize = 0x0292_f160;
/// CUnitLeaderTrait allocation extent.
pub const SIZE: usize = 0xa90;
/// CUnitLeaderTrait lexer-token ID i32.
pub const TOKEN_ID: usize = 0x08;
/// CUnitLeaderTrait token MSVC string.
pub const TOKEN: usize = 0x10;
/// Required experience, signed i32 whole points; scaled to fixed5 for the learning tooltip.
pub const REQUIRED_EXPERIENCE: usize = 0x8a0;
/// CUnitLeaderTrait canonical database ordinal i32.
pub const ORDINAL: usize = 0x8a4;
/// Category mask, u32; its low four bits select database subsets.
pub const CATEGORY_MASK: usize = 0x8ac;
/// CUnitLeaderTrait kind u32.
pub const KIND: usize = 0x920;
/// CUnitLeaderTrait ability-unlock string array data.
pub const ABILITY_DATA: usize = 0x960;
/// CUnitLeaderTrait ability-unlock string count i32.
pub const ABILITY_COUNT: usize = 0x96c;
/// CUnitLeaderTrait ability-unlock string stride.
pub const ABILITY_STRIDE: usize = 0x20;
/// CUnitLeaderTrait experience-bearing kind 2.
pub const EXPERIENCE_KIND_2: u32 = 2;
/// CUnitLeaderTrait experience-bearing kind 3.
pub const EXPERIENCE_KIND_3: u32 = 3;
/// CUnitLeaderTrait constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00ad_4ba0;
/// CUnitLeaderTrait ability predicate.
pub const HAS_ABILITY_RVA: usize = 0x00ad_7d40;
