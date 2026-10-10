//! Layout of a recursive attribute record.
//! This owner remains separate from an authored particle subsystem record.

/// Attribute member passed as the first argument to its checker.
pub const MEMBER: usize = 0x3b0;

/// 32-bit count controlling child-record traversal.
pub const CHILD_COUNT: usize = 0x6fc;
