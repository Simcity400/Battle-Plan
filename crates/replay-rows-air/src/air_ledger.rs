use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirLossEventRow {
    pub id: u64,
    pub game_hour: i32,
    pub country: u32,
    pub region: u32,
    pub native_cause: u32,
    pub family_base: u32,
    pub credited_enemy: bool,
    pub amount_raw: i64,
    pub opponent: Option<u32>,
    pub daily_slot: u32,
}

typed_codec!(AirLossEventRow, 64,
|row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() },
{ 0 => (game_hour): I32, 1 => (country): U32, 2 => (region): U32,
  3 => (native_cause): U32, 4 => (family_base): U32,
  5 => (credited_enemy): Bool, 6 => (amount_raw): I64, 7 => (opponent): OptionalU32,
  8 => (daily_slot): U32 }, |row| {
    if !engine_reference_values::native_values::air_loss::FAMILY_BASES.contains(&row.family_base)
        || row.family_base.checked_add(row.native_cause).is_none_or(|index| index as usize >= engine_reference_values::native_values::air_history::FIELD_COUNT as usize)
        || row.amount_raw == 0
        || row.daily_slot >= engine_reference_values::native_values::air_history::DAILY_WINDOW_CAPACITY
    {
        return Err(invalid("invalid air loss family, cause, amount or daily slot"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirDailyRosterRow {
    pub country: u32,
    pub eligible: bool,
}
typed_codec!(AirDailyRosterRow, 32,
|row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (eligible): Bool }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum BuildingBombingDamageCause {
    #[default]
    Unknown = 0,
    AirMission = 1,
    AirNaval = 2,
    LandCombat = 3,
    Scripted = 4,
    Weekly = 5,
    ProgramTeardown = 6,
    AbortDismantle = 7,
}

impl BuildingBombingDamageCause {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::AirMission => "air_mission",
            Self::AirNaval => "air_naval",
            Self::LandCombat => "land_combat",
            Self::Scripted => "scripted",
            Self::Weekly => "weekly",
            Self::ProgramTeardown => "program_teardown",
            Self::AbortDismantle => "abort_dismantle",
        }
    }
}

impl TryFrom<u8> for BuildingBombingDamageCause {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Ok(match value {
            0 => Self::Unknown,
            1 => Self::AirMission,
            2 => Self::AirNaval,
            3 => Self::LandCombat,
            4 => Self::Scripted,
            5 => Self::Weekly,
            6 => Self::ProgramTeardown,
            7 => Self::AbortDismantle,
            _ => return Err(invalid("unsupported building bombing damage cause")),
        })
    }
}

impl From<game_model::building_damage::Cause> for BuildingBombingDamageCause {
    fn from(cause: game_model::building_damage::Cause) -> Self {
        use game_model::building_damage::Cause;
        match cause {
            Cause::Unknown => Self::Unknown,
            Cause::AirMission => Self::AirMission,
            Cause::AirNaval => Self::AirNaval,
            Cause::LandCombat => Self::LandCombat,
            Cause::Scripted => Self::Scripted,
            Cause::Weekly => Self::Weekly,
            Cause::ProgramTeardown => Self::ProgramTeardown,
            Cause::AbortDismantle => Self::AbortDismantle,
        }
    }
}

impl NativeValue for BuildingBombingDamageCause {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        let Field::U8(value) = field else {
            return Err(invalid("field type differs from schema"));
        };
        Self::try_from(value)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BuildingBombingDamageRow {
    pub id: u64,
    pub game_hour: i32,
    pub attacker: Option<u32>,
    pub victim: u32,
    pub state: u32,
    pub province: Option<u32>,
    pub building_type: u32,
    pub native_cause: u32,
    pub cause: BuildingBombingDamageCause,
    pub amount_raw: i64,
    pub credited_raw: i64,
}

typed_codec!(BuildingBombingDamageRow, 64, |row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() },
{ 0 => (game_hour): I32, 1 => (attacker): OptionalU32, 2 => (victim): U32,
  3 => (state): U32, 4 => (province): OptionalU32, 5 => (building_type): U32,
  6 => (native_cause): U32, 7 => (cause): U8, 8 => (amount_raw): I64,
  9 => (credited_raw): I64 }, |row| {
    if row.amount_raw == 0 { Err(invalid("building bombing damage must be nonzero")) }
    else { Ok(()) }
});

replay_rows::native!(BuildingBombingDamageCause);
