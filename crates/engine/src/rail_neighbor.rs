//! Anonymous sorted pair in CProvinceRailwayInfo; no distinct RTTI name proven.

/// Neighbor/level pair stride; old attach_log.rs d81_walk_provinces iterates chunks_exact(8).
pub const STRIDE: usize = 8;
/// Neighbor province id, native i32; old attach_log.rs:145901-145940 decodes the first four bytes.
pub const PROVINCE_ID: usize = 0;
/// Railway level, native i32; SetRailway uses signed max/clamp and writes the resulting int at pair+4.
pub const LEVEL: usize = 4;
/// Generic sorted-vector insertion used by SetRailway and LoadToken; old d81_rail_change_gate/hooks_windows.rs:67-70 census.
pub const SORTED_INSERT_RVA: usize = 0x00e7_b440;
/// Generic sorted-vector growth helper used by rail pair mutation; old d81_rail_change_gate/hooks_windows.rs:67-70 census.
pub const VECTOR_GROW_RVA: usize = 0x00e7_b2b0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub from_province: u32,
    pub to_province: u32,
    /// Exact native signed level; no hard-coded vanilla maximum.
    pub level: i32,
}
