//! The map's weather effect entities, native thermal tooltip intervals and the
//! hourly temperature pass's settings ([`temperature`]) from `common/weather.txt`,
//! through the recorded content stack.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    mesh::{self, Descriptors},
    parser::native::{self, Shape},
    particles::{Catalog, Particle},
};

pub mod temperature;

const PATH: &str = "common/weather.txt";

/// `weather_effects` blocks in region flag order:
/// light rain, heavy rain, snow, blizzard, sandstorm.
pub const EFFECT_KINDS: [&str; 5] = ["rain_light", "rain_heavy", "snow", "blizzard", "sandstorm"];

/// One `weather_effect`: pair member 0 is `entity`, member 1 `fading_entity`.
/// Either may be absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectPair {
    pub entity: Vec<Particle>,
    pub fading_entity: Vec<Particle>,
    /// Dummy parent spheres of the two members, independent of their particles.
    pub parents: [Option<EffectParent>; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectParent {
    pub name: String,
    /// Authored cull radius times the parent's maximum basis length.
    pub radius: f32,
    /// The default state's sound events, in event order.
    pub sounds: Vec<WeatherSound>,
}

/// A weather entity's sound event: the effect it names and its `time`, the
/// seconds after the entity enters its state at which it fires.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherSound {
    pub effect: String,
    pub time: f32,
}

/// Effect pairs by (kind index, `small`), in authored order: the producer
/// picks one by index.
pub type Effects = BTreeMap<(usize, bool), Vec<EffectPair>>;

type Names = BTreeMap<(usize, bool), Vec<[Option<String>; 2]>>;

/// Weather-manager interval order, also the static-modifier slot identifiers.
const TEMPERATURE_MODIFIERS: [&str; 4] = [
    "weather_extreme_cold",
    "weather_very_cold",
    "weather_very_hot",
    "weather_extreme_hot",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemperatureRange {
    /// Signed native fixed5 bounds; lower inclusive, upper exclusive.
    pub lower: i64,
    pub upper: i64,
    /// Localise the effective static modifier's identifier. None is the
    /// native null modifier's empty display name, never a vanilla fallback.
    pub modifier: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemperatureRanges {
    pub ranges: [TemperatureRange; 4],
    /// The identifier reader's named content diagnostics.
    pub notes: Vec<String>,
}

// Load the file to EOF. Unknown settings terminate that loader; the
// presentation reader refuses them by name, including malformed ranges,
// instead of returning effects the loader would never reach.
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
    // Each file constructs the lexer in mode 0.
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
                // Map an unrecognized kind to slot zero.
                (TerrainKinds, _) => Shape::Object(TerrainList),
                (TerrainList, Some(b"terrain")) => Shape::Object(Terrain),
                (Terrain, _) => Shape::One, // No unknown-field skip.
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
                // Raw fixed5 list; exactly two values required or the outer loader
                // aborts. Validated below.
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
                    // Read the lexer's tokens, leaving the value slot unchanged.
                    Shape::RawList
                }
                // Read effects, their pair lists and each entity through the
                // corresponding field readers.
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

pub(crate) fn parse_effect_names(bytes: &[u8]) -> io::Result<Names> {
    let document = parse_weather(bytes)?;
    let mut out = Names::new();
    for effects in document
        .items
        .iter()
        .filter(|item| item.keyword_is("weather_effects"))
    {
        // A repeated effects block clears all banks.
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
                        // A failed boolean scan leaves the byte unchanged.
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

fn parse_temperature_ranges(bytes: &[u8]) -> io::Result<[[i64; 2]; 4]> {
    let document = parse_weather(bytes)?;
    // Construction zeroes all interval bounds. Repeated settings/groups
    // keep other slots; each encountered pair overwrites its two words.
    let mut ranges = [[0; 2]; 4];
    for settings in document
        .items
        .iter()
        .filter(|item| item.keyword_is("settings"))
    {
        for temperature in block(&settings.value, "weather settings")?
            .iter()
            .filter(|item| item.keyword_is("temperature"))
        {
            for field in block(&temperature.value, "weather temperature")? {
                let Some(slot) = TEMPERATURE_MODIFIERS
                    .iter()
                    .position(|key| field.keyword_is(key))
                else {
                    continue;
                };
                let values = block(&field.value, "weather temperature range")?;
                let [lower, upper] = values else {
                    return Err(invalid(format!(
                        "weather temperature range {} needs two values",
                        TEMPERATURE_MODIFIERS[slot]
                    )));
                };
                // Start each list entry at zero; failed fixed5 conversion leaves
                // zero, as the scanner does.
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

/// The four native thermal intervals and their stack-resolved modifier
/// identifiers. Numeric/classification consumers retain full fixed5 values.
pub fn temperature_ranges(layers: &ResolvedLayers) -> io::Result<TemperatureRanges> {
    let ranges = parse_temperature_ranges(&layers.read(PATH)?)
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

/// The particles one weather entity's default state emits. Only the forms the
/// weather pass draws are accepted: particle events at time zero that keep
/// their particle and attach to no node, with no mesh, attachment, state
/// transition, animation or weighted variants.
fn entity_particles(
    name: &str,
    entities: &Descriptors,
    catalog: &Catalog,
) -> io::Result<(Vec<Particle>, EffectParent)> {
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
        if event.time != 0.0 || !event.keep_particle || !event.node.is_empty() {
            return Err(refuse("has a timed, transient or attached particle event"));
        }
        let mut particle = catalog.resolve(object)?;
        particle.scale *= scale;
        out.push(particle);
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

/// The weather effects of the stack, each entity resolved to its particles.
pub fn effects(layers: &ResolvedLayers, catalog: &Catalog) -> io::Result<Effects> {
    let names = parse_effect_names(&layers.read(PATH)?)
        .map_err(|error| invalid(format!("{PATH}: {error}")))?;
    //The existing memoized registry is also used by entity lights/models.
    //Clones copy the parent's registration-time definition, not its final one.
    let roster = layers.descriptor_roster()?;
    let resolve = |name: &Option<String>| -> io::Result<(Vec<Particle>, Option<EffectParent>)> {
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
    names
        .into_iter()
        .map(|(key, pairs)| {
            let pairs = pairs
                .iter()
                .map(|[entity, fading]| {
                    let (entity, primary) = resolve(entity)?;
                    let (fading_entity, fading) = resolve(fading)?;
                    Ok(EffectPair {
                        entity,
                        fading_entity,
                        parents: [primary, fading],
                    })
                })
                .collect::<io::Result<_>>()?;
            Ok((key, pairs))
        })
        .collect()
}

#[cfg(test)]
mod tests;
