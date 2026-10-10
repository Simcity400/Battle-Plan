//! Sorted neighbor/level pair in province railway information.

/// Neighbor/level pair stride.
pub const STRIDE: usize = 8;
/// Neighbor province ID, i32.
pub const PROVINCE_ID: usize = 0;
/// Railway level, i32; mutation writes the signed maximum/clamped result at +4.
pub const LEVEL: usize = 4;
/// Sorted-vector insertion helper for rail-pair mutation and loading.
pub const SORTED_INSERT_RVA: usize = 0x00e7_b440;
/// Sorted-vector growth helper for rail-pair mutation.
pub const VECTOR_GROW_RVA: usize = 0x00e7_b2b0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub from_province: u32,
    pub to_province: u32,
    /// Exact signed level; no hard-coded vanilla maximum.
    pub level: i32,
}
