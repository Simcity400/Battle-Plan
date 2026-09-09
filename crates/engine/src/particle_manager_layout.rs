//! Manager of live particle runtimes and registered authored types; C++ class name is unconfirmed.
//! Retained from XMAP20/effect_census_authority.json's exact-build constructor and census analysis.
//! Census publication precedes CPdxParticleObject's anchor/scale writes; no hourly consistency is implied.

/// Particle manager singleton pointer, returned by FUN1423e8b80; effect_census_authority.json:164-168,253-255.
pub const SINGLETON_RVA: usize = 0x035c_18e8;
/// Particle manager allocation size, FUN1423e8ee0 as documented by effect_census_authority.json:171-175,255.
pub const SIZE: usize = 0x40;
/// Particle manager live runtime pointer array, effect_census_authority.json:267-281 corroborates constructor, append and removal.
pub const INSTANCES: usize = 8;
/// Particle manager live-array capacity i32, constructor zeroes it; effect_census_authority.json:270-281.
pub const INSTANCE_CAPACITY: usize = 0x10;
/// Particle manager live-array count i32, bounded by statistics/removal; effect_census_authority.json:271-280.
pub const INSTANCE_COUNT: usize = 0x14;
/// Particle manager live-array allocator vtable, constructor stores it; effect_census_authority.json:272-280.
pub const INSTANCE_ALLOCATOR: usize = 0x18;
/// Particle manager authored-type registry pointer array, effect_census_authority.json:283-299 confirms constructor and lookup.
pub const TYPES: usize = 0x20;
/// Particle manager authored-type registry capacity i32, constructor zeroes it; effect_census_authority.json:287-299.
pub const TYPE_CAPACITY: usize = 0x28;
/// Particle manager authored-type registry count i32, effect_census_authority.json:288-298 confirms lookup and removal.
pub const TYPE_COUNT: usize = 0x2c;
/// Particle manager authored-type allocator vtable, constructor stores it; effect_census_authority.json:289-298.
pub const TYPE_ALLOCATOR: usize = 0x30;
/// Particle manager trailing u32 initialized to zero, role unknown; effect_census_authority.json:301-305.
pub const UNKNOWN_U32: usize = 0x38;
/// Particle manager trailing byte initialized to zero, role unknown; effect_census_authority.json:302-305.
pub const UNKNOWN_BYTE: usize = 0x3c;
/// Both manager arrays store x64 pointers, effect_census_authority.json:273,290.
pub const POINTER_STRIDE: usize = 8;
/// FUN1423e8ee0 constructs and publishes the particle manager, effect_census_authority.json:171-175.
pub const CONSTRUCTOR_RVA: usize = 0x023e_8ee0;
/// FUN1423e91a0 drains and destroys the particle manager, effect_census_authority.json:206-210.
pub const DESTRUCTOR_RVA: usize = 0x023e_91a0;
/// FUN1423e8b80 returns the particle manager singleton, effect_census_authority.json:164-168.
pub const GETTER_RVA: usize = 0x023e_8b80;
/// FUN1423e8500 enumerates manager runtimes and splits by instance+0x54, effect_census_authority.json:153-157.
pub const ENABLED_CENSUS_RVA: usize = 0x023e_8500;
/// FUN1423e9100 enumerates runtimes and rebinds each type without changing the census header, effect_census_authority.json:185-190.
pub const REBIND_CENSUS_RVA: usize = 0x023e_9100;
