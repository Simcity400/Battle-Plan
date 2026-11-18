use super::*;

/// Exact per-write daily-history changes.
/// The key is an increasing event ordinal within this recording branch.
/// `game_hour` dates the native application, independently of the hourly drain.
/// Country references resolve through that frame's CountryLinks. `region`
/// resolves through the recording's verified content region inventory.
///
/// `amount_raw` is the signed fixed5 increment at both `family_base` and
/// `family_base + native_cause` (twice in one word for cause zero). Display
/// divisor is 100,000. Saturation is refused. The native writer books requested
/// loss counts, which can exceed actual removals from an exhausted wing pool.
/// `credited_enemy` distinguishes own losses from enemy credit. `opponent`
/// retains the opposite tag from the wrapper, with null for nonpositive tags.
/// `daily_slot` is the authenticated physical daily cursor at the write,
/// including newly allocated histories; save equality uses it across midnight.
/// The engine writer map owns ordinal names and source classification.
/// A complete hourly table certifies zero for country/region pairs with no
/// events. Lost and first/recovery partial intervals remain unavailable.
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

/// Exact hourly daily-update roster predicate.
/// Every canonical country is present; false is distinct from a missing witness.
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

/// Stable source classes; executable caller addresses stay in engine.
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

/// One signed health removal through the shared building-damage writer.
/// Air bombing and naval damage reach this writer; other callers retain
/// their source classification. The key is an increasing event ordinal
/// within this recording branch, never a building handle. `game_hour` dates
/// the application, independently of the later hourly drain. No-wing damage
/// retains a null attacker and its source. A wing's attacker is its effective
/// country, rather than its roster owner. `victim` is the location controller
/// at application, credited by the air wrapper, rather than the country used
/// for repair routing. Zero is the reserved null-country reference: retain
/// such damage with an unknown victim identity, never a guessed country.
/// `province` distinguishes province buildings from state buildings. Type
/// tokens resolve through recorded LexicalTokens and effective content.
/// A province building whose province has no state records state zero as
/// known absence. State-scoped buildings retain their actual state id.
///
/// `amount_raw` is pre-minus-post building health after clamps; a full level
/// is 10,000,000. `credited_raw` is the returned damage budget and can differ
/// under damage modifiers. Negative modifiers can increase health; those
/// gains remain exact negative removals rather than unavailable damage.
/// `native_cause` preserves the source-kind word independently of the stable
/// caller class. This is an hourly event ledger: a complete empty table is
/// zero events, an unavailable interval is unknown. Its first branch interval
/// and every interval after a lost event are unavailable.
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

