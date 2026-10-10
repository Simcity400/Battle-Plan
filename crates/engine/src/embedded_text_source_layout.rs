//! Layout of a text source supplied separately from the lookup caller.
//! This owner is distinct from the caller and other text-source layouts.

/// Embedded string supplied to a lookup and then a string-reading helper.
pub const TEXT: usize = 0x3b0;
