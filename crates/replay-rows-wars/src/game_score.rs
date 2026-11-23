use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GameScoreAirSite {
    pub base: u64,
    pub access: u8,
    pub error: Vec<u8>,
}
impl GameScoreAirSite {
    pub const ACCESS_UNKNOWN: u8 = 0;
    pub const ACCESS_FALSE: u8 = 1;
    pub const ACCESS_TRUE: u8 = 2;
}
value_struct!(GameScoreAirSite { base: u64, access: u8, error: Vec<u8> });
impl OpaqueValue for GameScoreAirSite {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GameScoreFleetCount {
    pub fleet: u64,
    pub task_forces: Option<i32>,
    pub error: Vec<u8>,
}
value_struct!(GameScoreFleetCount { fleet: u64, task_forces: Option<i32>, error: Vec<u8> });
impl OpaqueValue for GameScoreFleetCount {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryGameScoreInputRow {
    pub country: u32,
    pub army_count: Option<i32>,
    pub army_error: Vec<u8>,
    pub navy_count: Option<i32>,
    pub navy_error: Vec<u8>,
    pub land_count: Option<i32>,
    pub land_error: Vec<u8>,
    pub air_state: u8,
    pub air_error: Vec<u8>,
    pub first_sites: Vec<GameScoreAirSite>,
    pub fleets: Vec<GameScoreFleetCount>,
}
impl CountryGameScoreInputRow {
    pub const AIR_UNKNOWN: u8 = 0;
    pub const AIR_ABSENT: u8 = 1;
    pub const AIR_PRESENT: u8 = 2;

    fn check(&self) -> io::Result<()> {
        check_read(self.army_count.is_some(), &self.army_error)?;
        check_read(self.navy_count.is_some(), &self.navy_error)?;
        check_read(self.land_count.is_some(), &self.land_error)?;
        if self.air_state > Self::AIR_PRESENT {
            return Err(invalid("unsupported game score air state"));
        }
        check_read(self.air_state != Self::AIR_UNKNOWN, &self.air_error)?;
        if self.air_state != Self::AIR_PRESENT && !self.first_sites.is_empty() {
            return Err(invalid("game score air sites require a present subsystem"));
        }
        for site in &self.first_sites {
            if site.access > GameScoreAirSite::ACCESS_TRUE {
                return Err(invalid("unsupported game score air access"));
            }
            check_read(site.access != GameScoreAirSite::ACCESS_UNKNOWN, &site.error)?;
        }
        let mut sum = Some(0_i32);
        for fleet in &self.fleets {
            check_read(fleet.task_forces.is_some(), &fleet.error)?;
            sum = sum
                .zip(fleet.task_forces)
                .map(|(sum, count)| sum.wrapping_add(count));
        }
        if self.navy_count.is_some() && self.navy_count != sum {
            return Err(invalid(
                "game score navy count differs from its fleet witnesses",
            ));
        }
        Ok(())
    }
}
row_codec!(CountryGameScoreInputRow, 32, |row| row.country as u128, |key| { country: key as u32 },
{ 0 => army_count: OptionalI32, 1 => army_error: Bytes, 2 => navy_count: OptionalI32,
  3 => navy_error: Bytes, 4 => land_count: OptionalI32, 5 => land_error: Bytes,
  6 => air_state: U8, 7 => air_error: Bytes, 8 => first_sites: Opaque, 9 => fleets: Opaque },
|row| row.check());

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirWingGameScoreStatsRow {
    pub id: u64,
    pub planes: Option<i32>,
    pub planes_error: Vec<u8>,
    pub stat_raw: Option<i64>,
    pub stat_error: Vec<u8>,
    pub mission_index: Option<i32>,
    pub mission_error: Vec<u8>,
}
row_codec!(AirWingGameScoreStatsRow, 64, |row| row.id as u128, |key| { id: key as u64 },
{ 0 => planes: OptionalI32, 1 => planes_error: Bytes, 2 => stat_raw: OptionalI64,
  3 => stat_error: Bytes, 4 => mission_index: OptionalI32, 5 => mission_error: Bytes },
|row| {
    check_read(row.planes.is_some(), &row.planes_error)?;
    check_read(row.stat_raw.is_some(), &row.stat_error)?;
    check_read(row.mission_index.is_some(), &row.mission_error)?;
    if row.mission_index.is_some_and(|index| i8::try_from(index).is_err()) {
        return Err(invalid("game score mission index is outside its native signed byte"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GameScoreMultiplierRow {
    pub id: u32,
    pub total_raw: Option<i64>,
    pub total_error: Vec<u8>,
    pub army_raw: Option<i64>,
    pub army_error: Vec<u8>,
    pub navy_raw: Option<i64>,
    pub navy_error: Vec<u8>,
    pub air_raw: Option<i64>,
    pub air_error: Vec<u8>,
    pub industry_raw: Option<i64>,
    pub industry_error: Vec<u8>,
    pub land_raw: Option<i64>,
    pub land_error: Vec<u8>,
}
row_codec!(GameScoreMultiplierRow, 0, |_row| 0, |_key| { id: 0 },
{ 0 => total_raw: OptionalI64, 1 => total_error: Bytes, 2 => army_raw: OptionalI64,
  3 => army_error: Bytes, 4 => navy_raw: OptionalI64, 5 => navy_error: Bytes,
  6 => air_raw: OptionalI64, 7 => air_error: Bytes, 8 => industry_raw: OptionalI64,
  9 => industry_error: Bytes, 10 => land_raw: OptionalI64, 11 => land_error: Bytes },
|row| {
    if row.id != 0 { return Err(invalid("game score multiplier singleton id must be zero")); }
    for (value, error) in [(row.total_raw, &row.total_error), (row.army_raw, &row.army_error),
        (row.navy_raw, &row.navy_error), (row.air_raw, &row.air_error),
        (row.industry_raw, &row.industry_error), (row.land_raw, &row.land_error)] {
        check_read(value.is_some(), error)?;
    }
    Ok(())
});

fn check_read(known: bool, error: &[u8]) -> io::Result<()> {
    if known != error.is_empty() {
        Err(invalid(
            "game score read must have either a value or a named error",
        ))
    } else {
        Ok(())
    }
}
