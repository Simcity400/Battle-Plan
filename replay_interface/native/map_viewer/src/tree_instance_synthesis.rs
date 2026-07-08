use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// PAN2 tree draw decode: the captured mesh is slot 0, stride 56, with
// 120 vertices and 216 R16 indices. The JSON contains DrawIndexedInstanced
// 216 x 37, but no instance-buffer payload; the tree VS signature requires
// generated slot 1 as TEXCOORD4=float4(pos.x, pos.y, pos.z, y_rotation) and
// TEXCOORD5=float3(slopes), stride 28. trees.bmp is 1650x600 and scales to
// the 5632x2048 world by cell centers.
pub const TREE_INSTANCE_STRIDE_BYTES: u32 = 28;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeInstanceSynthesisStats {
    pub schema: String,
    pub tree_map_width: usize,
    pub tree_map_height: usize,
    pub map_width: usize,
    pub map_height: usize,
    pub instance_count: usize,
    pub instance_stride_bytes: u32,
    pub tree_index_counts: BTreeMap<u8, usize>,
    pub convention: String,
    pub captured_anchor_status: String,
}

#[derive(Clone, Debug)]
pub struct TreeInstanceSynthesis {
    pub instances: Vec<u8>,
    pub stats: TreeInstanceSynthesisStats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeCaptureDecodeReport {
    pub schema: String,
    pub draw_count: u32,
    pub per_instance_count: Option<u32>,
    pub instance_count: Option<u32>,
    pub mesh_vertex_stride: Option<u32>,
    pub mesh_vertex_count: usize,
    pub index_count: usize,
    pub instance_buffer_status: String,
    pub convention: String,
}

impl MapLayerSynthesisInputs {
    pub(crate) fn load_trees(&self) -> Result<IndexImage> {
        load_tree_raw_index_image(
            &self.trees_path,
            self.tree_width,
            self.tree_height,
            "trees.r8",
        )
    }
}

pub fn synthesize_tree_instances(
    inputs: &MapLayerSynthesisInputs,
) -> Result<TreeInstanceSynthesis> {
    let trees = inputs.load_trees()?;
    let heightmap = inputs.load_heightmap()?;
    synthesize_tree_instances_from_images(&trees, &heightmap, inputs.map_width, inputs.map_height)
}

pub fn synthesize_tree_instances_from_resolver(
    resolver: &GameFileResolver,
    map_width: usize,
    map_height: usize,
) -> Result<TreeInstanceSynthesis> {
    let trees_path = resolver.resolve_required(TREES_BMP_SOURCE, "trees.bmp")?;
    let heightmap_path = resolver.resolve_required(TREE_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
    let trees = load_bmp_indexed(&trees_path)?;
    let heightmap = load_bmp_indexed(&heightmap_path)?;
    synthesize_tree_instances_from_images(&trees, &heightmap, map_width, map_height)
}

pub(crate) fn synthesize_tree_instances_from_images(
    trees: &IndexImage,
    heightmap: &IndexImage,
    map_width: usize,
    map_height: usize,
) -> Result<TreeInstanceSynthesis> {
    if heightmap.width != map_width || heightmap.height != map_height {
        return Err(format!(
            "tree instance synthesis blocked: heightmap is {}x{}, expected {}x{}",
            heightmap.width, heightmap.height, map_width, map_height
        )
        .into());
    }
    let mut instances = Vec::new();
    let mut counts = BTreeMap::new();
    for y in 0..trees.height {
        for x in 0..trees.width {
            let tree_index = trees.pixels[y * trees.width + x];
            if tree_index == 0 {
                continue;
            }
            *counts.entry(tree_index).or_insert(0) += 1;
            let world_x = (x as f32 + 0.5) * map_width as f32 / trees.width as f32;
            let world_z =
                map_height as f32 - (y as f32 + 0.5) * map_height as f32 / trees.height as f32;
            let world_y = sample_tree_height(heightmap, world_x, world_z);
            let rotation = deterministic_tree_rotation(x as u32, y as u32, tree_index);
            let (slope_x, slope_z) = sample_tree_slope(heightmap, world_x, world_z);
            for value in [world_x, world_y, world_z, rotation, slope_x, slope_z, 0.0] {
                instances.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let instance_count = instances.len() / TREE_INSTANCE_STRIDE_BYTES as usize;
    Ok(TreeInstanceSynthesis {
        instances,
        stats: TreeInstanceSynthesisStats {
            schema: "battle-plan-tree-instance-synthesis-v1".to_string(),
            tree_map_width: trees.width,
            tree_map_height: trees.height,
            map_width,
            map_height,
            instance_count,
            instance_stride_bytes: TREE_INSTANCE_STRIDE_BYTES,
            tree_index_counts: counts,
            convention: format!("one nonzero map/trees.bmp cell -> one instance; position=(cell_center scaled to {map_width}x{map_height}, heightmap*0.1, flipped z); rotation=stable cell hash; slopes=heightmap gradient"),
            captured_anchor_status: "PAN2 tree bundle contains the 120-vertex template mesh and DrawIndexedInstanced 216x37 args, but no separate instance vertex buffer or nonzero per-instance records; exact 37-instance anchor is not reconstructable from this bundle".to_string(),
        },
    })
}

#[allow(dead_code)]
pub(crate) fn decode_captured_tree_template(
    bundle_dir: &std::path::Path,
    draw: &MapFrameDraw,
) -> Result<TreeCaptureDecodeReport> {
    let vertex_binding = draw
        .vertex_buffers
        .get("0")
        .ok_or("tree capture decode blocked: captured tree draw has no IA vertex buffer slot 0")?;
    let vertex_bytes = read_map_frame_buffer_bytes(bundle_dir, vertex_binding, "tree vertices")?;
    let mesh_vertex_count = if let Some(stride) = vertex_binding.stride {
        vertex_bytes.len() / stride as usize
    } else {
        0
    };
    let index_count = draw
        .index_buffer
        .as_ref()
        .map(|binding| binding.byte_len / 2)
        .unwrap_or(0);
    let instance_buffer_status = if draw.vertex_buffers.keys().any(|slot| slot != "0") {
        "captured instance vertex buffer present".to_string()
    } else {
        "missing: active tree draw has only IA slot 0; generated instances must be supplied by replay".to_string()
    };
    Ok(TreeCaptureDecodeReport {
        schema: "battle-plan-tree-capture-decode-v1".to_string(),
        draw_count: draw.draw_count,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        mesh_vertex_stride: vertex_binding.stride,
        mesh_vertex_count,
        index_count,
        instance_buffer_status,
        convention: "captured slot 0 is 120 template vertices at 56 bytes: float3 position, float3 normal, float4 tangent, float2 uv0, float2 uv1; replay adds slot 1 float4 pos_yrot + float3 slopes".to_string(),
    })
}

fn deterministic_tree_rotation(x: u32, y: u32, tree_index: u8) -> f32 {
    let mut value = x.wrapping_mul(0x9E37_79B1).rotate_left(5)
        ^ y.wrapping_mul(0x85EB_CA6B).rotate_left(13)
        ^ u32::from(tree_index).wrapping_mul(0xC2B2_AE35);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^= value >> 16;
    (value as f32 / u32::MAX as f32) * std::f32::consts::TAU
}

fn sample_tree_slope(heightmap: &IndexImage, x: f32, z: f32) -> (f32, f32) {
    let left = sample_tree_height(heightmap, x - 1.0, z);
    let right = sample_tree_height(heightmap, x + 1.0, z);
    let down = sample_tree_height(heightmap, x, z - 1.0);
    let up = sample_tree_height(heightmap, x, z + 1.0);
    ((right - left) * 0.5, (up - down) * 0.5)
}

fn sample_tree_height(heightmap: &IndexImage, x: f32, z: f32) -> f32 {
    let xi = x.round().clamp(0.0, (heightmap.width - 1) as f32) as usize;
    let yi = ((heightmap.height - 1) as f32 - z.round()).clamp(0.0, (heightmap.height - 1) as f32)
        as usize;
    heightmap.pixels[yi * heightmap.width + xi] as f32 * 0.1
}

fn load_tree_raw_index_image(
    path: &std::path::Path,
    width: usize,
    height: usize,
    label: &str,
) -> Result<IndexImage> {
    let pixels = std::fs::read(path).map_err(|err| {
        format!(
            "tree instance synthesis blocked: cannot read {} at {}: {err}",
            label,
            path.display()
        )
    })?;
    let expected = width
        .checked_mul(height)
        .ok_or("tree instance synthesis blocked: image dimensions overflow")?;
    if pixels.len() != expected {
        return Err(format!(
            "tree instance synthesis blocked: {} has {} bytes, expected {} for {}x{}",
            path.display(),
            pixels.len(),
            expected,
            width,
            height
        )
        .into());
    }
    Ok(IndexImage {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_instances_scale_tree_cells_to_world_space() {
        let trees = IndexImage {
            width: 2,
            height: 1,
            pixels: vec![0, 5],
        };
        let heightmap = IndexImage {
            width: 4,
            height: 4,
            pixels: vec![100; 16],
        };
        let generated =
            synthesize_tree_instances_from_images(&trees, &heightmap, 4, 4).expect("instances");
        assert_eq!(generated.stats.instance_count, 1);
        assert_eq!(
            generated.instances.len(),
            TREE_INSTANCE_STRIDE_BYTES as usize
        );
        let x = f32::from_le_bytes(generated.instances[0..4].try_into().unwrap());
        let y = f32::from_le_bytes(generated.instances[4..8].try_into().unwrap());
        let z = f32::from_le_bytes(generated.instances[8..12].try_into().unwrap());
        assert_eq!(x, 3.0);
        assert_eq!(y, 10.0);
        assert_eq!(z, 2.0);
    }

    #[test]
    fn tree_instance_generation_is_deterministic() {
        let trees = IndexImage {
            width: 3,
            height: 2,
            pixels: vec![0, 5, 0, 6, 0, 6],
        };
        let heightmap = IndexImage {
            width: 6,
            height: 4,
            pixels: (0..24).map(|value| value as u8).collect(),
        };
        let first =
            synthesize_tree_instances_from_images(&trees, &heightmap, 6, 4).expect("first pass");
        let second =
            synthesize_tree_instances_from_images(&trees, &heightmap, 6, 4).expect("second pass");
        assert_eq!(first.instances, second.instances);
        assert_eq!(first.stats, second.stats);
    }
}
