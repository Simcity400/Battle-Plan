//! The terrain mode's own layer: the game's "simplified colours" of each province's terrain.
//!
//! Every province of the census is a region of its own, so the layer never changes after
//! the content loads. A land province (definition flag 1) takes its definition's terrain
//! category; any other province, sea or lake, takes its strategic region's `naval_terrain`
//! category, or `ocean` when it has no region or the region names no category the database
//! holds. The region's colour is the category's `color` with its alpha replaced by
//! `MAP_MODE_TERRAIN_TRANSPARENCY` on land and `MAP_MODE_NAVAL_TERRAIN_TRANSPARENCY`
//! elsewhere. Every region writes the constant gradient [`TERRAIN_GRADIENT`] with no naval
//! effects, a zero thickness and an unchanged fill, and the channel takes
//! `GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_TERRAIN` with the state layer's outline
//! cutoff.
//!
//! A province the census has no terrain record for, a land province on the null terrain, a
//! naval province when the database has no `ocean`, and a category whose colour does not
//! read are left out of the layer and counted by reason.

use std::collections::{BTreeMap, BTreeSet};

use installed_content::{movement::ProvinceTerrain, regions::StrategicRegion, terrain::TerrainDefinitions};

use super::{ModeLayer, Style, pack};

/// The constant gradient byte every terrain region writes.
pub const TERRAIN_GRADIENT: u8 = 127;
/// The definition flag of a land province.
const LAND: u8 = 1;

/// The terrain mode's defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defines {
    /// `MAP_MODE_TERRAIN_TRANSPARENCY` and `MAP_MODE_NAVAL_TERRAIN_TRANSPARENCY`.
    pub alpha: [f32; 2],
    /// `GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_TERRAIN` and
    /// `GRADIENT_BORDERS_OUTLINE_CUTOFF_STATE`.
    pub channel: [f32; 2],
}

/// The content the layer is built from.
pub struct Sources<'a> {
    /// The terrain database.
    pub categories: &'a TerrainDefinitions,
    /// Every province of the census, ascending.
    pub provinces: &'a [u32],
    /// A province's definition flags and terrain category ordinal; `None` when its record
    /// was refused.
    pub terrain: &'a dyn Fn(u32) -> Option<ProvinceTerrain>,
    /// A province's strategic region.
    pub region_of: &'a BTreeMap<u32, u32>,
    pub regions: &'a BTreeMap<u32, StrategicRegion>,
}

/// The terrain layer and what it leaves out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub layer: ModeLayer,
    /// One line per reason, with its province count.
    pub warnings: Vec<String>,
}

/// The category a province shows and whether it is land.
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

/// Builds the layer from the content.
pub fn build(sources: &Sources, defines: &Defines) -> Built {
    let len = sources.provinces.last().map_or(1, |&last| last as usize + 1);
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
    Built {
        layer: ModeLayer {
            region_of,
            styles,
            thickness: Some(0.0),
            gradient: Some(TERRAIN_GRADIENT),
            channel: defines.channel,
        },
        warnings,
    }
}

#[cfg(test)]
mod tests;
