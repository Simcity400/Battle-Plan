
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const POSTEFFECT_REGION_WEIGHT_SCHEMA: &str = "hoi4.posteffect-region-weights.v1";
pub const POSTEFFECT_REGION_DEFINITION_SOURCE: &str = "map/definition.csv";
pub const POSTEFFECT_REGION_PROVINCES_SOURCE: &str = "map/provinces.bmp";
pub const POSTEFFECT_REGION_STATES_SOURCE: &str = "history/states";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostEffectRegionBoxInput {
    pub declaration_ordinal: usize,
    pub name: String,
    pub position: [f32; 3],
    pub size: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionProvinceWeight {
    pub province_id: u32,
    pub weight: f32,
    pub source_pixel_count: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostEffectRegionWeightSet {
    pub declaration_ordinal: usize,
    pub name: String,
    pub position: [f32; 3],
    pub size: [f32; 3],
    pub rows: Vec<RegionProvinceWeight>,
    pub total_source_pixel_count: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostEffectRegionWeightManifest {
    pub schema: String,
    pub definition_csv: ResolvedGameFile,
    pub provinces_bmp: ResolvedGameFile,
    pub history_states: Vec<ResolvedGameFile>,
    pub map_width: usize,
    pub map_height: usize,
    pub regions: Vec<PostEffectRegionWeightSet>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProvinceRasterMetrics {
    pixel_count: i32,
    min_x: usize,
    min_world_z: usize,
    max_x_exclusive: usize,
    max_world_z_exclusive: usize,
}

impl ProvinceRasterMetrics {
    fn at(x: usize, world_z: usize) -> Self {
        Self {
            pixel_count: 1,
            min_x: x,
            min_world_z: world_z,
            max_x_exclusive: x + 1,
            max_world_z_exclusive: world_z + 1,
        }
    }

    fn include(&mut self, x: usize, world_z: usize) -> Result<()> {
        self.pixel_count = self
            .pixel_count
            .checked_add(1)
            .ok_or("province raster pixel count exceeds the engine signed-i32 field")?;
        self.min_x = self.min_x.min(x);
        self.min_world_z = self.min_world_z.min(world_z);
        self.max_x_exclusive = self.max_x_exclusive.max(x + 1);
        self.max_world_z_exclusive = self.max_world_z_exclusive.max(world_z + 1);
        Ok(())
    }

    fn even_rounded_bounds(self) -> [f32; 4] {
        [
            floor_to_even(self.min_x) as f32,
            floor_to_even(self.min_world_z) as f32,
            ceil_to_even(self.max_x_exclusive) as f32,
            ceil_to_even(self.max_world_z_exclusive) as f32,
        ]
    }
}

pub fn posteffect_region_weights_from_resolver(
    resolver: &GameFileResolver,
    region_boxes: &[PostEffectRegionBoxInput],
) -> Result<PostEffectRegionWeightManifest> {
    let definition_csv = resolver.resolve_required_with_provenance(
        POSTEFFECT_REGION_DEFINITION_SOURCE,
        "posteffect province definitions",
    )?;
    let provinces_bmp = resolver.resolve_required_with_provenance(
        POSTEFFECT_REGION_PROVINCES_SOURCE,
        "posteffect province raster",
    )?;

    let (definitions, color_to_id) = load_definition_csv(&definition_csv.local_path)?;
    let raster = load_provinces_bmp(&provinces_bmp.local_path, &color_to_id)?;
    if raster.unknown_pixels != 0 {
        return Err(format!(
            "{} contains {} pixels absent from {}",
            provinces_bmp.local_path.display(),
            raster.unknown_pixels,
            definition_csv.local_path.display()
        )
        .into());
    }

    let enumerated_state_files =
        resolver.resolve_dir_files(POSTEFFECT_REGION_STATES_SOURCE, Some("txt"))?;
    let history_states = resolver.resolve_used_content(
        enumerated_state_files
            .iter()
            .map(|(logical_path, _)| logical_path.clone()),
    )?;
    let state_paths = history_states
        .iter()
        .map(|source| (source.logical_path.clone(), source.local_path.clone()))
        .collect::<Vec<_>>();
    let (states, province_to_state, _state_owners) = load_states(&state_paths)?;
    let passable_land = definitions
        .iter()
        .filter_map(|(&province_id, definition)| {
            if definition.province_type != "land" {
                return None;
            }
            let state_id = province_to_state.get(&province_id)?;
            let state = states.get(state_id)?;
            (!state.impassable).then_some(province_id)
        })
        .collect::<BTreeSet<_>>();

    let metrics =
        province_raster_metrics(&raster.ids, raster.width, raster.height, &passable_land)?;
    let regions = region_boxes
        .iter()
        .map(|region| reproduce_region_rows(region, &metrics))
        .collect::<Result<Vec<_>>>()?;

    Ok(PostEffectRegionWeightManifest {
        schema: POSTEFFECT_REGION_WEIGHT_SCHEMA.to_string(),
        definition_csv,
        provinces_bmp,
        history_states,
        map_width: raster.width,
        map_height: raster.height,
        regions,
    })
}

fn province_raster_metrics(
    ids_top_down: &[u32],
    width: usize,
    height: usize,
    passable_land: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, ProvinceRasterMetrics>> {
    let expected_len = width
        .checked_mul(height)
        .ok_or("province raster dimensions overflow")?;
    if ids_top_down.len() != expected_len {
        return Err(format!(
            "province raster length mismatch: expected {expected_len}, got {}",
            ids_top_down.len()
        )
        .into());
    }

    let mut metrics = BTreeMap::<u32, ProvinceRasterMetrics>::new();
    for (offset, &province_id) in ids_top_down.iter().enumerate() {
        if province_id == 0 || !passable_land.contains(&province_id) {
            continue;
        }
        let x = offset % width;
        let bmp_y = offset / width;
        let world_z = height - 1 - bmp_y;
        match metrics.get_mut(&province_id) {
            Some(metric) => metric.include(x, world_z)?,
            None => {
                metrics.insert(province_id, ProvinceRasterMetrics::at(x, world_z));
            }
        }
    }
    Ok(metrics)
}

fn reproduce_region_rows(
    region: &PostEffectRegionBoxInput,
    metrics: &BTreeMap<u32, ProvinceRasterMetrics>,
) -> Result<PostEffectRegionWeightSet> {
    if region
        .position
        .iter()
        .chain(region.size.iter())
        .any(|value| !value.is_finite())
    {
        return Err(format!(
            "posteffect region {:?} has a non-finite position or size",
            region.name
        )
        .into());
    }

    let half_size_x = region.size[0] * 0.5_f32;
    let half_size_z = region.size[2] * 0.5_f32;
    let box_min_x = region.position[0] - half_size_x;
    let box_max_x = region.position[0] + half_size_x;
    let box_min_z = region.position[2] - half_size_z;
    let box_max_z = region.position[2] + half_size_z;

    let mut contributions = Vec::<(u32, i32)>::new();
    let mut total_source_pixel_count = 0_i32;
    for (&province_id, metric) in metrics {
        let [min_x, min_z, max_x, max_z] = metric.even_rounded_bounds();
        if min_x < box_max_x && box_min_x < max_x && min_z < box_max_z && box_min_z < max_z {
            total_source_pixel_count = total_source_pixel_count
                .checked_add(metric.pixel_count)
                .ok_or("posteffect region pixel total exceeds the engine signed-i32 field")?;
            contributions.push((province_id, metric.pixel_count));
        }
    }

    let rows = if total_source_pixel_count == 0 {
        Vec::new()
    } else {
        contributions
            .into_iter()
            .map(|(province_id, source_pixel_count)| RegionProvinceWeight {
                province_id,
                weight: engine_normalized_weight(source_pixel_count, total_source_pixel_count),
                source_pixel_count,
            })
            .collect()
    };

    Ok(PostEffectRegionWeightSet {
        declaration_ordinal: region.declaration_ordinal,
        name: region.name.clone(),
        position: region.position,
        size: region.size,
        rows,
        total_source_pixel_count,
    })
}

fn floor_to_even(value: usize) -> usize {
    value & !1
}

fn ceil_to_even(value: usize) -> usize {
    (value + 1) & !1
}

fn engine_normalized_weight(contribution: i32, total: i32) -> f32 {
    let ratio = contribution as f32 / total as f32;
    if ratio >= 0.0_f32 {
        ratio.min(1.0_f32)
    } else {
        0.0_f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(position: [f32; 3], size: [f32; 3]) -> PostEffectRegionBoxInput {
        PostEffectRegionBoxInput {
            declaration_ordinal: 7,
            name: "test_region".to_string(),
            position,
            size,
        }
    }

    #[test]
    fn reproduces_full_counts_even_bounds_flip_and_ascending_rows() {
        let ids = vec![9, 9, 0, 0, 2, 0, 0, 0, 2, 2, 2, 0];
        let metrics = province_raster_metrics(&ids, 4, 3, &BTreeSet::from([2, 9])).unwrap();

        let rows =
            reproduce_region_rows(&region([1.5, 0.0, 2.0], [0.2, 1.0, 0.2]), &metrics).unwrap();
        assert_eq!(rows.total_source_pixel_count, 6);
        assert_eq!(rows.rows.len(), 2);
        assert_eq!(rows.rows[0].province_id, 2);
        assert_eq!(rows.rows[0].source_pixel_count, 4);
        assert_eq!(rows.rows[0].weight.to_bits(), (4.0_f32 / 6.0_f32).to_bits());
        assert_eq!(rows.rows[1].province_id, 9);
        assert_eq!(rows.rows[1].source_pixel_count, 2);
        assert_eq!(rows.rows[1].weight.to_bits(), (2.0_f32 / 6.0_f32).to_bits());
    }

    #[test]
    fn strict_half_open_overlap_rejects_edge_only_contact() {
        let ids = vec![1_u32];
        let metrics = province_raster_metrics(&ids, 1, 1, &BTreeSet::from([1])).unwrap();
        let edge =
            reproduce_region_rows(&region([2.5, 0.0, 1.0], [1.0, 1.0, 1.0]), &metrics).unwrap();
        assert!(edge.rows.is_empty());
        let overlap =
            reproduce_region_rows(&region([1.999, 0.0, 1.0], [0.002, 1.0, 1.0]), &metrics).unwrap();
        assert_eq!(overlap.rows[0].province_id, 1);
        assert_eq!(overlap.rows[0].weight.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn excludes_graphics_without_passable_land_bit() {
        let ids = vec![1_u32, 2_u32, 3_u32];
        let metrics = province_raster_metrics(&ids, 3, 1, &BTreeSet::from([2])).unwrap();
        assert_eq!(metrics.keys().copied().collect::<Vec<_>>(), vec![2]);
    }
}
