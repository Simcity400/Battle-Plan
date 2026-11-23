use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionRow {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
    pub reveal_skip: u8,
    pub theatre: Recorded<Option<u64>>,
}

impl DivisionRow {
    pub const HIT_POINTS_STEP: Step = Step::fixed5(10_000, Rounding::TowardZeroNonzero);
    pub const ORGANISATION_STEP: Step = Step::fixed5(10_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatRow {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub day: i32,
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub attacker_classes: Vec<u32>,
    pub defender_classes: Vec<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyOrderRow {
    pub owner_group: u64,
    pub instance: u32,
    pub country: u32,
    pub order_type: i32,
    pub active: u8,
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
    pub paradrop_target: u32,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionRowAt141 {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
    pub reveal_skip: u8,
}

row_codec!(DivisionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64 @ Self::HIT_POINTS_STEP, 6 => organisation: I64 @ Self::ORGANISATION_STEP, 7 => orders_group: U64, 8 => reveal_skip: U8, 9 => theatre: OptionalU64}, |_row| Ok(()));

row_codec!(LandCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => duration: I32, 2 => day: I32, 3 => attackers: U64List, 4 => defenders: U64List, 5 => attacker_classes: U32List, 6 => defender_classes: U32List}, |_row| Ok(()));

row_codec!(ArmyOrderRow, 96, |row| (row.owner_group as u128) << 32 | row.instance as u128, |key| { owner_group: (key >> 32) as u64, instance: key as u32 }, {0 => country: U32, 1 => order_type: I32, 2 => active: U8, 3 => paradrop_target: U32, 4 => path: U32List, 5 => target_states: U32List}, |_row| Ok(()));

#[cfg(any(test, feature = "test-support"))]
row_codec!(DivisionRowAt141, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => roster_owner: U32, 1 => logical_owner: U32, 2 => expeditionary_owner: U32, 3 => province: U32, 4 => template: U64, 5 => hit_points: I64 @ DivisionRow::HIT_POINTS_STEP, 6 => organisation: I64 @ DivisionRow::ORGANISATION_STEP, 7 => orders_group: U64, 8 => reveal_skip: U8}, |_row| Ok(()));
