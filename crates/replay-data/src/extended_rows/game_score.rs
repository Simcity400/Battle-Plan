use super::*;

/// Sites of the strategic-air subsystem's first list in stored order.
/// Repeated bases remain repeated: each occurrence contributes to score.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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
opaque_value!(Vec<GameScoreAirSite>);

/// Every country-fleet occurrence in roster order. Score adds stored
/// signed task-force counts with i32 wrapping; this is not a ship census.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct GameScoreFleetCount {
    pub fleet: u64,
    pub task_forces: Option<i32>,
    pub error: Vec<u8>,
}
value_struct!(GameScoreFleetCount { fleet: u64, task_forces: Option<i32>, error: Vec<u8> });
opaque_value!(Vec<GameScoreFleetCount>);

/// Hourly score operands for every canonical country's raw reference.
/// Army and Land retain stored counts, including stale/reset headers. Navy
/// retains ordered count witnesses above. Air distinguishes an unreadable
/// subsystem, a known null pointer and a present subsystem whose ordered
/// first-site list may be empty. Each failed scalar/list read has its own
/// named diagnostic; zero is known.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

/// Hourly fields for every wing in the shared wing census, without score
/// or access admission filters. Game score joins ordered base registrations.
/// It reads stored plane count and stat 0x33 using the signed mission
/// selector, widened while retaining its signed-byte range. Full wing
/// handles are retained. Each failed operand is independently unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

/// One hourly singleton with six effective signed fixed5 NCountry defines:
/// COUNTRY/ARMY/NAVY/AIR/INDUSTRY/PROVINCE_SCORE_MULTIPLIER. Each is read
/// independently; no defaults are inferred from vanilla content. Part
/// multipliers precede the total multiplier.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn country(country: u32) -> CountryGameScoreInputRow {
        CountryGameScoreInputRow {
            country,
            army_count: Some(0),
            navy_count: Some(0),
            land_count: Some(0),
            air_state: CountryGameScoreInputRow::AIR_ABSENT,
            ..Default::default()
        }
    }

    #[test]
    fn game_score_missing_counts_and_air_access_never_become_zero() {
        let mut row = country(4999);
        let before = row.clone();
        row.army_count = None;
        row.army_error = b"division header unreadable".to_vec();
        row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
        row.first_sites = vec![GameScoreAirSite {
            base: u64::MAX,
            access: GameScoreAirSite::ACCESS_UNKNOWN,
            error: b"air container unreadable".to_vec(),
        }];
        let delta = row.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.apply(Some(before)).unwrap(), row);
        assert!(
            TableRows::CountryGameScoreInputs(vec![row.clone()])
                .validate_keys()
                .is_ok()
        );
        for bad in [
            CountryGameScoreInputRow {
                army_error: Vec::new(),
                ..row.clone()
            },
            CountryGameScoreInputRow {
                army_count: Some(0),
                ..row.clone()
            },
            CountryGameScoreInputRow {
                air_state: 3,
                ..row.clone()
            },
            CountryGameScoreInputRow {
                air_state: CountryGameScoreInputRow::AIR_ABSENT,
                ..row.clone()
            },
        ] {
            assert!(bad.delta(None).is_err());
        }
        row.first_sites[0].access = 3;
        assert!(row.delta(None).is_err());
    }

    #[test]
    fn game_score_fleet_witnesses_keep_repeats_and_native_wrapping() {
        let row = CountryGameScoreInputRow {
            navy_count: Some(-2),
            fleets: vec![
                GameScoreFleetCount {
                    fleet: u64::MAX,
                    task_forces: Some(i32::MAX),
                    error: Vec::new(),
                };
                2
            ],
            ..country(1)
        };
        assert_eq!(
            row.delta(None)
                .unwrap()
                .unwrap()
                .apply::<CountryGameScoreInputRow>(None)
                .unwrap(),
            row
        );
        assert!(
            CountryGameScoreInputRow {
                navy_count: Some(0),
                ..row.clone()
            }
            .delta(None)
            .is_err()
        );
        let mut missing = row;
        missing.fleets[1].task_forces = None;
        missing.fleets[1].error = b"fleet count unreadable".to_vec();
        assert!(missing.delta(None).is_err());
        missing.navy_count = None;
        missing.navy_error = b"fleet count unreadable".to_vec();
        assert!(missing.delta(None).is_ok());
    }

    #[test]
    fn game_score_multiplier_and_wing_reads_keep_independent_unknowns() {
        let wing = AirWingGameScoreStatsRow {
            id: u64::MAX,
            planes: Some(0),
            stat_error: b"mission stat unreadable".to_vec(),
            mission_index: Some(-1),
            ..Default::default()
        };
        assert_eq!(
            wing.delta(None)
                .unwrap()
                .unwrap()
                .apply::<AirWingGameScoreStatsRow>(None)
                .unwrap(),
            wing
        );
        for index in [i32::from(i8::MIN) - 1, i32::from(i8::MAX) + 1] {
            assert!(
                AirWingGameScoreStatsRow {
                    mission_index: Some(index),
                    ..wing.clone()
                }
                .delta(None)
                .is_err()
            );
        }
        assert!(
            AirWingGameScoreStatsRow {
                mission_index: Some(-2),
                ..wing.clone()
            }
            .delta(None)
            .is_ok()
        );
        assert!(
            AirWingGameScoreStatsRow {
                stat_raw: Some(0),
                ..wing
            }
            .delta(None)
            .is_err()
        );
        let multipliers = GameScoreMultiplierRow {
            total_raw: Some(0),
            army_raw: Some(i64::MIN),
            navy_raw: Some(i64::MAX),
            air_raw: Some(0),
            industry_raw: Some(0),
            land_raw: Some(0),
            ..Default::default()
        };
        assert!(multipliers.delta(None).is_ok());
        assert!(
            GameScoreMultiplierRow {
                id: 1,
                ..multipliers.clone()
            }
            .delta(None)
            .is_err()
        );
        assert!(
            GameScoreMultiplierRow {
                land_raw: None,
                ..multipliers
            }
            .delta(None)
            .is_err()
        );
    }
}
