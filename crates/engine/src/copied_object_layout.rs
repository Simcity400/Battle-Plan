//! Shared member layout of copy source and destination objects.
//! The layout describes copied fields without assuming a complete object size.

/// Member supplied to the copy helper on both source and destination.
pub const MEMBER: usize = 0x3b0;

/// Eight-byte field copied between the two object bases.
pub const WIDE_FIELD: usize = 0x7cc;
