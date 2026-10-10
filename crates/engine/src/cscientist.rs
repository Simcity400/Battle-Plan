//! Scientist reached through its character. Trait and specialization vectors
//! use data, u32 capacity, i32 count and allocator headers. Copy construction
//! preserves both live counts; specialization records have stride 0x20.
//! Capacity is not a count and specialization population has no fixed cap.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0296_7e88;
/// Trait-token vector data.
pub const TRAIT_DATA: usize = 0x30;
/// Trait-vector capacity, u32, separate from live count.
pub const TRAIT_CAPACITY: usize = 0x38;
/// Trait-vector signed count used by copying.
pub const TRAIT_COUNT: usize = 0x3c;
/// Specialization-record vector data, stride 0x20.
pub const SKILLS: usize = 0x110;
/// Specialization-vector capacity, u32.
pub const SKILL_CAPACITY: usize = 0x118;
/// Specialization-vector signed count used by copying.
pub const SKILL_COUNT: usize = 0x11c;
/// Scientist-trait primary vtable.
pub const TRAIT_VTABLE_RVA: usize = 0x0270_5460;
/// Trait lexer token, serialized as its token string and used as database key.
pub const TRAIT_TOKEN: usize = 0x08;
/// Active byte set on assignment, cleared on unassignment and serialized.
pub const ACTIVE: usize = 0x130;
/// Scientist allocation size.
pub const SIZE: usize = 0x138;
/// Copy constructor preserving both vector counts and specialization stride.
pub const COPY_CONSTRUCTOR_RVA: usize = 0x013f_6010;
