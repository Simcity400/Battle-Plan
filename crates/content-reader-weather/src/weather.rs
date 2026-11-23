
use std::{collections::BTreeMap, io};

use content_reader_mesh::mesh::MeshProducts as _;
use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    mesh::{self, Descriptors},
    particles::{Catalog, Particle},
};

pub mod temperature;

const PATH: &str = "common/weather.txt";

pub const EFFECT_KINDS: [&str; 5] = ["rain_light", "rain_heavy", "snow", "blizzard", "sandstorm"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectPair {
    pub name: String,
    pub entity: Vec<EffectParticle>,
    pub fading_entity: Vec<EffectParticle>,
    pub parents: [Option<EffectParent>; 2],
    pub withheld: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectParticle {
    pub particle: Particle,
    pub delay: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectParent {
    pub name: String,
    pub radius: f32,
    pub sounds: Vec<WeatherSound>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeatherSound {
    pub effect: String,
    pub time: f32,
}

pub type Effects = BTreeMap<(usize, bool), Vec<EffectPair>>;

type Names = BTreeMap<(usize, bool), Vec<[Option<String>; 2]>>;

pub const TEMPERATURE_MODIFIERS: [&str; 4] = [
    "weather_extreme_cold",
    "weather_very_cold",
    "weather_very_hot",
    "weather_extreme_hot",
];

pub const MUD_MODIFIER: &str = "weather_mud";

pub const GROUND_SNOW_MODIFIERS: [&str; 2] =
    ["weather_ground_snow_medium", "weather_ground_snow_high"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceModifierRules {
    pub temperature: [[i64; 2]; 4],
    pub ground_snow: [[i64; 2]; 2],
    pub snow_gain_max: i64,
}

impl ProvinceModifierRules {
    pub fn reachable_temperature_modifiers(&self) -> Vec<Option<&'static str>> {
        let points = std::iter::once(i64::MIN).chain(self.temperature.iter().flatten().copied());
        let mut reached = Vec::new();
        for point in points {
            let modifier = self.temperature_modifier(point);
            if !reached.contains(&modifier) {
                reached.push(modifier);
            }
        }
        reached
    }

    pub fn reachable_ground_snow_modifiers(&self) -> Vec<Option<&'static str>> {
        let points = [i64::MIN, self.snow_gain_max]
            .into_iter()
            .chain(self.ground_snow.iter().flatten().copied())
            .filter(|&point| point <= self.snow_gain_max);
        let mut reached = Vec::new();
        for point in points {
            let modifier = self.ground_snow_modifier(point, 0);
            if !reached.contains(&modifier) {
                reached.push(modifier);
            }
        }
        reached
    }

    pub fn temperature_modifier(&self, temperature: i64) -> Option<&'static str> {
        self.temperature
            .iter()
            .position(|&range| holds(range, temperature))
            .map(|slot| TEMPERATURE_MODIFIERS[slot])
    }

    pub fn ground_snow_modifier(&self, snow: i64, permanent_snow: i64) -> Option<&'static str> {
        let snow = snow.wrapping_add(permanent_snow).min(self.snow_gain_max);
        let [medium, high] = self.ground_snow;
        if holds(high, snow) {
            Some(GROUND_SNOW_MODIFIERS[1])
        } else if holds(medium, snow) {
            Some(GROUND_SNOW_MODIFIERS[0])
        } else {
            None
        }
    }
}

fn holds([lower, upper]: [i64; 2], value: i64) -> bool {
    lower <= value && value < upper
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemperatureRange {
    pub lower: i64,
    pub upper: i64,
    pub modifier: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemperatureRanges {
    pub ranges: [TemperatureRange; 4],
    pub notes: Vec<String>,
}

#[derive(Clone, Copy)]
enum WeatherNode {
    File,
    TerrainKinds,
    TerrainList,
    Terrain,
    Settings,
    Temperature,
    Mud,
    Snow,
    Chance,
    Duration,
    Performance,
    Arctic,
    Effects,
    List,
    Pair,
}

fn parse_weather(bytes: &[u8]) -> io::Result<native::Read> {
    use WeatherNode::*;
    let mut refused = Vec::new();
    let document = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(File),
        None,
        &mut |node, field| {
            let key = field.keyword();
            match (node, key.as_deref()) {
                (File, Some(b"weather_effects")) => Shape::Object(Effects),
                (File, Some(b"terrain_modifiers")) => Shape::Object(TerrainKinds),
                (File, Some(b"settings")) => Shape::Object(Settings),
                (TerrainKinds, _) => Shape::Object(TerrainList),
                (TerrainList, Some(b"terrain")) => Shape::Object(Terrain),
                (Terrain, _) => Shape::One,
                (Settings, Some(b"temperature")) => Shape::Object(Temperature),
                (Settings, Some(b"mud")) => Shape::Object(Mud),
                (Settings, Some(b"snowing")) => Shape::Object(Snow),
                (Settings, Some(b"chance_increase")) => Shape::Object(Chance),
                (Settings, Some(b"duration")) => Shape::Object(Duration),
                (Settings, Some(b"performance")) => Shape::Object(Performance),
                (Settings, Some(b"arctic_water")) => Shape::Object(Arctic),
                (Settings, Some(b"visual_mud_effects")) => Shape::List,
                (
                    Temperature,
                    Some(
                        b"temperature_variation"
                        | b"max_temperature_change"
                        | b"temperature_neighbor_smoothing"
                        | b"temperature_change_neighbor_smoothing",
                    ),
                )
                | (
                    Mud,
                    Some(
                        b"water_gain_on_rain_light"
                        | b"water_gain_on_rain_heavy"
                        | b"water_gain_max"
                        | b"water_gain_min"
                        | b"water_gain_to_cm"
                        | b"min_temperature",
                    ),
                )
                | (
                    Snow,
                    Some(
                        b"snow_gain_on_snowing"
                        | b"snow_gain_on_blizzard"
                        | b"snow_gain_max"
                        | b"snow_gain_to_cm"
                        | b"snow_visual_min",
                    ),
                )
                | (
                    Performance,
                    Some(
                        b"provinces_per_update"
                        | b"regions_per_update"
                        | b"init_run_passes"
                        | b"texture_refresh_freq",
                    ),
                ) => Shape::One,
                (
                    Temperature,
                    Some(
                        b"weather_extreme_cold"
                        | b"weather_very_cold"
                        | b"weather_very_hot"
                        | b"weather_extreme_hot",
                    ),
                )
                | (Mud, Some(b"water_dryout_temperature" | b"water_dryout_multiplier"))
                | (
                    Snow,
                    Some(
                        b"snow_melt_temperature"
                        | b"snow_melt_multiplier"
                        | b"temperature"
                        | b"weather_ground_snow_medium"
                        | b"weather_ground_snow_high",
                    ),
                )
                | (Chance, Some(b"mud"))
                | (
                    Duration,
                    Some(
                        b"no_phenomenon" | b"rain_light" | b"rain_heavy" | b"snow" | b"blizzard"
                        | b"sandstorm",
                    ),
                )
                | (Arctic, Some(b"arctic_water_temperature" | b"arctic_water_end_temperature")) => {
                    Shape::RawList
                }
                (
                    Effects,
                    Some(b"rain_light" | b"rain_heavy" | b"snow" | b"blizzard" | b"sandstorm"),
                ) => Shape::Object(List),
                (List, Some(b"weather_effect")) => Shape::Object(Pair),
                (Pair, Some(b"entity" | b"fading_entity" | b"small")) => Shape::One,
                (Effects | List | Pair, _) => Shape::Skip,
                _ => {
                    refused.push(format!(
                        "weather loader field {} at line {}",
                        String::from_utf8_lossy(&field.key.text),
                        field.key.line
                    ));
                    Shape::Skip
                }
            }
        },
    );
    if let Some(cause) = refused.first() {
        return Err(invalid(format!("unsupported {cause}")));
    }
    for settings in document
        .items
        .iter()
        .filter(|item| item.keyword_is("settings"))
    {
        for group in block(&settings.value, "weather settings")? {
            if group.keyword_is("visual_mud_effects") {
                continue;
            }
            for field in block(&group.value, "weather settings group")? {
                if let Some(values) = field.value.items()
                    && values.len() != 2
                {
                    return Err(invalid(format!(
                        "weather range {} has {} values; native loader aborts",
                        field.key_str().unwrap_or_default(),
                        values.len()
                    )));
                }
            }
        }
    }
    if let Some(unsettled) = &document.unsettled {
        return Err(invalid(format!("unsupported weather loader: {unsettled}")));
    }
    Ok(document)
}

#[cfg(test)]
pub(crate) fn parse_effect_names(bytes: &[u8]) -> io::Result<Names> {
    let document = parse_weather(bytes)?;
    effect_names(&document)
}

fn effect_names(document: &native::Read) -> io::Result<Names> {
    let mut out = Names::new();
    for effects in document
        .items
        .iter()
        .filter(|item| item.keyword_is("weather_effects"))
    {
        out.clear();
        for list in block(&effects.value, "weather_effects")? {
            let Some(kind) = EFFECT_KINDS.iter().position(|key| list.keyword_is(key)) else {
                continue;
            };
            for value in block(&list.value, "weather effect list")?
                .iter()
                .filter(|item| item.keyword_is("weather_effect"))
            {
                let mut pair = [None, None];
                let mut small = false;
                for field in block(&value.value, "weather_effect")? {
                    if field.keyword_is("entity") || field.keyword_is("fading_entity") {
                        let name = field
                            .value
                            .read_str()
                            .map_err(|e| invalid(format!("weather entity: {e}")))?;
                        pair[usize::from(field.keyword_is("fading_entity"))] =
                            (!name.is_empty()).then(|| name.to_owned());
                    } else if field.keyword_is("small") {
                        small = field
                            .value
                            .scalar()
                            .and_then(crate::parser::read_bool)
                            .unwrap_or(small);
                    }
                }
                if pair.iter().any(Option::is_some) {
                    out.entry((kind, small)).or_default().push(pair);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
fn parse_temperature_ranges(bytes: &[u8]) -> io::Result<[[i64; 2]; 4]> {
    read_ranges(
        &parse_weather(bytes)?,
        "temperature",
        &TEMPERATURE_MODIFIERS,
    )
}

fn read_ranges<const N: usize>(
    document: &native::Read,
    group: &str,
    keys: &[&str; N],
) -> io::Result<[[i64; 2]; N]> {
    let mut ranges = [[0; 2]; N];
    for settings in document
        .items
        .iter()
        .filter(|item| item.keyword_is("settings"))
    {
        for settings_group in block(&settings.value, "weather settings")?
            .iter()
            .filter(|item| item.keyword_is(group))
        {
            for field in block(&settings_group.value, "weather settings group")? {
                let Some(slot) = keys.iter().position(|key| field.keyword_is(key)) else {
                    continue;
                };
                let values = block(&field.value, "weather range")?;
                let [lower, upper] = values else {
                    return Err(invalid(format!(
                        "weather {group} range {} needs two values",
                        keys[slot]
                    )));
                };
                let fixed = |value: &crate::parser::Item| {
                    value
                        .value
                        .scalar()
                        .and_then(crate::fixed5::script_fixed5)
                        .unwrap_or(0)
                };
                ranges[slot] = [fixed(lower), fixed(upper)];
            }
        }
    }
    Ok(ranges)
}

#[cfg(test)]
fn parse_province_modifier_rules(bytes: &[u8]) -> io::Result<ProvinceModifierRules> {
    let document = parse_weather(bytes)?;
    province_rules(&document).map_err(|error| invalid(format!("{PATH}: {error}")))
}

fn province_rules(document: &native::Read) -> io::Result<ProvinceModifierRules> {
    let mut snow_gain_max = 0;
    for settings in document
        .items
        .iter()
        .filter(|item| item.keyword_is("settings"))
    {
        for snowing in block(&settings.value, "weather settings")?
            .iter()
            .filter(|item| item.keyword_is("snowing"))
        {
            for field in block(&snowing.value, "weather settings group")? {
                if field.keyword_is("snow_gain_max")
                    && let Some(text) = field.value.scalar()
                {
                    snow_gain_max = crate::fixed5::decimal_fixed5(text);
                }
            }
        }
    }
    Ok(ProvinceModifierRules {
        temperature: read_ranges(&document, "temperature", &TEMPERATURE_MODIFIERS)?,
        ground_snow: read_ranges(&document, "snowing", &GROUND_SNOW_MODIFIERS)?,
        snow_gain_max,
    })
}

pub fn province_modifier_rules(layers: &ResolvedLayers) -> io::Result<ProvinceModifierRules> {
    let document = read_weather(layers)?;
    province_rules(&document)
}

fn read_weather(layers: &ResolvedLayers) -> io::Result<std::sync::Arc<native::Read>> {
    layers.recorded_product("weather file grammar", 1, &[], |layers| {
        parse_weather(&layers.read(PATH)?).map_err(|error| invalid(format!("{PATH}: {error}")))
    })
}

pub fn temperature_ranges(layers: &ResolvedLayers) -> io::Result<TemperatureRanges> {
    let document = read_weather(layers)?;
    let ranges = read_ranges(&document, "temperature", &TEMPERATURE_MODIFIERS)
        .map_err(|error| invalid(format!("{PATH}: {error}")))?;
    let identifiers = crate::static_modifiers::read_identifiers(layers, &TEMPERATURE_MODIFIERS)?;
    let mut out = std::array::from_fn(|slot| TemperatureRange {
        lower: ranges[slot][0],
        upper: ranges[slot][1],
        modifier: None,
    });
    for (range, identifier) in out.iter_mut().zip(identifiers.identifiers) {
        range.modifier = identifier.map_err(|refusal| {
            invalid(format!(
                "{}: temperature modifier {}: {}",
                refusal.file, refusal.modifier, refusal.reason
            ))
        })?;
    }
    Ok(TemperatureRanges {
        ranges: out,
        notes: identifiers.notes,
    })
}

fn entity_particles(
    name: &str,
    entities: &Descriptors,
    catalog: &Catalog,
) -> io::Result<(Vec<EffectParticle>, EffectParent)> {
    let refuse = |why: &str| invalid(format!("weather entity {name}: {why}"));
    let mut link = name;
    for _ in 0..=entities.entities.len() {
        let entity = entities
            .entities
            .get(link)
            .ok_or_else(|| refuse(&format!("missing entity {link}")))?;
        match entity.clone.as_deref() {
            Some(parent) => link = parent,
            None => break,
        }
    }
    let entity = entities.inherit(name).ok_or_else(|| refuse("missing"))?;
    if entity.mesh.is_some() || !entity.attachments.is_empty() {
        return Err(refuse("has a mesh or attachments"));
    }
    let scale = entity.scale.unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(refuse("has an unsupported scale"));
    }
    let radius = entity.cull_radius.unwrap_or(1.0) * (scale * scale).sqrt();
    if !radius.is_finite() {
        return Err(refuse("has a non-finite transformed cull radius"));
    }
    let state = entity.default_state.unwrap_or(mesh::DEFAULT_STATE);
    let [state] = entity
        .states
        .get(state)
        .map(Vec::as_slice)
        .unwrap_or_default()
    else {
        return Err(refuse("needs exactly one default state"));
    };
    if state.next_state.is_some() || !state.animation.is_empty() {
        return Err(refuse("has a state transition or animation"));
    }
    let mut out = Vec::new();
    for event in &state.events {
        let Some(object) = &event.particle else {
            continue;
        };
        if !event.keep_particle {
            return Err(refuse(&format!(
                "particle event {object} does not keep its particle"
            )));
        }
        if !event.node.is_empty() {
            return Err(refuse(&format!(
                "particle event {object} attaches to node {}",
                event.node
            )));
        }
        let once = !state.looping || event.trigger_once;
        let plain_clock = state.speed == 1.0 && state.start == 0.0 && state.random_start <= 0.0;
        if event.time != 0.0 && !(event.time > 0.0 && event.time.is_finite() && once && plain_clock)
        {
            return Err(refuse(&format!(
                "particle event {object} at time {} in a state that loops it or scales or offsets its clock",
                event.time
            )));
        }
        let mut particle = catalog.resolve(object)?;
        particle.scale *= scale;
        out.push(EffectParticle {
            particle,
            delay: event.time,
        });
    }
    Ok((
        out,
        EffectParent {
            name: name.to_owned(),
            radius,
            sounds: state
                .events
                .iter()
                .filter_map(|event| {
                    Some(WeatherSound {
                        effect: event.sound.clone()?,
                        time: event.time,
                    })
                })
                .collect(),
        },
    ))
}

pub fn effects(layers: &ResolvedLayers, catalog: &Catalog) -> io::Result<Effects> {
    let document = read_weather(layers)?;
    let names = effect_names(&document)
        .map_err(|error| invalid(format!("{PATH}: {error}")))?;
    let roster = layers.descriptor_roster()?;
    let resolve = |name: &Option<String>| -> io::Result<(Vec<EffectParticle>, Option<EffectParent>)> {
        match name {
            Some(name) => {
                if let Some(path) = roster.unreadable_names.get(name) {
                    return Err(invalid(format!(
                        "weather entity {name}: unreadable descriptor {path}"
                    )));
                }
                let (particles, parent) = entity_particles(name, &roster.descriptors, catalog)?;
                Ok((particles, Some(parent)))
            }
            None => Ok((Vec::new(), None)),
        }
    };
    Ok(names
        .into_iter()
        .map(|((kind, small), pairs)| {
            let pairs = pairs
                .iter()
                .enumerate()
                .map(|(index, [entity, fading])| {
                    let name = format!(
                        "{PATH} {}{} weather_effect {} (entity {}, fading_entity {})",
                        EFFECT_KINDS[kind],
                        if small { " small" } else { "" },
                        index + 1,
                        entity.as_deref().unwrap_or("none"),
                        fading.as_deref().unwrap_or("none"),
                    );
                    let resolved = resolve(entity).and_then(|primary| Ok((primary, resolve(fading)?)));
                    match resolved {
                        Ok(((entity, primary), (fading_entity, fading))) => EffectPair {
                            name,
                            entity,
                            fading_entity,
                            parents: [primary, fading],
                            withheld: None,
                        },
                        Err(error) => EffectPair {
                            withheld: Some(error.to_string()),
                            name,
                            ..EffectPair::default()
                        },
                    }
                })
                .collect();
            ((kind, small), pairs)
        })
        .collect())
}

#[cfg(test)]
mod tests;
