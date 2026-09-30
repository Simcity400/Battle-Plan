//! The map's weather effect entities from `common/weather.txt`, resolved to
//! their particles.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{all, block, boolean, invalid, last, require, text},
    mesh::{self, Descriptors},
    parser,
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

fn parse_effect_names(bytes: &[u8]) -> io::Result<Names> {
    let document = parser::parse(bytes)?;
    let effects = block(
        require(&document.items, "weather_effects", "weather.txt")?,
        "weather_effects",
    )?;
    let mut out = Names::new();
    for (kind, key) in EFFECT_KINDS.iter().enumerate() {
        for list in all(effects, key.as_bytes()) {
            for value in all(block(list, key)?, b"weather_effect") {
                let fields = block(value, "weather_effect")?;
                let name = |key: &str| -> io::Result<Option<String>> {
                    Ok(match last(fields, key.as_bytes()) {
                        Some(value) => Some(text(value, key)?.to_owned()).filter(|v| !v.is_empty()),
                        None => None,
                    })
                };
                let pair = [name("entity")?, name("fading_entity")?];
                let small = match last(fields, b"small") {
                    Some(value) => boolean(value, "weather_effect small")?,
                    None => false,
                };
                // `140f0b050` keeps a pair only when an entity resolved.
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
