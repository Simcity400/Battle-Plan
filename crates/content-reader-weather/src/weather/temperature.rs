
use std::{collections::BTreeMap, io};

use replay_data::{
    temperature::Content, temperature::Province, temperature::Settings,
    temperature::TerrainModifier,
};

use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    map::DefinitionRow,
    province_geometry::ProvinceGeometry,
    province_graph::ProvinceGraph,
    regions::StrategicRegions,
};

pub fn read(layers: &ResolvedLayers) -> io::Result<Content> {
    let (graph, geometry) = ProvinceGraph::read(layers)?;
    read_with(layers, &graph, &geometry)
}

pub fn read_with(
    layers: &ResolvedLayers,
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
) -> io::Result<Content> {
    let database = crate::movement::terrain::TerrainDatabase::read(layers)?;
    let ordinal = |name: &[u8]| database.ordinal(name);
    let document = super::read_weather(layers)?;
    let settings = settings_from(&document, ordinal)
        .map_err(|error| invalid(format!("{}: {error}", super::PATH)))?;
    let files = crate::terrain::read_default_map(layers)?;
    let path = files.file("definitions")?;
    let rows = crate::map::definition_rows(&layers.read(&path)?)
        .map_err(|error| invalid(format!("{path}: {error}")))?;
    let regions = crate::regions::read(layers)?;
    from_parts(
        |province| {
            graph
                .edges
                .get(&province)
                .map(|edges| edges.iter().map(|&(neighbour, _)| neighbour).collect())
                .unwrap_or_default()
        },
        geometry,
        &rows,
        &regions,
        settings,
        ordinal,
    )
}

#[cfg(test)]
pub(crate) fn settings(bytes: &[u8], ordinal: impl Fn(&[u8]) -> i32) -> io::Result<Settings> {
    let document = super::parse_weather(bytes)?;
    settings_from(&document, ordinal)
}

fn settings_from(document: &content_reader_script::native::Read, ordinal: impl Fn(&[u8]) -> i32) -> io::Result<Settings> {
    let mut settings = Settings::default();
    for item in &document.items {
        if item.keyword_is("terrain_modifiers") {
            settings.terrain.clear();
            for kind in block(&item.value, "weather terrain_modifiers")? {
                if !kind.keyword_is("temperature") {
                    continue;
                }
                for entry in block(&kind.value, "weather terrain modifier kind")? {
                    let mut modifier = TerrainModifier {
                        terrain: -1,
                        value: 0,
                    };
                    for field in block(&entry.value, "weather terrain modifier")? {
                        let scalar = field.value.scalar();
                        if field.keyword_is("type") {
                            modifier.terrain =
                                ordinal(crate::parser::c_string(scalar.unwrap_or_default()));
                        } else if field.keyword_is("value")
                            && let Some(value) = scalar.and_then(crate::fixed5::script_fixed5)
                        {
                            modifier.value = value;
                        }
                    }
                    settings.terrain.push(modifier);
                }
            }
        } else if item.keyword_is("settings") {
            for group in block(&item.value, "weather settings")? {
                let temperature = group.keyword_is("temperature");
                let performance = group.keyword_is("performance");
                if !temperature && !performance {
                    continue;
                }
                for field in block(&group.value, "weather settings group")? {
                    let Some(text) = field.value.scalar() else {
                        continue;
                    };
                    let fixed = crate::fixed5::decimal_fixed5(text);
                    let slot = if temperature {
                        if field.keyword_is("temperature_variation") {
                            &mut settings.variation
                        } else if field.keyword_is("max_temperature_change") {
                            &mut settings.max_change
                        } else if field.keyword_is("temperature_neighbor_smoothing") {
                            &mut settings.smoothing
                        } else if field.keyword_is("temperature_change_neighbor_smoothing") {
                            &mut settings.change_smoothing
                        } else {
                            continue;
                        }
                    } else {
                        if field.keyword_is("provinces_per_update") {
                            settings.provinces_per_update = crate::fixed5::decimal_i32(text);
                        }
                        continue;
                    };
                    *slot = fixed;
                }
            }
        }
    }
    Ok(settings)
}

pub(crate) fn from_parts(
    neighbours: impl Fn(u32) -> Vec<u32>,
    geometry: &ProvinceGeometry,
    rows: &[DefinitionRow],
    regions: &StrategicRegions,
    settings: Settings,
    ordinal: impl Fn(&[u8]) -> i32,
) -> io::Result<Content> {
    let count = geometry.centroids.len() + 1;
    if geometry
        .centroids
        .keys()
        .zip(1..)
        .any(|(&id, position)| id != position)
    {
        return Err(invalid(
            "province records are not numbered by position; weather slots cannot be joined",
        ));
    }
    let exists = |province: u32| geometry.centroids.contains_key(&province);
    let mut region_of = BTreeMap::new();
    for (&id, region) in &regions.regions {
        for province in region.native_provinces(exists) {
            region_of.insert(province, id);
        }
    }
    let null_named = settings
        .terrain
        .iter()
        .any(|modifier| modifier.terrain == 0);
    let provinces = (0..count as u32)
        .map(|id| Province {
            active: id > 0 && geometry.bounds.contains_key(&id),
            neighbours: if id == 0 { Vec::new() } else { neighbours(id) },
            region: region_of.get(&id).copied(),
            terrain: match rows.get(id as usize) {
                Some(row) if usize::from(row.id) != id as usize => Err(format!(
                    "definition.csv line {} declares province {} at record {id}",
                    row.line, row.id
                )),
                Some(row) => match ordinal(&row.terrain) {
                    0 if null_named => Err(format!(
                        "terrain {:?} is the null terrain, whose validity is unread, and a temperature terrain modifier names the null terrain",
                        String::from_utf8_lossy(&row.terrain)
                    )),
                    0 => Ok(None),
                    index => Ok(Some(index)),
                },
                None => Err("the record was invented for an unclaimed colour; its default terrain is unread".into()),
            },
        })
        .collect();
    let periods = regions
        .regions
        .iter()
        .map(|(&id, region)| {
            let periods = match &region.periods_refused {
                Some(reason) => Err(reason.clone()),
                None => Ok(region.periods.clone()),
            };
            (id, periods)
        })
        .collect();
    Ok(Content {
        provinces,
        periods,
        settings,
    })
}

#[cfg(test)]
mod tests;
