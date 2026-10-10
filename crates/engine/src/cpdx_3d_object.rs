//! 3D presentation-object base inherited by particle objects.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_9ba0;
/// 3D-object constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022c_2410;
/// Transform, `f32[16]`, identity-initialized without implying world resolution.
pub const TRANSFORM: usize = 0x10;
/// Descriptor pointer supplied as the constructor's second argument.
pub const DESCRIPTOR: usize = 0x58;
/// Stored presentation flags byte.
pub const FLAGS: usize = 0x60;
/// Low bits added to the existing flags byte during construction.
pub const CONSTRUCTOR_FLAGS_MASK: u8 = 7;
/// Enabled bit propagated to the particle runtime.
pub const ENABLED_MASK: u8 = 1;
