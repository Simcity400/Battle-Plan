//! Province railway state. Persistent cooldown state uses the manager's
//! province slot.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029e_f400);

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
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f0_4f70);
/// Railway-level setter updating an existing level or inserting a pair.
pub const SET_RAILWAY_RVA: Rva = Rva(0x00f0_8540);
/// Railway token loader rebuilding pairs.
pub const LOAD_TOKEN_RVA: Rva = Rva(0x00f0_7ed0);
/// Destructor freeing pairs and clearing pointer/count.
pub const DESTRUCTOR_RVA: Rva = Rva(0x00f0_5420);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    /// Native signed tick count, including zero; no display-unit conversion.
    pub cooldown_ticks: i32,
    /// [`BUILDING`] is non-null.
    pub building_bound: bool,
}

/// Whether the info bytes from offset zero hold a non-null [`BUILDING`].
pub fn building_bound(info: &[u8]) -> bool {
    info[BUILDING..BUILDING + 8] != [0; 8]
}
