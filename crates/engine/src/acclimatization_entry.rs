//! Native category/weight entry in the division acclimatization vector.

/// Category/weight entry stride.
pub const STRIDE: usize = 0x10;
/// Registered category pointer.
pub const CATEGORY: usize = 0x00;
/// Raw i64 weight.
pub const WEIGHT: usize = 0x08;

/// One entry with its category resolved to the category's token.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// [`crate::acclimatization_category::TOKEN`] of the entry's category.
    pub category: u32,
    pub weight: i64,
}
