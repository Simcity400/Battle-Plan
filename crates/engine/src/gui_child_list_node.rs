//! Engine linked-list node used by list and overlapping-element widgets; concrete template name is not retained.

/// GUI child-list node item pointer +0.
pub const ITEM: usize = 0;
/// GUI child-list previous node pointer +8.
pub const PREVIOUS: usize = 0x08;
/// GUI child-list next node pointer +0x10.
pub const NEXT: usize = 0x10;
