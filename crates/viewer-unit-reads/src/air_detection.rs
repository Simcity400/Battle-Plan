use replay_data::{rows::AirMissionInputRow, rows::CountryCounterInputRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{cell::OnceCell, collections::BTreeMap};

const ONE: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;

pub fn console_override(frame: &DecodedFrame, country: u32) -> Result<Option<i64>, String> {
    use replay_data::rows::AirSortieModifierRow as Row;
    let rows = frame
        .complete::<tables::AirSortieModifiers>()
        .map_err(|e| e.to_string())?;
    let mut witnesses = rows
        .iter()
        .filter(|row| row.scope == Row::DETECTION_OVERRIDE_STATUS);
    let witness = witnesses
        .next()
        .ok_or("Air detection override certificate is missing")?;
    if witnesses.next().is_some() || witness.subject != 0 || witness.region != 0 || witness.raw != 1
    {
        return Err("Air detection override vector was not certified".into());
    }
    let mut matched = rows
        .iter()
        .filter(|row| row.scope == Row::DETECTION_OVERRIDE && row.subject == country);
    let value = matched.next();
    if matched.next().is_some() || value.is_some_and(|row| row.region != 0) {
        return Err("Air detection override key is invalid".into());
    }
    Ok(value.map(|row| row.raw))
}

#[derive(Clone, Copy)]
pub enum CountryModifier {
    InterceptionDetection = 0,
    SuperiorityDetection = 1,
    HomeDefence = 2,
    PowerProjection = 3,
    VolunteerCapacity = 4,
    BomberNight = 5,
    Night = 6,
    Weather = 7,
    CarrierNight = 8,
    Fuel = 9,
}

pub struct Countries<'a> {
    frame: &'a DecodedFrame,
    rows: BTreeMap<u32, &'a CountryCounterInputRow>,
    definitions:
        OnceCell<Result<BTreeMap<u32, &'a replay_data::rows::ModifierDefinitionRow>, String>>,
}
impl<'a> Countries<'a> {
    pub fn read(frame: &'a DecodedFrame) -> Result<Self, String> {
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let mut rows = BTreeMap::new();
        for row in frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|e| e.to_string())?
        {
            let slot = *links
                .get(&row.country)
                .ok_or("country Air modifier owner is unresolved")?;
            if rows.insert(slot, row).is_some() {
                return Err("country Air modifier owner is duplicated".into());
            }
        }
        Ok(Self {
            frame,
            rows,
            definitions: OnceCell::new(),
        })
    }

    pub fn value(&self, slot: u32, modifier: CountryModifier) -> Result<i64, String> {
        let row = self
            .rows
            .get(&slot)
            .ok_or("country Air modifier row is missing")?;
        let values = row
            .air_modifiers
            .require("CountryCounterInputs.air_modifiers")?
            .as_ref()
            .ok_or("country Air modifier accumulator was unreadable")?;
        let index = modifier as usize;
        let raw = values[index];
        if raw.is_none() {
            return Ok(0);
        }
        let key = engine_native::ccountry::AIR_MODIFIER_KEYS[index];
        self.effective(key, raw)
    }

    fn definition(&self, key: u32) -> Result<&replay_data::rows::ModifierDefinitionRow, String> {
        self.definitions
            .get_or_init(|| {
                self.frame
                    .complete::<tables::ModifierDefinitions>()
                    .map(|rows| rows.iter().map(|row| (row.index, row)).collect())
                    .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&key)
            .copied()
            .ok_or_else(|| format!("Air modifier definition {key} is missing"))
    }
    fn features(&self) -> Option<u32> {
        match self.frame.complete::<tables::SimulationFeatures>() {
            Ok([row]) => Some(row.dlc_mask),
            _ => None,
        }
    }
    pub fn effective(&self, key: u32, raw: Option<i64>) -> Result<i64, String> {
        if raw.is_none() {
            return Ok(0);
        }
        replay_derive::naval_mission_derive::effective(self.definition(key)?, self.features(), raw)
            .map_err(str::to_owned)
    }
    fn requested(
        &self,
        key: u32,
        value: Option<replay_data::rows::AirNormalizationEntry>,
    ) -> Result<i64, String> {
        let value = value.ok_or("wing Air normalization modifier lookup was unreadable")?;
        if !value.present {
            if value.raw.is_some() {
                return Err("absent Air modifier has a raw value".into());
            }
            return Ok(0);
        }
        if value.raw.is_some() {
            return self.effective(key, value.raw);
        }
        let definition = self.definition(key)?;
        let enabled = definition.feature_selector == 0
            || (definition.eligibility != 0
                && definition.feature_selector
                    & self
                        .features()
                        .ok_or("Air normalization needs the feature mask")?
                    != 0);
        if enabled {
            Err("enabled Air normalization modifier is missing its raw value".into())
        } else {
            Ok(0)
        }
    }

    pub fn detection(&self, slot: u32) -> Result<MissionModifiers, String> {
        Ok(MissionModifiers {
            interception: self.value(slot, CountryModifier::InterceptionDetection)?,
            superiority: self.value(slot, CountryModifier::SuperiorityDetection)?,
            home_defence: self.value(slot, CountryModifier::HomeDefence)?,
            power_projection: self.value(slot, CountryModifier::PowerProjection)?,
        })
    }
}

fn ace_term(
    wing: &replay_data::rows::AirWingRow,
    index: usize,
    defines: super::air_range::Defines,
    countries: &Countries<'_>,
    sources: &impl super::air_range::Sources,
) -> Result<i64, String> {
    if wing.ace == 0 {
        return Ok(0);
    }
    let Some(ace) = sources.ace(wing.ace)? else {
        return Ok(0);
    };
    if ace.alive == 0 {
        return Ok(0);
    }
    let entries = ace
        .normalization
        .require("living ace Air normalization")?
        .ok_or("living ace Air normalization entries are unavailable")?;
    let raw = countries.effective([0x105, 0x106][index], entries[index])?;
    let scaled = multiply(raw, super::air_range::ace_scale(ace, defines, sources)?);
    let effectiveness = sources
        .command_power(ace.canonical_slot)?
        .ace_effectiveness_factor
        .ok_or("ace effectiveness modifier is unavailable")?;
    Ok(multiply(ONE.wrapping_add(effectiveness), scaled))
}

pub fn wing_normalization(
    wing: &replay_data::rows::AirWingRow,
    input: &replay_data::rows::AirMissionNormalization,
    carrier_mission: bool,
    defines: super::air_range::Defines,
    countries: &Countries<'_>,
    sources: &impl super::air_range::Sources,
) -> Result<(i64, i64), String> {
    Ok((
        wing_night_normalization(wing, input, carrier_mission, defines, countries, sources)?,
        wing_weather_normalization(wing, input, defines, countries, sources)?,
    ))
}

pub fn wing_night_normalization(
    wing: &replay_data::rows::AirWingRow,
    input: &replay_data::rows::AirMissionNormalization,
    carrier_mission: bool,
    defines: super::air_range::Defines,
    countries: &Countries<'_>,
    sources: &impl super::air_range::Sources,
) -> Result<i64, String> {
    let effective = if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    };
    let slot = sources
        .slot(effective)
        .ok_or("Air normalization country is unresolved")?;
    let mut night = ONE
        .wrapping_add(countries.value(slot, CountryModifier::Night)?)
        .wrapping_add(countries.requested(0x105, input.modifiers[0])?)
        .wrapping_add(
            input
                .night_stat
                .ok_or("current mission night statistic is unavailable")?,
        )
        .wrapping_add(ace_term(wing, 0, defines, countries, sources)?);
    if wing.mission_region.is_some() && wing.mission_mask & 8 != 0 {
        night = night.wrapping_add(countries.value(slot, CountryModifier::BomberNight)?);
    }
    if carrier_mission {
        night = night.wrapping_sub(countries.value(slot, CountryModifier::CarrierNight)?);
    }
    Ok(night.clamp(0, 2 * ONE))
}

pub fn wing_weather_normalization(
    wing: &replay_data::rows::AirWingRow,
    input: &replay_data::rows::AirMissionNormalization,
    defines: super::air_range::Defines,
    countries: &Countries<'_>,
    sources: &impl super::air_range::Sources,
) -> Result<i64, String> {
    let effective = if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    };
    let slot = sources
        .slot(effective)
        .ok_or("Air normalization country is unresolved")?;
    let weather = ONE
        .wrapping_add(countries.value(slot, CountryModifier::Weather)?)
        .wrapping_add(countries.requested(0x106, input.modifiers[1])?)
        .wrapping_add(ace_term(wing, 1, defines, countries, sources)?);
    Ok(weather.clamp(0, 2 * ONE))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Defines {
    pub occupation: i64,
    pub radar: i64,
    pub aircraft: i64,
    pub effective_aircraft: i32,
    pub night: i64,
    pub no_search_mission: i64,
}
impl Defines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        Ok(Self {
            occupation: defines.fixed5("NAir", "DETECT_CHANCE_FROM_OCCUPATION")?,
            radar: defines.fixed5("NAir", "DETECT_CHANCE_FROM_RADARS")?,
            aircraft: defines.fixed5("NAir", "DETECT_CHANCE_FROM_AIRCRAFTS")?,
            effective_aircraft: defines
                .int32("NAir", "DETECT_CHANCE_FROM_AIRCRAFTS_EFFECTIVE_COUNT")?,
            night: defines.fixed5("NAir", "DETECT_CHANCE_FROM_NIGHT")?,
            no_search_mission: defines.fixed5("NAir", "NO_SEARCH_MISSION_DETECT_FACTOR")?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MissionModifiers {
    pub interception: i64,
    pub superiority: i64,
    pub home_defence: i64,
    pub power_projection: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Execution {
    pub mask: u32,
    pub aircraft: i32,
}
impl Execution {
    pub fn read(row: &AirMissionInputRow) -> Result<Option<Self>, String> {
        let mask = (*row.current_type.require("AirMissionInputs.current_type")?)
            .ok_or("current Air mission execution is unavailable")?;
        if mask == 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            mask,
            aircraft: (*row
                .effective_aircraft
                .require("AirMissionInputs.effective_aircraft")?)
            .ok_or("executing Air mission aircraft count is unavailable")?,
        }))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mission {
    pub execution: Execution,
    pub home: bool,
    pub night: i64,
    pub weather: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detection {
    pub total: i64,
    pub occupation: i64,
    pub radar: i64,
    pub aircraft: i64,
    pub environmental_factor: i64,
    pub night: i64,
    pub weather: i64,
}

fn multiply(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}
fn fraction(value: i64, denominator: i64, zero: i64) -> i64 {
    if denominator == 0 {
        zero
    } else if super::air_range::finite(value) {
        value.wrapping_mul(ONE).wrapping_div(denominator)
    } else {
        REFUSED
    }
}

pub fn derive(
    defines: Defines,
    modifiers: MissionModifiers,
    occupation: i64,
    radar: i64,
    mean_night: i64,
    weather_modifier: Option<i64>,
    missions: impl IntoIterator<Item = Mission>,
) -> Detection {
    let mut count = 0_i32;
    let mut search = 0_i64;
    let mut night = 0_i64;
    let mut weather = 0_i64;
    for mission in missions {
        let Execution { mask, aircraft } = mission.execution;
        if mask == 0 {
            continue;
        }
        count = count.wrapping_add(aircraft);
        let planes = i64::from(aircraft).wrapping_mul(ONE);
        night = night.wrapping_add(multiply(mission.night, planes));
        weather = weather.wrapping_add(multiply(mission.weather, planes));
        let factor = if mask & 1 != 0 {
            modifiers.superiority
        } else if mask & 4 != 0 {
            modifiers.interception
        } else {
            defines.no_search_mission
        };
        let home = if mission.home {
            modifiers.home_defence
        } else {
            modifiers.power_projection
        };
        search = search.wrapping_add(multiply(
            ONE.wrapping_add(factor).wrapping_add(home),
            planes,
        ));
    }
    let aircraft = multiply(
        fraction(
            search,
            i64::from(defines.effective_aircraft).wrapping_mul(ONE),
            ONE,
        )
        .clamp(0, ONE),
        defines.aircraft,
    );
    let denominator = i64::from(count).wrapping_mul(ONE);
    let night = multiply(
        multiply(mean_night, defines.night),
        fraction(night, denominator, 0),
    );
    let weather = weather_modifier.map_or(0, |modifier| {
        multiply(modifier, fraction(weather, denominator, 0))
    });
    let occupation = multiply(occupation, defines.occupation);
    let radar = multiply(radar, defines.radar);
    let environmental_factor = ONE.wrapping_add(night).wrapping_add(weather);
    let total = multiply(
        aircraft.wrapping_add(radar).wrapping_add(occupation),
        environmental_factor,
    )
    .clamp(1_000, ONE);
    Detection {
        total,
        occupation,
        radar,
        aircraft,
        environmental_factor,
        night,
        weather,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defines() -> Defines {
        Defines {
            occupation: 10_000,
            radar: 50_000,
            aircraft: 80_000,
            effective_aircraft: 100,
            night: -20_000,
            no_search_mission: -50_000,
        }
    }
    #[test]
    fn zero_execution_ignores_stale_count_and_uses_the_native_detection_floor() {
        let row = AirMissionInputRow {
            current_type: (Some(0)).into(),
            effective_aircraft: (Some(i32::MAX)).into(),
            ..Default::default()
        };
        assert!(Execution::read(&row).unwrap().is_none());
        let result = derive(
            defines(),
            MissionModifiers::default(),
            0,
            0,
            100_000,
            Some(-50_000),
            [],
        );
        assert_eq!(
            result,
            Detection {
                total: 1_000,
                occupation: 0,
                radar: 0,
                aircraft: 0,
                environmental_factor: ONE,
                night: 0,
                weather: 0
            }
        );
    }
    #[test]
    fn current_mask_priority_home_bonus_and_signed_environment_use_separate_rounding() {
        let result = derive(
            defines(),
            MissionModifiers {
                superiority: 10_000,
                interception: 90_000,
                home_defence: 10_000,
                power_projection: -20_000,
            },
            50_000,
            20_000,
            50_000,
            Some(-10_000),
            [Mission {
                execution: Execution {
                    mask: 5,
                    aircraft: 50,
                },
                home: true,
                night: 150_000,
                weather: 50_000,
            }],
        );
        assert_eq!(result.aircraft, 48_000);
        assert_eq!(result.night, -15_000);
        assert_eq!(result.weather, -5_000);
        assert_eq!(result.total, 50_400);
    }
    #[test]
    fn zero_aircraft_divisor_saturates_before_scaling_and_missing_weather_is_zero() {
        let mut defines = defines();
        defines.effective_aircraft = 0;
        let result = derive(defines, MissionModifiers::default(), 0, 0, 0, None, []);
        assert_eq!(result.aircraft, 80_000);
        assert_eq!(result.total, 80_000);
        assert_eq!(result.weather, 0);
    }
    #[test]
    fn country_terms_use_canonical_identity_and_recorded_feature_metadata() {
        use viewer_foundation::test_support::{test_frame, test_put as put};
        use replay_data::rows::*;
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            }]),
        );
        let mut packet = [None; 10];
        packet[0] = Some(-25_000);
        packet[1] = Some(250_000);
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 11,
                air_modifiers: Recorded::Value(Some(packet)),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::ModifierDefinitions(vec![
                ModifierDefinitionRow {
                    index: 8,
                    feature_selector: 4,
                    eligibility: 1,
                    ..Default::default()
                },
                ModifierDefinitionRow {
                    index: 9,
                    flags: 0x10,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: 0,
                ..Default::default()
            }]),
        );
        let countries = Countries::read(&frame).unwrap();
        assert_eq!(
            countries
                .value(1, CountryModifier::InterceptionDetection)
                .unwrap(),
            0
        );
        assert_eq!(
            countries
                .value(1, CountryModifier::SuperiorityDetection)
                .unwrap(),
            ONE
        );
        assert_eq!(countries.value(1, CountryModifier::Weather).unwrap(), 0);
        assert_eq!(
            countries
                .requested(
                    8,
                    Some(AirNormalizationEntry {
                        present: true,
                        raw: None
                    })
                )
                .unwrap(),
            0
        );
        assert!(
            countries
                .requested(
                    9,
                    Some(AirNormalizationEntry {
                        present: true,
                        raw: None
                    })
                )
                .is_err()
        );
        drop(countries);
        put(
            &mut frame,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: 4,
                ..Default::default()
            }]),
        );
        assert_eq!(
            Countries::read(&frame)
                .unwrap()
                .value(1, CountryModifier::InterceptionDetection)
                .unwrap(),
            -25_000
        );
    }
    #[test]
    fn wing_normalization_uses_pool_override_selected_bombing_and_carrier_subtraction() {
        use viewer_foundation::test_support::{test_frame, test_put as put};
        use replay_data::rows::*;
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 22,
                    canonical_slot: 2,
                },
            ]),
        );
        let mut packet = [None; 10];
        packet[CountryModifier::Night as usize] = Some(-20_000);
        packet[CountryModifier::Weather as usize] = Some(30_000);
        packet[CountryModifier::BomberNight as usize] = Some(50_000);
        packet[CountryModifier::CarrierNight as usize] = Some(10_000);
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 22,
                air_modifiers: Recorded::Value(Some(packet)),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::ModifierDefinitions(
                [0x104, 0x105, 0x106, 0x10d]
                    .into_iter()
                    .map(|index| ModifierDefinitionRow {
                        index,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let countries = Countries::read(&frame).unwrap();
        let sources = super::super::air_range::FrameSources::new(&frame);
        let range = super::super::air_range::Defines {
            map_scale: ONE,
            ace_wing_size: ONE,
            ace_wing_size_max_bonus: ONE,
        };
        let mut wing = AirWingRow {
            owner: 11,
            country_override: 22,
            mission_region: Some(7),
            mission_mask: 8,
            ..Default::default()
        };
        let mut packet = AirMissionNormalization {
            night_stat: Some(5_000),
            modifiers: [
                Some(AirNormalizationEntry {
                    present: true,
                    raw: Some(15_000),
                }),
                Some(AirNormalizationEntry {
                    present: true,
                    raw: Some(-10_000),
                }),
            ],
        };
        assert_eq!(
            wing_normalization(&wing, &packet, true, range, &countries, &sources).unwrap(),
            (140_000, 120_000)
        );
        wing.mission_region = None;
        assert_eq!(
            wing_normalization(&wing, &packet, false, range, &countries, &sources).unwrap(),
            (100_000, 120_000)
        );
        packet.night_stat = Some(i64::MAX);
        assert_eq!(
            wing_normalization(&wing, &packet, false, range, &countries, &sources)
                .unwrap()
                .0,
            0
        );
        packet.night_stat = None;
        assert!(
            wing_normalization(&wing, &packet, false, range, &countries, &sources)
                .unwrap_err()
                .contains("current mission night")
        );
    }

    #[test]
    fn living_unassigned_ace_uses_its_owner_effectiveness_and_missing_packet_refuses() {
        use viewer_foundation::test_support::{test_frame, test_put as put};
        use replay_data::rows::*;
        let mut frame = test_frame(1);
        put(&mut frame, TableRows::CountryLinks(vec![]));
        put(&mut frame, TableRows::CountryCounterInputs(vec![]));
        put(
            &mut frame,
            TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
                index: 0x105,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::AceRangeInputs(vec![AceRangeInputRow {
                ace: 7,
                alive: 1,
                canonical_slot: 2,
                normalization: Some([Some(20_000), None]).into(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryCommandPower(vec![CountryCommandPowerRow {
                canonical_slot: 2,
                ace_effectiveness_factor: Some(50_000),
                ..Default::default()
            }]),
        );
        let wing = AirWingRow {
            ace: 7,
            ..Default::default()
        };
        let range = super::super::air_range::Defines {
            map_scale: ONE,
            ace_wing_size: ONE,
            ace_wing_size_max_bonus: ONE,
        };
        {
            let sources = super::super::air_range::FrameSources::new(&frame);
            let countries = Countries::read(&frame).unwrap();
            assert_eq!(
                ace_term(&wing, 0, range, &countries, &sources).unwrap(),
                30_000
            );
            assert_eq!(ace_term(&wing, 1, range, &countries, &sources).unwrap(), 0);
        }
        put(
            &mut frame,
            TableRows::AceRangeInputs(vec![AceRangeInputRow {
                ace: 7,
                alive: 1,
                canonical_slot: 2,
                ..Default::default()
            }]),
        );
        let countries = Countries::read(&frame).unwrap();
        let sources = super::super::air_range::FrameSources::new(&frame);
        assert!(
            ace_term(&wing, 0, range, &countries, &sources)
                .unwrap_err()
                .contains("living ace")
        );
    }
    #[test]
    fn console_override_requires_its_certificate_and_keeps_zero_and_unclamped_values() {
        use viewer_foundation::test_support::{test_frame, test_put as put};
        use replay_data::rows::*;
        let mut frame = test_frame(1);
        put(&mut frame, TableRows::AirSortieModifiers(vec![]));
        assert!(console_override(&frame, 7).is_err());
        for (known, value) in [
            (false, None),
            (true, None),
            (true, Some(0)),
            (true, Some(-123_456)),
            (true, Some(i64::MAX)),
        ] {
            let mut rows = vec![AirSortieModifierRow {
                scope: AirSortieModifierRow::DETECTION_OVERRIDE_STATUS,
                raw: i64::from(known),
                ..Default::default()
            }];
            if let Some(raw) = value {
                rows.push(AirSortieModifierRow {
                    scope: AirSortieModifierRow::DETECTION_OVERRIDE,
                    subject: 7,
                    raw,
                    ..Default::default()
                });
            }
            put(&mut frame, TableRows::AirSortieModifiers(rows));
            if known {
                assert_eq!(console_override(&frame, 7).unwrap(), value);
            } else {
                assert!(console_override(&frame, 7).is_err());
            }
        }
    }
}
