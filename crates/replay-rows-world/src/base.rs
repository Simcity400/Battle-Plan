use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ClockRow {
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
    pub speed: i32,
    pub debug_current_ref_id: Option<u32>,
    pub unit: Option<i32>,
    pub order_index: Option<i32>,
    pub front_index: Option<i32>,
    pub theatre_index: Option<i32>,
    pub theater_group_index: Option<i32>,
    pub military_deployment_line_index: Option<i32>,
    pub military_deployment_conveyor_index: Option<i32>,
    pub equipment_variant_index: Option<i32>,
    pub country_leader_index: Option<i32>,
    pub navy_id: Option<i32>,
    pub land_combat_id: Option<i32>,
    pub multiplayer_random_count: Option<i32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CampaignRow {
    pub id: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryRow {
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Vec<u8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
    pub peace_conference_participant: u8,
    pub manpower_residual: i32,
    pub border_land_doctrine_level: Recorded<Option<i32>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryLinkRow {
    pub id: u32,
    pub canonical_slot: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionRise {
    pub hour: i32,
    pub count: u32,
}
replay_rows::value::value_struct!(ConstructionRise { hour: i32, count: u32 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceRow {
    pub id: u32,
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub victory_points: i32,
    pub fort_positive: u8,
    pub move_gate: u8,
    pub port_sea: i32,
    pub attack: Recorded<Witnessed<ProvinceAttackInputs>>,
    pub selected_areas: Recorded<Retired>,
    pub fort_bunker: Recorded<Option<i64>>,
    pub controller_area: Recorded<Option<u32>>,
    pub owner_area: Recorded<Option<u32>>,
    pub area_node: Recorded<Option<ControllerAreaNode>>,
}

impl ProvinceRow {
    pub const MOVE_NULL_OWNER_AREA: u8 = 1;
    pub const MOVE_BORDER_CONFLICT: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateRow {
    pub infrastructure: Recorded<Option<StateInfrastructure>>,
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    pub movement_infrastructure: i64,
    pub manpower_available: i32,
    pub manpower_locked: Recorded<i32>,
    pub custom_name: Vec<u8>,
    pub owner_name: Vec<u8>,
    pub continent: Recorded<Option<u32>>,
    pub railway_infrastructure: Recorded<Option<i64>>,
    pub population: Recorded<i32>,
    pub best_vp_index: Recorded<Option<i32>>,
    pub single_controller: Recorded<Option<u32>>,
    pub category: Recorded<Option<u32>>,
    pub marked_owners: Recorded<Option<Vec<u32>>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateInfrastructure {
    pub building_type: u32,
    pub level: i32,
    pub maximum: i32,
}
replay_rows::value::value_struct!(StateInfrastructure { building_type: u32, level: i32, maximum: i32 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarRelationRow {
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    pub first_was_instigator: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionRow {
    pub id: u64,
    pub name: Vec<u8>,
    pub template_token: Option<u32>,
    pub color_red: u32,
    pub color_green: u32,
    pub color_blue: u32,
    pub color_alpha: u32,
    pub color_overridden: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionMemberRow {
    pub country: u32,
    pub faction: u64,
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PlayerRow {
    pub ordinal: Recorded<u32>,
    pub id: i32,
    pub country: u32,
    pub flags: u8,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HumanControllerRow {
    pub country: u32,
    pub count: i8,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateRowAt141 {
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    pub movement_infrastructure: i64,
    pub custom_name: Vec<u8>,
    pub owner_name: Vec<u8>,
    pub manpower_available: i32,
    pub continent: Recorded<Option<u32>>,
    pub railway_infrastructure: Recorded<Option<i64>>,
    pub population: Recorded<i32>,
    pub category: Recorded<Option<u32>>,
    pub manpower_locked: Recorded<i32>,
    pub best_vp_index: Recorded<Option<i32>>,
    pub single_controller: Recorded<Option<u32>>,
    pub infrastructure: Recorded<Option<StateInfrastructure>>,
}

row_codec!(ClockRow, 0, |_row| 0, |_key| {  }, {0 => hour: I32, 1 => play_time: U64, 2 => play_session_started_at: I64, 3 => seed: I32, 4 => speed: I32, 5 => debug_current_ref_id: OptionalU32, 6 => unit: OptionalI32, 7 => order_index: OptionalI32, 8 => front_index: OptionalI32, 9 => theatre_index: OptionalI32, 10 => theater_group_index: OptionalI32, 11 => military_deployment_line_index: OptionalI32, 12 => military_deployment_conveyor_index: OptionalI32, 13 => equipment_variant_index: OptionalI32, 14 => country_leader_index: OptionalI32, 15 => navy_id: OptionalI32, 16 => land_combat_id: OptionalI32, 17 => multiplayer_random_count: OptionalI32}, |_row| Ok(()));

row_codec!(CampaignRow, 0, |_row| 0, |_key| {  }, {0 => id: Bytes}, |_row| Ok(()));

row_codec!(CountryRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32, 1 => tag: Bytes, 2 => political_power_raw: OptionalI64, 3 => command_power_raw: I64, 4 => stability_raw: I64, 5 => war_support_raw: I64, 6 => convoys_raw: OptionalI64, 7 => peace_conference_participant: U8, 8 => manpower_residual: I32, 9 => border_land_doctrine_level: OptionalI32}, |_row| Ok(()));

row_codec!(CountryLinkRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => canonical_slot: U32}, |_row| Ok(()));

row_codec!(ProvinceRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => state: U32, 1 => owner: U32, 2 => controller: U32, 3 => victory_points: I32, 4 => fort_positive: U8, 5 => move_gate: U8, 6 => port_sea: I32, 7 => attack: Opaque, 8 => selected_areas: Opaque, 9 => fort_bunker: OptionalI64, 10 => controller_area: OptionalU32, 11 => owner_area: OptionalU32, 12 => area_node: Opaque}, |_row| Ok(()));

row_codec!(StateRow, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32, 2 => movement_infrastructure: I64, 3 => custom_name: Bytes, 4 => owner_name: Bytes, 5 => manpower_available: I32, 6 => continent: OptionalU32, 7 => railway_infrastructure: OptionalI64, 8 => population: I32, 9 => category: OptionalU32, 10 => manpower_locked: I32, 11 => best_vp_index: OptionalI32, 12 => single_controller: OptionalU32, 13 => infrastructure: Opaque, 14 => marked_owners: Opaque}, |_row| Ok(()));

row_codec!(WarRelationRow, 64, |row| (row.first_country as u128) << 32 | row.second_country as u128, |key| { first_country: (key >> 32) as u32, second_country: key as u32 }, {0 => start_hour: I32, 1 => end_hour: I32, 2 => cancel: U8, 3 => first_casualties: I64, 4 => second_casualties: I64, 5 => first_unknown_casualties: I64, 6 => second_unknown_casualties: I64, 7 => first_was_instigator: U8}, |_row| Ok(()));

row_codec!(FactionRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => name: Bytes, 1 => template_token: OptionalU32, 2 => color_red: U32, 3 => color_green: U32, 4 => color_blue: U32, 5 => color_alpha: U32, 6 => color_overridden: U8}, |_row| Ok(()));

row_codec!(FactionMemberRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => faction: U64, 1 => ordinal: U32}, |_row| Ok(()));

row_codec!(PlayerRow, 32, |row| row.id as u32 as u128, |key| { id: key as u32 as i32 }, {0 => country: U32, 1 => flags: U8, 2 => user: Bytes, 3 => name: Bytes, 4 => ordinal: U32}, |_row| Ok(()));

row_codec!(HumanControllerRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => count: I8}, |_row| Ok(()));

#[cfg(any(test, feature = "test-support"))]
row_codec!(StateRowAt141, 32, |row| row.id as u128, |key| { id: key as u32 }, {0 => owner: U32, 1 => controller: U32, 2 => movement_infrastructure: I64, 3 => custom_name: Bytes, 4 => owner_name: Bytes, 5 => manpower_available: I32, 6 => continent: OptionalU32, 7 => railway_infrastructure: OptionalI64, 8 => population: I32, 9 => category: OptionalU32, 10 => manpower_locked: I32, 11 => best_vp_index: OptionalI32, 12 => single_controller: OptionalU32, 13 => infrastructure: Opaque}, |_row| Ok(()));

impl OpaqueValue for StateInfrastructure {}

impl OpaqueValue for ConstructionRise {}
