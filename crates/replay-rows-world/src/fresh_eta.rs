use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OwnerTransportSpeedRow {
    pub owner: u32,
    pub region: u32,
    pub speed: Option<i64>,
}
typed_codec!(OwnerTransportSpeedRow, 64, |row| (row.owner as u128) << 32 | row.region as u128,
|key| Self { owner: (key >> 32) as u32, region: key as u32, ..Default::default() },
{ 0 => (speed): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MovementStateTermRow {
    pub state: u32,
    pub roster: u32,
    pub owner: u32,
    pub terms: Option<StateMovementTerms>,
}
typed_codec!(MovementStateTermRow, 96, |row| (row.state as u128) << 64 | (row.roster as u128) << 32 | row.owner as u128,
|key| Self { state: (key >> 64) as u32, roster: (key >> 32) as u32, owner: key as u32, ..Default::default() },
{ 0 => (terms): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateMovementTerms {
    pub army_speed: Option<i64>,
    pub attack_speed: Option<i64>,
    pub relation_speed: Option<i64>,
    pub branch: u8,
}
value_struct!(StateMovementTerms {
    army_speed: Option<i64>,
    attack_speed: Option<i64>,
    relation_speed: Option<i64>,
    branch: u8,
});
impl OpaqueValue for StateMovementTerms {}
impl StateMovementTerms {
    pub const NEITHER: u8 = 0;
    pub const FRIENDLY: u8 = 1;
    pub const ENEMY: u8 = 2;
    pub const RELATION_MASK: u8 = 3;
    pub const ATTACK_REFUSED: u8 = 4;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CoreSupplyAccessRow {
    pub owner: u32,
    pub province: u32,
    pub access: Option<bool>,
}
typed_codec!(CoreSupplyAccessRow, 64, |row| (row.owner as u128) << 32 | row.province as u128,
|key| Self { owner: (key >> 32) as u32, province: key as u32, ..Default::default() },
{ 0 => (access): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryUnitSpeedEntryRow {
    pub country: u32,
    pub key: u32,
    pub entry: Option<Option<i64>>,
}
typed_codec!(CountryUnitSpeedEntryRow, 64, |row| (row.country as u128) << 32 | row.key as u128,
|key| Self { country: (key >> 32) as u32, key: key as u32, ..Default::default() },
{ 0 => (entry): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RiverRelationRow {
    pub owner: u32,
    pub controller: u32,
    pub entry: Option<Option<i64>>,
}
typed_codec!(RiverRelationRow, 64, |row| (row.owner as u128) << 32 | row.controller as u128,
|key| Self { owner: (key >> 32) as u32, controller: key as u32, ..Default::default() },
{ 0 => (entry): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayBuildingState {
    pub building: Option<RailwayBuildingHealth>,
}
value_struct!(RailwayBuildingState {
    building: Option<RailwayBuildingHealth>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayBuildingHealth {
    pub level: i16,
    pub healthy: i16,
    pub partial: Option<i64>,
}
value_struct!(RailwayBuildingHealth {
    level: i16,
    healthy: i16,
    partial: Option<i64>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwaySelectionInputs {
    pub levels: Option<Vec<i32>>,
    pub pair_count: Option<i32>,
    pub building_level: Option<Option<i16>>,
}
value_struct!(RailwaySelectionInputs {
    levels: Option<Vec<i32>>,
    pair_count: Option<i32>,
    building_level: Option<Option<i16>>,
});

impl OpaqueValue for RailwayBuildingState {}
impl OpaqueValue for RailwaySelectionInputs {}
