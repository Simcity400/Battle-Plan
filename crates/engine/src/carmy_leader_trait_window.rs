//! CArmyLeaderTraitWindow presentation layout; its hooks are excluded.
//! Only the selected leader's two trees are represented here.

/// Trait-window refresh function.
pub const REFRESH_RVA: usize = 0x016b_dc20;
/// Refresh entry sentinel.
pub const REFRESH_SENTINEL: [u8; 18] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x30, 0x49,
    0x63, 0xf0,
];
/// Selected CUnitLeader reference, u64.
pub const SELECTED_LEADER_REFERENCE: usize = 0x78;
/// First CUnitLeaderTraitTree pointer.
pub const FIRST_TREE: usize = 0x120;
/// Second CUnitLeaderTraitTree pointer.
pub const SECOND_TREE: usize = 0x130;
/// Active/exile row-helper array pointer.
pub const ACTIVE_EXILE_ROWS: usize = 0x100;
/// Active/exile row-helper array count.
pub const ACTIVE_EXILE_ROW_COUNT: usize = 0x10c;
/// Advisor row-helper array pointer.
pub const ADVISOR_ROWS: usize = 0x1090;
/// Advisor row-helper array count.
pub const ADVISOR_ROW_COUNT: usize = 0x109c;
/// Active/exile row helper.
pub const ACTIVE_EXILE_ROW_HELPER_RVA: usize = 0x016b_47d0;
/// Advisor row helper.
pub const ADVISOR_ROW_HELPER_RVA: usize = 0x016b_40a0;
/// Enumerate definitions into both trees in call order.
pub const POPULATE_TREES_RVA: usize = 0x016b_d890;
