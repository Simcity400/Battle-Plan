//! Engine child-list header returned by the overlap list interface; concrete template name is not retained.

/// GUI child-list first node pointer +0.
pub const HEAD: usize = 0;
/// GUI child-list last node pointer +8.
pub const TAIL: usize = 0x08;
/// GUI child-list count u32 +0x10.
pub const COUNT: usize = 0x10;
