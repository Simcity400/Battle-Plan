//! Recorded native war-overview operands.

use super::*;

/// Hourly owned war occurrences, including retired ones, in diplomacy vector order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarOwnedRelationRow {
    pub owner: u32,
    pub ordinal: u32,
    pub first_country: u32,
    pub second_country: u32,
    pub retired: u8,
    pub first_goal_count: u32,
    pub second_goal_count: u32,
}
typed_codec!(WarOwnedRelationRow, 64, |row| (row.owner as u128) << 32 | row.ordinal as u128, |key| Self { owner: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (first_country): U32, 1 => (second_country): U32, 2 => (retired): U8, 3 => (first_goal_count): U32, 4 => (second_goal_count): U32 }, |_row| Ok(()));

/// Hourly ordered goal handles and their naming operands; a null/stale handle keeps resolved=0.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarRelationGoalRow {
    pub owner: u32,
    pub relation_ordinal: u32,
    pub side: u32,
    pub ordinal: u32,
    pub handle: u64,
    pub resolved: u8,
    pub type_token: u32,
    pub actor: u32,
    pub recipient: u32,
    pub state_count: u32,
    pub first_state: u32,
    pub country_count: u32,
    pub first_country: u32,
}
typed_codec!(WarRelationGoalRow, 128, |row| (row.owner as u128) << 96 | (row.relation_ordinal as u128) << 64 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { owner: (key >> 96) as u32, relation_ordinal: (key >> 64) as u32, side: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (handle): U64, 1 => (resolved): U8, 2 => (type_token): U32, 3 => (actor): U32, 4 => (recipient): U32, 5 => (state_count): U32, 6 => (first_state): U32, 7 => (country_count): U32, 8 => (first_country): U32 }, |_row| Ok(()));

/// Hourly representative score inputs and the cached subject predicate toward LocalPlayer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarOverviewCountryRow {
    pub country: u32,
    pub major: u8,
    pub divisions: i32,
    pub factories: i32,
    pub view_subject: u8,
    /// Only 1 makes divisions, factories and view_subject available.
    pub score_available: u8,
}
typed_codec!(WarOverviewCountryRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (major): U8, 1 => (divisions): I32, 2 => (factories): I32, 3 => (view_subject): U8, 4 => (score_available): U8 }, |_row| Ok(()));

/// The recorded default naming prefix, read from the running game; not a content key catalogue.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WarNameRuleRow {
    pub index: u32,
    pub prefix: Vec<u8>,
    /// Actual playing/observer country used by this hour's walk.
    pub viewpoint: u32,
}
typed_codec!(WarNameRuleRow, 32, |row| row.index as u128, |key| Self { index: key as u32, ..Default::default() },
 { 0 => (prefix): Bytes, 1 => (viewpoint): U32 }, |_row| Ok(()));
