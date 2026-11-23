
use std::collections::{BTreeMap, BTreeSet};

use content_reader_map::movement::ProvinceTerrain;
use content_reader_key_names::terrain::TerrainDefinitions;
use content_reader_regions::regions::StrategicRegion;

use map_display_data::landscape::{ModeLayer, Style};
use super::pack;

pub const TERRAIN_GRADIENT: u8 = 127;
const LAND: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defines {
    pub alpha: [f32; 2],
    pub channel: [f32; 2],
}

pub struct Sources<'a> {
    pub categories: &'a TerrainDefinitions,
    pub provinces: &'a [u32],
    pub terrain: &'a dyn Fn(u32) -> Option<ProvinceTerrain>,
    pub region_of: &'a BTreeMap<u32, u32>,
    pub regions: &'a BTreeMap<u32, StrategicRegion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub layer: ModeLayer,
    pub warnings: Vec<String>,
}

fn category(
    sources: &Sources,
    province: u32,
    terrain: ProvinceTerrain,
) -> Result<(usize, bool), String> {
    let database = sources.categories;
    if terrain.flags & LAND != 0 {
        return match usize::try_from(terrain.index) {
            Ok(index) if index >= 1 && index <= database.categories.len() => Ok((index - 1, true)),
            _ => Err("land provinces on the null terrain".to_owned()),
        };
    }
    let named = sources
        .region_of
        .get(&province)
        .and_then(|region| sources.regions.get(region))
        .and_then(|region| region.naval_terrain.as_deref());
    database
        .naval_category(named)
        .map(|index| (index, false))
        .ok_or_else(|| "sea and lake provinces without a naval terrain, as the terrain database declares no ocean".to_owned())
}

pub fn build(sources: &Sources, defines: &Defines) -> std::io::Result<Built> {
    let len = sources
        .provinces
        .last()
        .map_or(1, |&last| last as usize + 1);
    let mut region_of = vec![0; len];
    let mut styles = BTreeMap::new();
    let mut refused = BTreeMap::<String, BTreeSet<u32>>::new();
    for &province in sources.provinces {
        let Some(terrain) = (sources.terrain)(province) else {
            refused
                .entry("provinces without a terrain record".to_owned())
                .or_default()
                .insert(province);
            continue;
        };
        let shown = category(sources, province, terrain).and_then(|(index, land)| {
            let category = &sources.categories.categories[index];
            let mut color = category.color()?;
            color[3] = defines.alpha[usize::from(!land)];
            Ok(color)
        });
        match shown {
            Ok(color) => {
                region_of[province as usize] = province;
                styles.insert(
                    province,
                    Style {
                        color: pack(color),
                        pulse: false,
                        fill: 1.0,
                    },
                );
            }
            Err(why) => {
                refused.entry(why).or_default().insert(province);
            }
        }
    }
    let warnings = refused
        .into_iter()
        .map(|(why, provinces)| {
            format!(
                "The terrain map mode leaves {} provinces unfilled: {why}",
                provinces.len()
            )
        })
        .collect();
    Ok(Built {
        layer: ModeLayer {
            region_of: region_of.into(),
            styles: map_display_data::value_index::ValueIndex::try_from_iter(styles)?,
            thickness: Some(0.0),
            gradient: Some(TERRAIN_GRADIENT),
            channel: defines.channel,
        },
        warnings,
    })
}

#[cfg(test)]
mod tests;
