use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FleetRow {
    pub id: u64,
    pub owner: u32,
    pub home_base: u32,
    pub name: Recorded<Option<Vec<u8>>>,
    pub leader: Recorded<Option<u64>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TaskForceRow {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
    pub route_count: i32,
    pub combat_references: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipRow {
    pub name: Recorded<Option<Vec<u8>>>,
    pub naming: Recorded<Option<ShipNamingOperands>>,
    pub required_manpower: Recorded<Option<i32>>,
    pub id: u64,
    pub task_force: u64,
    pub strength: i64,
    pub organisation: i64,
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
    pub acquired_manpower: Recorded<Option<i32>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipNamingOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub override_name: Vec<u8>,
    pub programmatic: u8,
    pub equipment: Option<u64>,
}
replay_rows::value::value_struct!(ShipNamingOperands {
    kind: i32, order: i32, ordered: u8, override_name: Vec<u8>,
    programmatic: u8, equipment: Option<u64>,
});

impl ShipRow {
    pub const EXPERIENCE_STEP: Step = Step::fixed5(1_000, Rounding::TowardZero);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatRow {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: i64,
    pub defender_positioning: i64,
    pub attacker_anti_air: i64,
    pub defender_anti_air: i64,
    pub registry_ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatParticipantRow {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
}

row_codec!(FleetRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => home_base: U32, 2 => name: Opaque, 3 => leader: OptionalU64}, |_row| Ok(()));

row_codec!(TaskForceRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => fleet: U64, 1 => roster_owner: U32, 2 => logical_owner: U32, 3 => expeditionary_owner: U32, 4 => province: U32, 5 => mission: U32, 6 => route_count: I32, 7 => combat_references: I32}, |_row| Ok(()));

row_codec!(ShipRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => task_force: U64, 1 => strength: I64, 2 => organisation: I64, 3 => experience: I64 @ Self::EXPERIENCE_STEP, 4 => category: U32, 5 => sunk_convoys: I32, 6 => class_id: I32, 7 => acquired_manpower: OptionalI32, 8 => required_manpower: OptionalI32, 9 => name: Opaque, 10 => naming: Opaque}, |_row| Ok(()));

row_codec!(NavalCombatRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => province: U32, 1 => attacker_positioning: I64, 2 => defender_positioning: I64, 3 => attacker_anti_air: I64, 4 => defender_anti_air: I64, 5 => registry_ordinal: U32}, |_row| Ok(()));

row_codec!(NavalCombatParticipantRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, {0 => ship: U64, 1 => owner: U32, 2 => state: U32}, |row| if row.side > 1 { Err(invalid("naval combat participant side exceeds one")) } else { Ok(()) });

impl OpaqueValue for ShipNamingOperands {}
