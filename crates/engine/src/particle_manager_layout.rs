//! Manager of live particle runtimes and authored types. Census publication
//! precedes instance anchor and scale writes; it does not establish hourly consistency.

/// Particle-manager singleton pointer.
pub const SINGLETON_RVA: usize = 0x035c_18e8;
/// Particle-manager allocation size.
pub const SIZE: usize = 0x40;
/// Live particle-runtime pointer array.
pub const INSTANCES: usize = 8;
/// Live-array signed i32 capacity.
pub const INSTANCE_CAPACITY: usize = 0x10;
/// Live-array signed i32 count.
pub const INSTANCE_COUNT: usize = 0x14;
/// Live-array allocator vtable.
pub const INSTANCE_ALLOCATOR: usize = 0x18;
/// Authored-type registry pointer array.
pub const TYPES: usize = 0x20;
/// Type-registry signed i32 capacity.
pub const TYPE_CAPACITY: usize = 0x28;
/// Type-registry signed i32 count.
pub const TYPE_COUNT: usize = 0x2c;
/// Type-registry allocator vtable.
pub const TYPE_ALLOCATOR: usize = 0x30;
/// Trailing u32 initialized to zero; role is unknown.
pub const UNKNOWN_U32: usize = 0x38;
/// Trailing byte initialized to zero; role is unknown.
pub const UNKNOWN_BYTE: usize = 0x3c;
/// Pointer width shared by both manager arrays.
pub const POINTER_STRIDE: usize = 8;
/// Particle-manager construction and publication entry.
pub const CONSTRUCTOR_RVA: usize = 0x023e_8ee0;
/// Particle-manager drain and destruction entry.
pub const DESTRUCTOR_RVA: usize = 0x023e_91a0;
/// Particle-manager singleton getter.
pub const GETTER_RVA: usize = 0x023e_8b80;
/// Runtime enumeration split by instance enable state.
pub const ENABLED_CENSUS_RVA: usize = 0x023e_8500;
/// Runtime type rebinding that leaves the census header unchanged.
pub const REBIND_CENSUS_RVA: usize = 0x023e_9100;
