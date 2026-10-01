//! The map's weather effect entities from `common/weather.txt`, resolved to
//! their particles.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    mesh::{self, Descriptors},
    parser::native::{self, Shape},
    particles::{Catalog, Particle},
};

const PATH: &str = "common/weather.txt";

/// `weather_effects` blocks in the region flag order `14166ec50` tests:
/// light rain, heavy rain, snow, blizzard, sandstorm.
pub const EFFECT_KINDS: [&str; 5] = ["rain_light", "rain_heavy", "snow", "blizzard", "sandstorm"];

/// One `weather_effect`: pair member 0 is `entity`, member 1 `fading_entity`
/// (`140f0aec0`). Either may be absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectPair {
    pub entity: Vec<Particle>,
    pub fading_entity: Vec<Particle>,
}

/// Effect pairs by (kind index, `small`), in authored order: the producer
/// picks one by index.
pub type Effects = BTreeMap<(usize, bool), Vec<EffectPair>>;

type Names = BTreeMap<(usize, bool), Vec<[Option<String>; 2]>>;

// 140F08970 loads the file to EOF. Unknown settings terminate that loader;
// the presentation reader refuses them by name, including malformed ranges,
// instead of returning effects the native loader would never reach.
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

pub(crate) fn parse_effect_names(bytes: &[u8]) -> io::Result<Names> {
    use WeatherNode::*;
    let mut refused = Vec::new();
    // 140F08970 constructs the lexer in mode 0 (1424A63A0).
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
                // 1419FBD90 maps an unrecognized kind to slot zero.
                (TerrainKinds, _) => Shape::Object(TerrainList),
                (TerrainList, Some(b"terrain")) => Shape::Object(Terrain),
                (Terrain, _) => Shape::One, // 1419FBB70: no unknown-field skip.
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
                // 140F0AD80 -> 140604600: raw fixed5 list; exactly two values
                // required or the outer loader aborts. Validated below.
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
                    // 140F0AD80 -> 140604600: the lexer's tokens, value slot
                    // (+0xC0) unchanged.
                    Shape::RawList
                }
                // SWeatherEffectsReader 140F0B0D0, SWeatherEffectListReader
                // 140F0B050, SWeatherEffectEntityReader 140F0AEC0.
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
    let mut out = Names::new();
    for effects in document
        .items
        .iter()
        .filter(|item| item.keyword_is("weather_effects"))
    {
        // A repeated effects block clears all banks (140F08970).
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
                        // SWeatherEffectEntityReader::vf4@140F0AEC0:
                        // read_bool_field, byte unchanged on failed scan.
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

/// The particles one weather entity's default state emits. Only the forms the
/// weather pass draws are accepted: particle events at time zero that keep
/// their particle and attach to no node, with no mesh, attachment, state
/// transition, animation or weighted variants.
fn entity_particles(
    name: &str,
    entities: &Descriptors,
    catalog: &Catalog,
) -> io::Result<Vec<Particle>> {
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
    Ok(out)
}

/// Whether `bytes` holds any of `names` as a whole identifier: a cheap filter
/// so only descriptor files naming a weather entity are parsed.
fn mentions(bytes: &[u8], names: &[String]) -> bool {
    bytes
        .split(|&byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))
        .any(|word| !word.is_empty() && names.iter().any(|name| name.as_bytes() == word))
}

/// Entity descriptors for `names` and their `clone` parents, from the files
/// in `paths` that mention them, merged in path order.
fn weather_descriptors(
    paths: &[String],
    mut names: Vec<String>,
    mut read: impl FnMut(&str) -> io::Result<Vec<u8>>,
) -> io::Result<Descriptors> {
    let mut parsed: BTreeMap<usize, Descriptors> = BTreeMap::new();
    loop {
        for (index, path) in paths.iter().enumerate() {
            if parsed.contains_key(&index) {
                continue;
            }
            let bytes = read(path)?;
            if mentions(&bytes, &names) {
                let descriptors = mesh::parse_descriptors(&bytes, path)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
                parsed.insert(index, descriptors);
            }
        }
        let mut merged = Descriptors::default();
        for descriptors in parsed.values() {
            merged.entities.extend(descriptors.entities.clone());
        }
        let before = names.len();
        for name in names.clone() {
            if let Some(parent) = merged.entities.get(&name).and_then(|e| e.clone.clone())
                && !names.contains(&parent)
            {
                names.push(parent);
            }
        }
        if names.len() == before {
            return Ok(merged);
        }
    }
}

/// The weather effects of the stack, each entity resolved to its particles.
pub fn effects(layers: &ResolvedLayers, catalog: &Catalog) -> io::Result<Effects> {
    let names = parse_effect_names(&layers.read(PATH)?)
        .map_err(|error| invalid(format!("{PATH}: {error}")))?;
    let mut wanted: Vec<String> = names
        .values()
        .flatten()
        .flatten()
        .flatten()
        .cloned()
        .collect();
    wanted.sort();
    wanted.dedup();
    let entities =
        weather_descriptors(catalog.assets(), wanted, |path| layers.read_uncovered(path))?;
    let resolve = |name: &Option<String>| -> io::Result<Vec<Particle>> {
        match name {
            Some(name) => entity_particles(name, &entities, catalog),
            None => Ok(Vec::new()),
        }
    };
    names
        .into_iter()
        .map(|(key, pairs)| {
            let pairs = pairs
                .iter()
                .map(|[entity, fading]| {
                    Ok(EffectPair {
                        entity: resolve(entity)?,
                        fading_entity: resolve(fading)?,
                    })
                })
                .collect::<io::Result<_>>()?;
            Ok((key, pairs))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_variables_keyword_tokens_and_error_recovery() {
        let names = parse_effect_names(br#"@sprite=rain weather_effects={RAIN_LIGHT={
            weather_effect={unknown={entity=hidden} "entity"=ignored ENTITY=@sprite small=yes small=bad}
        }}"#).unwrap();
        assert_eq!(names[&(0, true)][0][0].as_deref(), Some("rain"));
        assert!(
            parse_effect_names(b"settings={temperature={weather_very_hot={1}}}")
                .unwrap_err()
                .to_string()
                .contains("weather_very_hot")
        );
        assert!(
            parse_effect_names(b"settings={unsupported={}} ")
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }

    #[test]
    fn effects_keep_banks_pair_members_and_authored_order() {
        let names = parse_effect_names(
            br#"weather_effects = {
                rain_light = {
                    weather_effect = { entity = rain fading_entity = cloud }
                    weather_effect = { entity = other }
                    weather_effect = { entity = small_rain small = yes }
                }
                sandstorm = { weather_effect = { fading_entity = dust small = yes } }
                snow = { weather_effect = { } }
            }"#,
        )
        .unwrap();
        let name = |value: &str| Some(value.to_owned());
        assert_eq!(
            names[&(0, false)],
            [[name("rain"), name("cloud")], [name("other"), None]]
        );
        assert_eq!(names[&(0, true)], [[name("small_rain"), None]]);
        assert_eq!(names[&(4, true)], [[None, name("dust")]]);
        assert!(!names.contains_key(&(2, false)));
    }

    #[test]
    fn entities_resolve_through_clones_and_refuse_unsupported_forms() {
        let entities = mesh::parse_descriptors(
            br#"
            entity = { name = base scale = 2 default_state = idle
                state = { name = idle state_time = 2 looping = no
                    event = { time = 0 particle = drops keep_particle = yes }
                    event = { trigger_once = yes sound = { soundeffect = rain } }
                }
            }
            entity = { name = custom clone = base }
            entity = { name = timed state = { name = idle event = { time = 1 particle = drops keep_particle = yes } } }
            entity = { name = orphan clone = absent }
            "#,
            "gfx/entities/weather.asset",
        )
        .unwrap();
        let mut catalog = Catalog::default();
        catalog.add_test_particle("drops", "unrelated_id", 0.5, "gfx/particles/x/file.asset");
        assert_eq!(
            entity_particles("custom", &entities, &catalog).unwrap(),
            [Particle {
                path: "gfx/particles/x/file.asset".into(),
                name: "unrelated_id".into(),
                scale: 1.0,
                registration: 0,
            }]
        );
        for name in ["timed", "orphan", "absent"] {
            assert!(
                entity_particles(name, &entities, &catalog).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn descriptors_come_from_files_that_name_the_entities_and_follow_clone_parents() {
        let paths = ["a.asset", "units.asset", "storm.asset", "late.asset"].map(String::from);
        let files = BTreeMap::from([
            ("a.asset", &b"entity = { name = base scale = 2 }"[..]),
            // Unrelated files may be malformed: they are never parsed.
            ("units.asset", b"entity = { name = stormtrooper"),
            ("storm.asset", b"entity = { name = storm clone = base }"),
            ("late.asset", b"entity = { name = base scale = 3 }"),
        ]);
        let entities =
            weather_descriptors(
                &paths,
                vec!["storm".into()],
                |path| Ok(files[path].to_vec()),
            )
            .unwrap();
        assert_eq!(entities.entities.len(), 2);
        assert_eq!(entities.inherit("storm").unwrap().scale, Some(3.0));
        assert!(
            weather_descriptors(&paths[1..2], vec!["stormtrooper".into()], |path| {
                Ok(files[path].to_vec())
            })
            .is_err()
        );
    }
}
