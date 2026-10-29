//! Province railway state. Persistent cooldown state uses the manager's
//! province slot.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029e_f2d0;

/// Object allocation size.
pub const SIZE: usize = 0x70;
/// Province backing-record pointer copied from the resolved province.
pub const PROVINCE_RECORD: usize = 0x08;
/// Building-status pointer into the resolved province, separate from its template.
pub const BUILDING_STATUS: usize = 0x10;
/// Railway-building instance pointer.
pub const BUILDING: usize = 0x18;
/// Neighbour-indexed signed levels sized from the definition neighbour count.
/// Lookup uses the neighbour index, and serialization retains every entry.
pub const LEVELS_BY_NEIGHBOR_INDEX: usize = 0x20;
/// Neighbour-indexed railway-level signed count, checked against the definition.
pub const LEVEL_COUNT: usize = 0x2c;
/// CProvinceRailwayInfo sorted neighbor/level pair data pointer.
pub const NEIGHBOR_PAIRS: usize = 0x38;
/// Signed pair count, sign-extended for sorted search. The nonzero test is
/// independent of pair pointer, capacity and indexed levels.
pub const NEIGHBOR_PAIR_COUNT: usize = 0x44;
/// Pending-construction vector written by railway-level additions.
pub const PENDING_CONSTRUCTION: usize = 0x50;
/// Railway cooldown ticks, native i32.
pub const COOLDOWN_TICKS: usize = 0x68;
/// Province-railway-info constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00f0_4f00;
/// Railway-level setter updating an existing level or inserting a pair.
pub const SET_RAILWAY_RVA: usize = 0x00f0_84d0;
/// Railway token loader rebuilding pairs.
pub const LOAD_TOKEN_RVA: usize = 0x00f0_7e60;
/// Destructor freeing pairs and clearing pointer/count.
pub const DESTRUCTOR_RVA: usize = 0x00f0_53b0;
/// Railway setter entry sentinel bytes.
pub const SET_RAILWAY_SENTINEL: [u8; 16] = [
    0x4c, 0x8b, 0xdc, 0x53, 0x56, 0x57, 0x48, 0x83, 0xec, 0x50, 0x80, 0x3d, 0x30, 0x71, 0x78, 0x02,
];
/// Railway loader entry sentinel bytes.
pub const LOAD_TOKEN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Railway-info destructor entry sentinel bytes.
pub const DESTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    /// Native signed tick count, including zero; no display-unit conversion.
    pub cooldown_ticks: i32,
}
