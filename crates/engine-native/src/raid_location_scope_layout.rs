//! Location scope embedded in a raid; pointer-arm pointees have separate layouts.

/// Scope second pointer arm.
pub const SECOND_POINTER: usize = 0x08;
/// Scope third pointer arm.
pub const THIRD_POINTER: usize = 0x10;
/// Scope fifth pointer arm.
pub const FIFTH_POINTER: usize = 0x20;
/// Country resolver selecting scope arms in priority order.
pub const COUNTRY_RESOLVER_RVA: usize = 0x00fd_1ad0;
/// Scope province resolver.
pub const PROVINCE_RESOLVER_RVA: usize = 0x00fd_1760;
/// First-arm country helper.
pub const FIRST_ARM_COUNTRY_HELPER_RVA: usize = 0x010c_62f0;
