//! Layout of an updated nested object selected through its caller.
//! The caller's pointer at +0x48 selects this owner; matching field offsets do
//! not make it the same owner as a dated collection.

/// Nested object's member supplied to the update helper.
pub const MEMBER: usize = 0x3b0;

/// Byte set to one after the nested member operation.
pub const FLAG: usize = 0x3f9;
