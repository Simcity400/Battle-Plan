//! Location scope embedded in a raid; pointer-arm pointees have separate layouts.

/// Scope second pointer arm.
pub const SECOND_POINTER: usize = 0x08;
/// Scope third pointer arm.
pub const THIRD_POINTER: usize = 0x10;
/// Scope fifth pointer arm.
pub const FIFTH_POINTER: usize = 0x20;
