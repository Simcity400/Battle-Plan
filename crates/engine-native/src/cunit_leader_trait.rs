//! CUnitLeaderTrait definitions are individually allocated; their database also holds sentinel objects.

/// CUnitLeaderTrait vtable.
pub const VTABLE_RVA: usize = 0x0294_57a0;
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
/// CUnitLeaderTrait constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00ae_3c60;
