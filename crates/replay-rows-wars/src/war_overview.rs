use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarOverviewCountryRow {
    pub country: u32,
    pub major: u8,
    pub divisions: i32,
    pub factories: i32,
    pub view_subject: u8,
    pub score_available: u8,
}
typed_codec!(WarOverviewCountryRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (major): U8, 1 => (divisions): I32, 2 => (factories): I32, 3 => (view_subject): U8, 4 => (score_available): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarNameRuleRow {
    pub index: u32,
    pub prefix: Vec<u8>,
    pub viewpoint: u32,
}
typed_codec!(WarNameRuleRow, 32, |row| row.index as u128, |key| Self { index: key as u32, ..Default::default() },
 { 0 => (prefix): Bytes, 1 => (viewpoint): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarParticipationRow {
    pub war_first: u32,
    pub war_second: u32,
    pub direction: u32,
    pub first_country: u32,
    pub second_country: u32,
    pub equipment_damage: i64,
    pub province_capture: i64,
    pub air_damage: i64,
    pub strategic_bombing: i64,
    pub sunk_ships: i64,
    pub sunk_convoys: i64,
    pub casualties: i64,
    pub unknown_casualties: i64,
    pub lend_lease_sent: i64,
    pub lend_lease_received: i64,
}
typed_codec!(WarParticipationRow, 96, |row| (row.war_first as u128) << 64 | (row.war_second as u128) << 32 | row.direction as u128, |key| Self { war_first: (key >> 64) as u32, war_second: (key >> 32) as u32, direction: key as u32, ..Default::default() },
 { 0 => (first_country): U32, 1 => (second_country): U32,
   2 => (equipment_damage): I64, 3 => (province_capture): I64, 4 => (air_damage): I64,
   5 => (strategic_bombing): I64, 6 => (sunk_ships): I64, 7 => (sunk_convoys): I64,
   8 => (casualties): I64, 9 => (unknown_casualties): I64,
   10 => (lend_lease_sent): I64, 11 => (lend_lease_received): I64 },
 |row| if row.direction > 1 { Err(invalid("war participation direction outside 0/1")) } else { Ok(()) });

impl WarParticipationRow {
    pub fn scores(&self) -> game_model::war_participation::Scores {
        game_model::war_participation::Scores {
            first: self.first_country,
            second: self.second_country,
            equipment_damage: self.equipment_damage,
            province_capture: self.province_capture,
            air_damage: self.air_damage,
            strategic_bombing: self.strategic_bombing,
            sunk_ships: self.sunk_ships,
            sunk_convoys: self.sunk_convoys,
            casualties: self.casualties,
            unknown_casualties: self.unknown_casualties,
            lend_lease_sent: self.lend_lease_sent,
            lend_lease_received: self.lend_lease_received,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarCapturedProvinceRow {
    pub war_first: u32,
    pub war_second: u32,
    pub direction: u32,
    pub province: i32,
    pub credited: u8,
}
typed_codec!(WarCapturedProvinceRow, 128, |row| (row.war_first as u128) << 96 | (row.war_second as u128) << 64 | (row.direction as u128) << 32 | row.province as u32 as u128,
 |key| Self { war_first: (key >> 96) as u32, war_second: (key >> 64) as u32, direction: (key >> 32) as u32, province: key as u32 as i32, ..Default::default() },
 { 0 => (credited): U8 }, |row| if row.direction > 1 || row.credited != 1 { Err(invalid("invalid war captured-province membership")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarLendLeaseHistoryRow {
    pub country: u32,
    pub target: u32,
    pub industry_sent: i64,
    pub fuel_sent: i64,
    pub industry_received: i64,
    pub fuel_received: i64,
}
typed_codec!(WarLendLeaseHistoryRow, 64, |row| (row.country as u128) << 32 | row.target as u128,
 |key| Self { country: (key >> 32) as u32, target: key as u32, ..Default::default() },
 { 0 => (industry_sent): I64, 1 => (fuel_sent): I64, 2 => (industry_received): I64, 3 => (fuel_received): I64 }, |row| if [row.industry_sent,row.fuel_sent,row.industry_received,row.fuel_received].iter().all(|&v|v==0) { Err(invalid("war delivery rows retain only nonzero histories")) } else { Ok(()) });
