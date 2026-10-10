//! CUnitLeaderDatabase holds canonical trait and skill vectors plus derived category subsets.
//! Exact-vtable filtering is required: sentinel objects can occur beyond the null slot.

/// CUnitLeaderDatabase pointer global.
pub const SINGLETON_RVA: usize = 0x0331_6d00;
/// CUnitLeaderDatabase vtable.
pub const VTABLE_RVA: usize = 0x0292_f230;
/// CUnitLeaderDatabase allocation extent.
pub const SIZE: usize = 0x420;
/// Completed-load count, i32; a nonzero value refuses another completed load.
pub const LOADED_COUNT: usize = 0x14;
/// CUnitLeaderDatabase canonical CUnitLeaderTrait* data.
pub const TRAIT_DATA: usize = 0x58;
/// CUnitLeaderDatabase canonical trait capacity i32.
pub const TRAIT_CAPACITY: usize = 0x60;
/// CUnitLeaderDatabase canonical trait count i32.
pub const TRAIT_COUNT: usize = 0x64;
/// Canonical trait-vector allocator pointer.
pub const TRAIT_ALLOCATOR: usize = 0x68;
/// Derived trait vectors selected by the trait category mask.
pub const TRAIT_SHARDS: usize = 0x70;
/// Canonical skill-definition vector, including a null skill.
pub const SKILL_DATA: usize = 0xd0;
/// CUnitLeaderDatabase canonical skill capacity i32.
pub const SKILL_CAPACITY: usize = 0xd8;
/// CUnitLeaderDatabase canonical skill count i32.
pub const SKILL_COUNT: usize = 0xdc;
/// CUnitLeaderDatabase canonical skill allocator pointer.
pub const SKILL_ALLOCATOR: usize = 0xe0;
/// CUnitLeaderDatabase derived skill-vector array.
pub const SKILL_SHARDS: usize = 0xe8;
/// CUnitLeaderDatabase category-vector stride.
pub const SHARD_STRIDE: usize = 0x18;
/// CUnitLeaderDatabase trait and skill shard count.
pub const SHARD_COUNT: usize = 4;
/// Preload byte; prevents definition creation while set.
pub const FIRST_PASS: usize = 0x418;
/// CUnitLeaderDatabase singleton creator.
pub const CREATE_SINGLETON_RVA: usize = 0x0017_3950;
/// Constructor; creates canonical vectors and null definitions.
pub const CONSTRUCTOR_RVA: usize = 0x00ad_4190;
/// Trait append; all definitions enter the canonical vector.
pub const APPEND_TRAIT_RVA: usize = 0x00ad_7890;
/// Trait-subset builder from category bits.
pub const BUILD_TRAIT_SHARDS_RVA: usize = 0x00ad_6030;
/// Category-skill lookup in the corresponding derived vector.
pub const LOOKUP_SKILL_RVA: usize = 0x00ad_ace0;
