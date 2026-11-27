use replay_data::rows::{AirBaseRow, AirMissionInputRow, AirWingRow, ClockState, CountryCounterInputRow, TaskForceRow};
use super::{AceRangeInputRow, BTreeMap, DecodedFrame, Fact, Hour, OnceLock, ProvinceKind, ResolvedLayers, data, io, tables};
#[cfg(test)]
use super::{NativeInputs, RaidAirScoreRow};
use air_range::Sources;
use viewer_unit_reads::{air_detection, air_range};

const ONE: i64 = 100_000;
const INVALID: i64 = 0xffff_ffff;
const HOUR_EPOCH: i32 = 43_800_000;
const NO_HOME_REGION: u32 = 170;

pub struct Defines {
    wing_size: i32,
    mission_factors: Vec<i64>,
    detection: i64,
    maximum_attack: i64,
    maximum_defence: i64,
    maximum_speed: i64,
    disruption_defence: i64,
    disruption_attack: i64,
    disruption_speed: i64,
    night_threshold: i64,
    day_night: replay_data::day_night::Defines,
    ace: air_range::Defines,
    night: [Fact<i64>; 2],
}

impl Default for Defines {
    fn default() -> Self {
        Self {
            wing_size: 0,
            mission_factors: Vec::new(),
            detection: 0,
            maximum_attack: 0,
            maximum_defence: 0,
            maximum_speed: 0,
            disruption_defence: 0,
            disruption_attack: 0,
            disruption_speed: 0,
            night_threshold: 0,
            day_night: Default::default(),
            ace: air_range::Defines {
                map_scale: 0,
                ace_wing_size: 0,
                ace_wing_size_max_bonus: 0,
            },
            night: [Ok(0), Ok(0)],
        }
    }
}

impl Defines {
    pub fn read(
        defines: &content_reader_defines::defines::Defines,
        layers: &ResolvedLayers,
    ) -> io::Result<Self> {
        let requests = ["air_attack_factor", "air_defence_factor"].map(|key| {
            content_reader_politics::static_modifiers::Request {
                modifier: "night",
                key,
            }
        });
        let night = content_reader_politics::static_modifiers::read(layers, &requests)?.values;
        let night: [Fact<i64>; 2] = night
            .into_iter()
            .map(|value| value.map_err(|why| format!("night {}: {}", why.key, why.reason)))
            .collect::<Vec<_>>()
            .try_into()
            .expect("two requested night keys");
        Ok(Self {
            wing_size: defines.int32("NNavy", "AIR_SPOTTER_NORMALIZED_AIRWING_SIZE")?,
            mission_factors: defines.fixed5_array("NNavy", "AIR_MISSION_SPOTTING_FACTORS")?,
            detection: defines.fixed5("NNavy", "AIR_SPOTTER_DETECTION_FACTOR")?,
            maximum_attack: defines.fixed5("NAir", "AIR_WING_MAX_STATS_ATTACK")?,
            maximum_defence: defines.fixed5("NAir", "AIR_WING_MAX_STATS_DEFENCE")?,
            maximum_speed: defines.fixed5("NAir", "AIR_WING_MAX_STATS_SPEED")?,
            disruption_defence: defines.fixed5("NAir", "DISRUPTION_DEFENCE_DEFENCE_FACTOR")?,
            disruption_attack: defines.fixed5("NAir", "DISRUPTION_DEFENCE_ATTACK_FACTOR")?,
            disruption_speed: defines.fixed5("NAir", "DISRUPTION_DEFENCE_SPEED_FACTOR")?,
            night_threshold: defines.fixed5("NAir", "DAY_NIGHT_COVERAGE_FACTOR")?,
            day_night: replay_data::day_night::Defines::read(
                |section, key| defines.int32(section, key),
                |section, key| defines.fixed5(section, key),
            )?,
            ace: air_range::Defines {
                map_scale: 0,
                ace_wing_size: defines.fixed5("NAir", "ACE_WING_SIZE")?,
                ace_wing_size_max_bonus: defines.fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")?,
            },
            night,
        })
    }
}

pub struct Prepared<'a> {
    frame: &'a DecodedFrame,
    regions: BTreeMap<u32, Vec<&'a AirWingRow>>,
    countries: OnceLock<Fact<air_detection::Countries<'a>>>,
    country_rows: OnceLock<Fact<BTreeMap<u32, &'a CountryCounterInputRow>>>,
    inputs: OnceLock<Fact<BTreeMap<u64, &'a AirMissionInputRow>>>,
    sources: air_range::FrameSources<'a>,
}

impl<'a> Prepared<'a> {
    fn read(hour: &Hour<'a>) -> Fact<Self> {
        let wings: BTreeMap<_, _> = data::<tables::AirWings>(hour.frame)?
            .iter()
            .map(|row| (row.id, row))
            .collect();
        let mut regions = BTreeMap::<_, Vec<_>>::new();
        for registration in data::<tables::AirRegionalMissions>(hour.frame)? {
            let wing = *wings
                .get(&registration.wing)
                .ok_or("Air patrol registration has no wing")?;
            if hour.links.get(&wing.owner) == Some(&hour.ours) {
                let region = wing
                    .mission_region
                    .ok_or("Air patrol registration has no region")?;
                regions
                    .entry(region)
                    .or_default()
                    .push((registration.ordinal, wing));
            }
        }
        let regions = regions
            .into_iter()
            .map(|(region, mut rows)| {
                rows.sort_by_key(|&(ordinal, _)| ordinal);
                if rows
                    .iter()
                    .enumerate()
                    .any(|(index, &(ordinal, _))| index != ordinal as usize)
                {
                    return Err("Air patrol regional registration order is incomplete".to_owned());
                }
                Ok((region, rows.into_iter().map(|(_, wing)| wing).collect()))
            })
            .collect::<Fact<_>>()?;
        Ok(Self {
            frame: hour.frame,
            regions,
            countries: OnceLock::new(),
            country_rows: OnceLock::new(),
            inputs: OnceLock::new(),
            sources: air_range::FrameSources::new(hour.frame),
        })
    }

    fn countries(&self) -> Fact<&air_detection::Countries<'a>> {
        self.countries
            .get_or_init(|| air_detection::Countries::read(self.frame))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn country(&self, slot: u32) -> Fact<&'a CountryCounterInputRow> {
        self.country_rows
            .get_or_init(|| {
                data::<tables::CountryCounterInputs>(self.frame)?
                    .iter()
                    .map(|row| {
                        Ok((
                            self.sources
                                .slot(row.country)
                                .ok_or("Air patrol counter country is unresolved")?,
                            row,
                        ))
                    })
                    .collect()
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&slot)
            .copied()
            .ok_or_else(|| "Air patrol effective country has no counter accumulator".into())
    }

    fn input(&self, wing: u64) -> Fact<&'a AirMissionInputRow> {
        self.inputs
            .get_or_init(|| {
                Ok(data::<tables::AirMissionInputs>(self.frame)?
                    .iter()
                    .map(|row| (row.wing, row))
                    .collect())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&wing)
            .copied()
            .ok_or_else(|| "Air patrol mission operands are absent".into())
    }

    fn country_term(&self, slot: u32, key: u32, direct: bool) -> Fact<i64> {
        let raw = self
            .country(slot)?
            .air_combat_modifier(key)
            .map_err(str::to_owned)?;
        if direct {
            Ok(raw.unwrap_or(0))
        } else {
            self.countries()?.effective(key, raw)
        }
    }

    fn ace(&self, wing: &AirWingRow) -> Fact<Option<&AceRangeInputRow>> {
        if wing.ace == 0 {
            return Ok(None);
        }
        Ok(self.sources.ace(wing.ace)?.filter(|ace| ace.alive != 0))
    }

    fn base(&self, wing: &AirWingRow) -> Fact<Option<AirBaseRow>> {
        if wing.air_base == 0 {
            return Ok(None);
        }
        let rows = data::<tables::AirBases>(self.frame)?;
        if let Ok(at) = rows.binary_search_by_key(&wing.air_base, |row| row.id) {
            let row = &rows[at];
            return Ok(Some(AirBaseRow {
                id: row.id,
                kind: row.kind,
                state: row.state,
                province: row.province,
                host: row.host,
                ..Default::default()
            }));
        }
        let Some(witness) = data::<tables::AirReferenceWitnesses>(self.frame)?
            .iter()
            .find(|row| row.handle == wing.air_base)
        else {
            return Ok(None);
        };
        Ok(Some(AirBaseRow {
            id: witness.handle,
            kind: witness
                .base_kind
                .ok_or("Air patrol base subtype is unavailable")?,
            state: witness.state,
            province: witness.province,
            host: witness.host.ok_or("Air patrol base host is unavailable")?,
            ..Default::default()
        }))
    }

    fn carrier_force(&self, base: &AirBaseRow) -> Fact<Option<&TaskForceRow>> {
        if base.host == 0 {
            return Ok(None);
        }
        let ships = data::<tables::Ships>(self.frame)?;
        let Some(ship) = ships.iter().find(|row| row.id == base.host) else {
            return Ok(None);
        };
        let forces = data::<tables::TaskForces>(self.frame)?;
        forces
            .iter()
            .find(|row| row.id == ship.task_force)
            .map(Some)
            .ok_or_else(|| "Air patrol carrier has no task force".into())
    }

    fn home_region(&self, hour: &Hour<'_>, wing: &AirWingRow) -> Fact<u32> {
        let Some(base) = self.base(wing)? else {
            return Ok(NO_HOME_REGION);
        };
        let province = if let Some(state) = base.state {
            if let Some(province) = base.province {
                province
            } else {
                let kind = super::super::bases::site_kind(base.kind);
                *hour
                    .anchors
                    .air_base_first_provinces
                    .get(&(state, kind))
                    .ok_or("Air patrol base has no content province list")?
                    .as_ref()
                    .map_err(|why| why.to_string())?
            }
        } else {
            let Some(force) = self.carrier_force(&base)? else {
                return Ok(NO_HOME_REGION);
            };
            let at = data::<tables::Provinces>(self.frame)?
                .iter()
                .find(|row| row.id == force.province)
                .ok_or("Air patrol carrier province is absent")?;
            if hour.anchors.kinds.get(&force.province) == Some(&ProvinceKind::Land) {
                at.port_sea as u32
            } else {
                force.province
            }
        };
        hour.anchors
            .region_of
            .get(&province)
            .copied()
            .ok_or_else(|| "Air patrol base province has no strategic region".into())
    }

    fn normalized_combat(
        &self,
        hour: &Hour<'_>,
        region: u32,
        province: u32,
        wing: &AirWingRow,
        input: &AirMissionInputRow,
        mask: u32,
    ) -> Fact<(i64, i64)> {
        let defines = &hour.content.patrol_air;
        let effective = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let slot = self
            .sources
            .slot(effective)
            .ok_or("Air patrol effective country is unresolved")?;
        let mut factors = [
            ONE.wrapping_add(required(
                *input
                    .patrol_wing_attack_raw
                    .require("AirMissionInputs.patrol_wing_attack_raw")?,
                "wing combat attack modifier",
            )?)
            .wrapping_add(self.country_term(slot, 0x6f, true)?),
            ONE.wrapping_add(required(
                *input
                    .patrol_wing_defence_raw
                    .require("AirMissionInputs.patrol_wing_defence_raw")?,
                "wing combat defence modifier",
            )?)
            .wrapping_add(self.country_term(slot, 0x70, true)?),
        ];
        if let Some(ace) = self.ace(wing)? {
            if ace.combat_modifiers_known != replay_data::rows::Recorded::Value(true) {
                return Err("Air patrol living ace combat accumulator is unavailable".into());
            }
            let scale = air_range::ace_scale(ace, defines.ace, &self.sources)?;
            factors[0] = factors[0].wrapping_add(multiply(
                (*ace.attack_modifier_raw.require("attack_modifier_raw")?).unwrap_or(0),
                scale,
            ));
            factors[1] = factors[1].wrapping_add(multiply(
                (*ace.defence_modifier_raw.require("defence_modifier_raw")?).unwrap_or(0),
                scale,
            ));
            let positive = [ace.type_attack_positive(), ace.type_defence_positive()]
                .map(|value| value.map_err(str::to_owned));
            for (factor, positive) in factors.iter_mut().zip(positive) {
                if positive? {
                    *factor = factor.wrapping_add(
                        self.sources
                            .command_power(slot)?
                            .air_ace_bonuses_factor
                            .ok_or("Air patrol ace country bonus is unavailable")?,
                    );
                }
            }
        }
        let bonus_key = if mask & 4 != 0 {
            Some(0xd)
        } else if mask & 1 != 0 {
            Some(0xb)
        } else if mask & 2 != 0 {
            Some(0xf)
        } else if mask & 0x110 != 0 {
            Some(0x10)
        } else {
            None
        };
        let mut mission_bonus = 0;
        if wing.mission_region.is_some() && province > 0 {
            if let Some(key) = bonus_key {
                mission_bonus = self.country_term(slot, key, false)?;
            }
            if mask == 8 {
                factors[1] = factors[1].wrapping_add(self.country_term(slot, 0x76, false)?);
            }
            let normalization = input
                .normalization
                .require("AirMissionInputs.normalization")?
                .as_ref()
                .ok_or("Air patrol normalization is unavailable")?;
            let weather = air_detection::wing_weather_normalization(
                wing,
                normalization,
                defines.ace,
                self.countries()?,
                &self.sources,
            )?;
            let weather_row = data::<tables::WeatherRegions>(self.frame)?
                .iter()
                .find(|row| row.id == region)
                .ok_or("Air patrol regional weather is absent")?;
            if weather_row.map_modifiers_known != replay_data::rows::Recorded::Value(true) {
                return Err("Air patrol regional weather accumulator is unavailable".into());
            }
            for (index, raw) in [
                *weather_row.air_attack.require("air_attack")?,
                *weather_row.air_defence.require("air_defence")?,
            ]
            .into_iter()
            .enumerate()
            {
                factors[index] = factors[index].wrapping_add(multiply(
                    self.countries()?.effective([0x6f, 0x70][index], raw)?,
                    weather,
                ));
            }
            let (clock, updated) = province_clock(hour, province)?;
            if clock.night_factor > defines.night_threshold {
                let carrier_mission = match self.base(wing)? {
                    Some(base) => self
                        .carrier_force(&base)?
                        .is_some_and(|force| force.mission != 0),
                    None => false,
                };
                let night = air_detection::wing_night_normalization(
                    wing,
                    normalization,
                    carrier_mission,
                    defines.ace,
                    self.countries()?,
                    &self.sources,
                )?;
                for index in 0..2 {
                    let applied = if updated {
                        Some(clock.night_factor)
                    } else {
                        *input
                            .patrol_night_applied_raw
                            .require("AirMissionInputs.patrol_night_applied_raw")?
                    };
                    let raw = night_raw(defines, applied, index)?;
                    if raw == 0 {
                        continue;
                    }
                    let key = [0x6f, 0x70][index];
                    let definition = hour
                        .definitions
                        .as_ref()
                        .map_err(Clone::clone)?
                        .get(&key)
                        .ok_or("Air patrol night modifier definition is absent")?;
                    if definition.category_mask == 0 {
                        continue;
                    }
                    factors[index] = factors[index].wrapping_add(multiply(
                        self.countries()?.effective(key, Some(raw))?,
                        night,
                    ));
                }
            }
            let home = self.home_region(hour, wing)?;
            if region != 0 && home != 0 {
                let term =
                    self.country_term(slot, if region == home { 0x74 } else { 0x75 }, false)?;
                factors[0] = factors[0].wrapping_add(term);
                factors[1] = factors[1].wrapping_add(term);
            }
        }
        Ok((
            fraction(
                multiply(
                    factors[0].wrapping_add(mission_bonus),
                    required(
                        *input
                            .patrol_attack_raw
                            .require("AirMissionInputs.patrol_attack_raw")?,
                        "cached attack",
                    )?,
                ),
                defines.maximum_attack,
            ),
            fraction(
                multiply(
                    factors[1].wrapping_add(mission_bonus),
                    required(
                        *input
                            .patrol_defence_raw
                            .require("AirMissionInputs.patrol_defence_raw")?,
                        "cached defence",
                    )?,
                ),
                defines.maximum_defence,
            ),
        ))
    }

    fn speed(&self, defines: &Defines, wing: &AirWingRow, input: &AirMissionInputRow) -> Fact<i64> {
        let mut factor = ONE;
        if let Some(ace) = self.ace(wing)? {
            if ace.combat_modifiers_known != replay_data::rows::Recorded::Value(true) {
                return Err("Air patrol living ace speed accumulator is unavailable".into());
            }
            let raw = self
                .countries()?
                .effective(2, *ace.speed_modifier_raw.require("speed_modifier_raw")?)?;
            let scale = air_range::ace_scale(ace, defines.ace, &self.sources)?;
            let effectiveness = self
                .sources
                .command_power(ace.canonical_slot)?
                .ace_effectiveness_factor
                .ok_or("Air patrol ace effectiveness is unavailable")?;
            factor = factor.wrapping_add(multiply(
                ONE.wrapping_add(effectiveness),
                multiply(raw, scale),
            ));
            if self
                .countries()?
                .effective(2, ace.type_speed_modifier().map_err(str::to_owned)?)?
                != 0
            {
                let effective = if wing.country_override > 0 {
                    wing.country_override as u32
                } else {
                    wing.owner
                };
                let slot = self
                    .sources
                    .slot(effective)
                    .ok_or("Air patrol effective country is unresolved")?;
                factor = factor.wrapping_add(
                    self.sources
                        .command_power(slot)?
                        .air_ace_bonuses_factor
                        .ok_or("Air patrol ace country bonus is unavailable")?,
                );
            }
        }
        Ok(fraction(
            multiply(
                factor,
                required(
                    *input
                        .patrol_speed_raw
                        .require("AirMissionInputs.patrol_speed_raw")?,
                    "default cached speed",
                )?,
            ),
            defines.maximum_speed,
        ))
    }
}

fn required(value: Option<i64>, name: &str) -> Fact<i64> {
    value.ok_or_else(|| format!("Air patrol {name} is unavailable"))
}

fn night_raw(defines: &Defines, applied: Option<i64>, index: usize) -> Fact<i64> {
    let applied = required(applied, "held night accumulator application marker")?;
    if applied == -ONE {
        return Ok(0);
    }
    if !(0..=ONE).contains(&applied) {
        return Err("Air patrol night application marker is outside its native domain".into());
    }
    Ok(multiply(
        *defines.night[index].as_ref().map_err(Clone::clone)?,
        applied,
    ))
}

fn province_clock(hour: &Hour<'_>, province: u32) -> Fact<(replay_data::day_night::Clock, bool)> {
    let row = data::<tables::WeatherProvinces>(hour.frame)?
        .iter()
        .find(|row| row.id == province)
        .ok_or("Air patrol sampled province clock is absent")?;
    match row.clock.state(province).map_err(str::to_owned)? {
        ClockState::Held(clock) => Ok((clock, false)),
        ClockState::Updated => {
            let (_, geometry) = hour.topology.as_ref().map_err(Clone::clone)?;
            let centroid = geometry
                .centroids
                .get(&province)
                .copied()
                .ok_or("Air patrol sampled province centroid is absent")?;
            let defines = &hour.content.patrol_air.day_night;
            Ok((
                replay_data::day_night::ProvinceClock::new(
                    centroid,
                    [geometry.width, geometry.height],
                    defines,
                )
                .update(
                    &replay_data::day_night::HourOperands::new(hour.frame.hour, defines)
                        .map_err(str::to_owned)?,
                ),
                true,
            ))
        }
        ClockState::NotRecorded => Err("Air patrol sampled province clock was not recorded".into()),
    }
}
fn multiply(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}
fn fraction(value: i64, denominator: i64) -> i64 {
    if denominator == 0 || !air_range::finite(value) {
        INVALID
    } else {
        value.wrapping_mul(ONE).wrapping_div(denominator)
    }
}

fn disruption(
    defines: &Defines,
    raw: i64,
    count: i32,
    attack: i64,
    defence: i64,
    speed: i64,
) -> i64 {
    if defence < 1 {
        return ONE;
    }
    let denominator = multiply(
        multiply(
            ONE.wrapping_add(multiply(defence, defines.disruption_defence)),
            ONE.wrapping_add(multiply(attack, defines.disruption_attack)),
        ),
        ONE.wrapping_add(multiply(speed, defines.disruption_speed)),
    )
    .wrapping_mul(i64::from(count));
    let ratio = content_reader_map::province_geometry::native_fixed5_sqrt(fraction(raw, denominator));
    if !air_range::finite(ratio) {
        return 100;
    }
    ONE.wrapping_sub(ratio.wrapping_mul(ONE) / 316_200)
        .clamp(100, ONE)
}

fn wing_weight(count: i32, reciprocal: i64, mission_factor: i64, disruption: i64) -> i64 {
    let normalized = i64::from(count).wrapping_mul(reciprocal).wrapping_mul(ONE) / ONE;
    multiply(multiply(normalized, mission_factor), disruption)
}

fn weighted_density(
    sums: [i64; 2],
    visibility: [i64; 2],
    scale: i64,
    detection: i64,
) -> Option<i64> {
    (sums[0].wrapping_add(sums[1]) != 0).then(|| {
        multiply(
            multiply(
                multiply(sums[0], visibility[0]).wrapping_add(multiply(sums[1], visibility[1])),
                scale,
            ),
            detection,
        )
    })
}

pub fn density(hour: &Hour<'_>, region: u32, visibility: [i64; 2]) -> Fact<Option<i64>> {
    let prepared = hour
        .patrol_air
        .get_or_init(|| Prepared::read(hour))
        .as_ref()
        .map_err(Clone::clone)?;
    let Some(wings) = prepared.regions.get(&region) else {
        return Ok(None);
    };
    let defines = &hour.content.patrol_air;
    let denominator = i64::from(defines.wing_size).wrapping_mul(ONE);
    let reciprocal = if denominator == 0 {
        INVALID
    } else {
        (10_000_000_000_i128 / i128::from(denominator)) as i64
    };
    let mut sums = [0_i64; 2];
    for &wing in wings {
        let input = prepared.input(wing.id)?;
        let mask = (*input
            .current_type
            .require("AirMissionInputs.current_type")?)
        .ok_or("Air patrol current execution is unavailable")?;
        if mask as i32 <= 0 || !mask.is_power_of_two() {
            continue;
        }
        let Some(&mission_factor) = defines.mission_factors.get(mask.trailing_zeros() as usize)
        else {
            continue;
        };
        if mission_factor == 0 {
            continue;
        }
        let count = (*input
            .effective_aircraft
            .require("AirMissionInputs.effective_aircraft")?)
        .ok_or("Air patrol retained aircraft are unavailable")?;
        if count == 0 {
            continue;
        }
        let mut factor = ONE;
        if mask & 0x1f7fa != 0 {
            let raw = required(
                *input
                    .patrol_disruption_raw
                    .require("AirMissionInputs.patrol_disruption_raw")?,
                "raw mission disruption",
            )?;
            if raw != 0 && mask & 0x4400 == 0 && count > 0 {
                let provinces = hour
                    .anchors
                    .region_provinces
                    .get(&region)
                    .ok_or("Air patrol sampled region has no content provinces")?;
                let index = hour
                    .frame
                    .hour
                    .wrapping_sub(HOUR_EPOCH)
                    .checked_rem(
                        i32::try_from(provinces.len())
                            .map_err(|_| "Air patrol region province extent overflow")?,
                    )
                    .and_then(|index| usize::try_from(index).ok())
                    .ok_or("Air patrol sampled province has no native index")?;
                let province = *provinces
                    .get(index)
                    .ok_or("Air patrol sampled province index is absent")?;
                let (attack, defence) =
                    prepared.normalized_combat(hour, region, province, wing, input, mask)?;
                if defence > 0 {
                    factor = disruption(
                        defines,
                        raw,
                        count,
                        attack,
                        defence,
                        prepared.speed(defines, wing, input)?,
                    );
                }
            }
        }
        let weight = wing_weight(count, reciprocal, mission_factor, factor);
        sums[0] = sums[0].wrapping_add(multiply(
            weight,
            required(
                *input
                    .detection_stat_13
                    .require("AirMissionInputs.detection_stat_13")?,
                "surface aircraft detection statistic",
            )?,
        ));
        sums[1] = sums[1].wrapping_add(multiply(
            weight,
            required(
                *input
                    .detection_stat_14
                    .require("AirMissionInputs.detection_stat_14")?,
                "submarine aircraft detection statistic",
            )?,
        ));
    }
    if sums[0].wrapping_add(sums[1]) == 0 {
        return Ok(None);
    }
    let scores = data::<tables::RaidAirScores>(hour.frame)?;
    let mut witnessed = false;
    let mut scale = Some(0);
    for row in scores {
        if hour.links.get(&row.country) == Some(&hour.ours) {
            witnessed = true;
            if row.region == region {
                scale = row.air_spotting_scale;
            }
        }
    }
    if !witnessed {
        return Err("Air patrol regional scale has no country witness".into());
    }
    let scale = required(scale, "regional spotting scale")?;
    Ok(weighted_density(sums, visibility, scale, defines.detection))
}

#[cfg(test)]
mod tests {
    use replay_data::rows::{AirMissionInputRow, AirRegionalMissionRow, AirWingRow, DayNightClock, TableId, TableRows, WeatherProvinceRow};
    use super::super::tests::{anchors, content, frame, hour, set};
    use super::{BTreeMap, DecodedFrame, Defines, Fact, HOUR_EPOCH, NativeInputs, ONE, RaidAirScoreRow, density, disruption, night_raw, province_clock, weighted_density};
    use replay_frame::frame::TableView;

    fn input_frame() -> DecodedFrame {
        let mut frame = frame();
        set(
            &mut frame,
            TableRows::AirWings(vec![
                AirWingRow {
                    id: 11,
                    owner: 1,
                    country_override: 2,
                    mission_region: Some(1),
                    ..Default::default()
                },
                AirWingRow {
                    id: 22,
                    owner: 2,
                    country_override: 1,
                    mission_region: Some(1),
                    ..Default::default()
                },
            ]),
        );
        set(
            &mut frame,
            TableRows::AirRegionalMissions(vec![
                AirRegionalMissionRow {
                    wing: 11,
                    ..Default::default()
                },
                AirRegionalMissionRow {
                    wing: 22,
                    ..Default::default()
                },
            ]),
        );
        set(
            &mut frame,
            TableRows::AirMissionInputs(vec![AirMissionInputRow {
                wing: 11,
                current_type: (Some(8)).into(),
                effective_aircraft: (Some(80)).into(),
                detection_stat_13: (Some(ONE)).into(),
                detection_stat_14: (Some(0)).into(),
                patrol_disruption_raw: (Some(0)).into(),
                ..Default::default()
            }]),
        );
        set(
            &mut frame,
            TableRows::RaidAirScores(vec![RaidAirScoreRow {
                country: 1,
                region: 1,
                air_spotting_scale: Some(ONE),
                ..Default::default()
            }]),
        );
        for table in [
            TableId::CountryCounterInputs,
            TableId::WeatherRegions,
            TableId::WeatherProvinces,
            TableId::AceRangeInputs,
            TableId::AirBases,
        ] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "conditional input deliberately unavailable".into(),
            };
        }
        frame
    }

    fn prepared_density(frame: &DecodedFrame, visibility: [i64; 2]) -> Fact<Option<i64>> {
        let mut anchors = anchors();
        let mut content = content(b"categories={ocean={}}");
        content.patrol_air = Defines {
            wing_size: 80,
            mission_factors: vec![ONE; 4],
            detection: ONE,
            ..Default::default()
        };
        anchors.mission_content = Ok(content);
        let native = NativeInputs::default();
        let links = [(1, 1), (2, 2)].into();
        let ids = [(1, 1), (2, 2)].into();
        density(
            &hour(frame, &anchors, &native, (&links, &ids)),
            1,
            visibility,
        )
    }

    #[test]
    fn pool_owner_admission_is_independent_of_effective_volunteer_country() {
        let frame = input_frame();
        assert_eq!(prepared_density(&frame, [2 * ONE; 2]), Ok(Some(2 * ONE)));
    }

    #[test]
    fn no_contributors_and_zero_sums_skip_unreached_tables() {
        let mut frame = input_frame();
        set(
            &mut frame,
            TableRows::AirRegionalMissions(vec![AirRegionalMissionRow {
                wing: 22,
                ..Default::default()
            }]),
        );
        *frame.tables[TableId::AirMissionInputs.index()] = TableView::Unavailable {
            reason: "unused mission inputs".into(),
        };
        *frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
            reason: "unused scale".into(),
        };
        assert_eq!(prepared_density(&frame, [2 * ONE; 2]), Ok(None));
        let mut frame = input_frame();
        set(
            &mut frame,
            TableRows::AirMissionInputs(vec![AirMissionInputRow {
                wing: 11,
                current_type: (Some(8)).into(),
                effective_aircraft: (Some(80)).into(),
                patrol_disruption_raw: (Some(0)).into(),
                detection_stat_13: (Some(ONE)).into(),
                detection_stat_14: (Some(-ONE)).into(),
                ..Default::default()
            }]),
        );
        *frame.tables[TableId::RaidAirScores.index()] = TableView::Unavailable {
            reason: "cancelled sum does not load scale".into(),
        };
        assert_eq!(prepared_density(&frame, [ONE, 2 * ONE]), Ok(None));
    }

    #[test]
    fn weighted_zero_and_zero_disruption_skip_conditional_combat_graph() {
        assert_eq!(prepared_density(&input_frame(), [0, 0]), Ok(Some(0)));
        let mut frame = input_frame();
        set(
            &mut frame,
            TableRows::AirMissionInputs(vec![AirMissionInputRow {
                wing: 11,
                current_type: (Some(8)).into(),
                effective_aircraft: (Some(80)).into(),
                patrol_disruption_raw: (Some(1)).into(),
                detection_stat_13: (Some(ONE)).into(),
                detection_stat_14: (Some(0)).into(),
                ..Default::default()
            }]),
        );
        assert!(prepared_density(&frame, [2 * ONE; 2]).is_err());
    }

    #[test]
    fn sampled_night_uses_full_held_or_current_content_clock() {
        let mut frame = frame();
        frame.hour = HOUR_EPOCH + 15;
        set(
            &mut frame,
            TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                id: 1,
                clock: DayNightClock::held(replay_data::day_night::Clock {
                    local_hour: 0,
                    night_factor: 50_001,
                }),
                ..Default::default()
            }]),
        );
        let mut anchors = anchors();
        let mut content = content(b"categories={ocean={}}");
        content.patrol_air.day_night.gmt_offset = 200;
        anchors.mission_content = Ok(content);
        let native = NativeInputs::default();
        let links = BTreeMap::new();
        let ids = BTreeMap::new();
        assert_eq!(
            province_clock(&hour(&frame, &anchors, &native, (&links, &ids)), 1)
                .map(|(clock, _)| clock.night_factor),
            Ok(50_001)
        );
        set(
            &mut frame,
            TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                id: 1,
                clock: DayNightClock::UPDATED,
                ..Default::default()
            }]),
        );
        let mut hour = hour(&frame, &anchors, &native, (&links, &ids));
        let graph = content_reader_map::province_graph::ProvinceGraph::default();
        let geometry = content_reader_map::province_geometry::ProvinceGeometry {
            width: 100,
            height: 50,
            centroids: [(1, [31, 21])].into(),
            ..Default::default()
        };
        hour.topology = Ok((&graph, &geometry));
        let defines = &hour.content.patrol_air.day_night;
        let expected = replay_data::day_night::ProvinceClock::new([31, 21], [100, 50], defines)
            .update(&replay_data::day_night::HourOperands::new(frame.hour, defines).unwrap());
        assert_eq!(expected.local_hour, 0);
        assert_eq!(
            province_clock(&hour, 1).map(|(clock, _)| clock.night_factor),
            Ok(expected.night_factor)
        );
    }

    #[test]
    fn held_application_marker_distinguishes_startup_from_modded_zero_local_hour() {
        let defines = Defines {
            night: [
                Err("static night content is unavailable".into()),
                Ok(-33_333),
            ],
            ..Default::default()
        };
        let startup = Some(-ONE);
        assert_eq!(night_raw(&defines, startup, 0), Ok(0));
        assert_eq!(night_raw(&defines, startup, 1), Ok(0));
        let held = Some(50_001);
        assert!(night_raw(&defines, held, 0).is_err());
        assert_eq!(night_raw(&defines, held, 1), Ok(-16_666));
        assert!(night_raw(&defines, None, 1).is_err());
    }

    #[test]
    fn pre_visibility_cancellation_and_weighted_zero_have_distinct_admission() {
        assert_eq!(
            weighted_density([17, -17], [200_000, 100_000], ONE, ONE),
            None
        );
        assert_eq!(weighted_density([1, 1], [0, 0], ONE, ONE), Some(0));
        assert_eq!(
            weighted_density([ONE, 2 * ONE], [2 * ONE; 2], 50_000, 2_500),
            Some(7_500)
        );
        assert_eq!(
            weighted_density([i64::MIN, i64::MIN], [ONE; 2], ONE, ONE),
            None
        );
    }

    #[test]
    fn disruption_uses_normalized_combat_denominator_and_native_reduction() {
        let defines = Defines {
            disruption_attack: 43_000,
            disruption_defence: 57_000,
            disruption_speed: ONE,
            ..Default::default()
        };
        assert_eq!(disruption(&defines, ONE, 1, ONE, ONE, ONE), 85_076);
        assert_eq!(disruption(&defines, -ONE, 1, ONE, ONE, ONE), ONE);
        assert_eq!(disruption(&defines, i64::MAX, 1, ONE, ONE, ONE), 100);
        assert_eq!(disruption(&defines, i64::MAX, 1, ONE, 0, ONE), ONE);
        assert_eq!(disruption(&defines, ONE, 0, ONE, ONE, ONE), 100);
    }
}
