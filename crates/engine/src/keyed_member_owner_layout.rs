//! Owner layout for two members processed with opaque identifiers.
//! The identifiers are call arguments, separate from object offsets.

/// Member passed to the primary keyed handler.
pub const MEMBER: usize = 0x3b0;

/// Opaque identifier paired with MEMBER in the helper call.
pub const MEMBER_ID: u32 = 0x414d;

/// Member passed to the secondary keyed handler on the same owner.
pub const OTHER_MEMBER: usize = 0x4d0;

/// Opaque identifier paired with OTHER_MEMBER in the helper call.
pub const OTHER_MEMBER_ID: u32 = 0x3e91;
