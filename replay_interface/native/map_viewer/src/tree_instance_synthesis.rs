use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// CAPTURE (mined/hoi4_frame54162, chunk 14646, IA slot 1 buffer 131227,
// pStrides[0] = 24) closes the actual tree-instance IA stride. TEXCOORD4 is
// float4(pos.xyz,y_rotation) and TEXCOORD5 is float2(slopes). The previous
// 28-byte interpretation came from the HLSL declaration rather than IA state.
pub const TREE_INSTANCE_STRIDE_BYTES: u32 = 24;
pub const TREE_INSTANCE_GENERATOR_OMISSION: &str =
    "tree_instance_generator_yaw_and_slope_unresolved";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedTreeInstance {
    pub anchor: [f32; 3],
    pub yaw_radians: f32,
    pub slopes: [f32; 2],
}

pub fn decode_captured_tree_instance(bytes: &[u8]) -> Result<CapturedTreeInstance> {
    if bytes.len() != TREE_INSTANCE_STRIDE_BYTES as usize {
        return Err(format!(
            "tree CAP9 decode blocked: expected {} bytes, got {}",
            TREE_INSTANCE_STRIDE_BYTES,
            bytes.len()
        )
        .into());
    }
    let values: [f32; 6] = std::array::from_fn(|index| {
        f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    });
    if values.iter().any(|value| !value.is_finite()) {
        return Err("tree CAP9 decode blocked: instance contains a non-finite float".into());
    }
    Ok(CapturedTreeInstance {
        anchor: [values[0], values[1], values[2]],
        yaw_radians: values[3],
        slopes: [values[4], values[5]],
    })
}

/// Bit-order-preserving X/Z lattice recovered from all 3,985 retained CAP9
/// records. Y, yaw, and slopes are deliberately outside this law.
pub fn tree_cell_anchor_xz(
    cell_x: usize,
    cell_y: usize,
    tree_width: usize,
    tree_height: usize,
    map_width: usize,
    map_height: usize,
) -> Result<[f32; 2]> {
    if tree_width == 0 || tree_height == 0 || map_width == 0 || map_height == 0 {
        return Err("tree CAP9 anchor blocked: zero-sized tree or world map".into());
    }
    if cell_x >= tree_width || cell_y >= tree_height {
        return Err(format!(
            "tree CAP9 anchor blocked: cell ({cell_x},{cell_y}) outside {tree_width}x{tree_height}"
        )
        .into());
    }
    let sx = map_width as f32 / tree_width as f32;
    let sz = map_height as f32 / tree_height as f32;
    let stagger = if cell_y & 1 == 0 { 0.0 } else { 0.5 };
    Ok([
        cell_x as f32 * sx + stagger * sx,
        (tree_height - cell_y - 1) as f32 * sz,
    ])
}

/// SCATTER LAW (MAP-OBJECTS lane, 2026-07-09, capture-proven 8/8 exact):
/// the engine builds tree instance buffers per 256x256-world-unit chunk
/// (= 75x75 trees.bmp cells at the 5632x2048 / 1650x600 vanilla scale) and
/// issues one DrawIndexedInstanced per (visible chunk, tree palette index)
/// with instance count = number of nonzero cells of that index in the chunk.
/// Evidence: live_trees_measure trees_pass (8 draws) — model mapobject_5
/// (forest_dunge_blend, LOD by camera height 348.07 -> sisataShape) drew
/// 37/86/452/296 instances and mapobject_6 (forest_dunge -> shape
/// polySurfaceShape875) drew 69/141/783/416, which equal the trees.bmp
/// per-chunk counts of palette index 5 / index 6 for chunks
/// (10,6),(10,5),(11,6),(11,5) exactly (all 8 numbers), and those four
/// chunks are exactly the CB0 camera frustum's ground footprint. Cross-check:
/// live_PAN2's DrawIndexedInstanced 216x37 = index-5 count 37 of chunk (10,6)
/// at camera height 270.39 (blend LOD slot 2, loddist [200,300)).
/// Captured draw order: chunk x ascending, then chunk z descending.
///
/// Consumed by the scatter-law gate; wired into the product tree layer once a
/// captured instance VB grounds intra-chunk jitter/rotation.
#[allow(dead_code)]
pub const TREE_CHUNK_WORLD_UNITS: f32 = 256.0;

/// Per-chunk, per-palette-index nonzero trees.bmp cell counts under the
/// [`TREE_CHUNK_WORLD_UNITS`] chunk law. Key = (chunk_x, chunk_z, tree_index)
/// with chunk_z in world coordinates (z = 0 at the map's south edge, i.e.
/// the same flipped-z convention as instance synthesis).
#[allow(dead_code)]
pub fn tree_chunk_index_counts(
    trees: &IndexImage,
    map_width: usize,
    map_height: usize,
) -> BTreeMap<(u32, u32, u8), usize> {
    let mut counts = BTreeMap::new();
    let sx = map_width as f32 / trees.width as f32;
    let sz = map_height as f32 / trees.height as f32;
    for y in 0..trees.height {
        for x in 0..trees.width {
            let tree_index = trees.pixels[y * trees.width + x];
            if tree_index == 0 {
                continue;
            }
            let world_x = (x as f32 + 0.5) * sx;
            let world_z = map_height as f32 - (y as f32 + 0.5) * sz;
            let chunk_x = (world_x / TREE_CHUNK_WORLD_UNITS) as u32;
            let chunk_z = (world_z / TREE_CHUNK_WORLD_UNITS) as u32;
            *counts.entry((chunk_x, chunk_z, tree_index)).or_insert(0) += 1;
        }
    }
    counts
}

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
    let candidate_count = trees.pixels.iter().filter(|value| **value != 0).count();
    Err(format!(
        "{TREE_INSTANCE_GENERATOR_OMISSION}: {candidate_count} trees.bmp cells have exact palette, staggered X/Z, and height inputs, but the captured cell-to-yaw and slope producers remain unresolved; refusing to synthesize a 24-byte instance payload"
    )
    .into())
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
        convention: "captured slot 0 is 120 template vertices at 56 bytes; captured slot 1 records are 24 bytes: float4 pos_yrot + float2 slopes".to_string(),
    })
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
    fn captured_tree_instance_stride_and_decode_are_exact() {
        let values = [3.0f32, 10.0, 2.0, 1.134_464, 0.048_976, -0.125_650_3];
        let bytes = values
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let decoded = decode_captured_tree_instance(&bytes).expect("captured instance");
        assert_eq!(TREE_INSTANCE_STRIDE_BYTES, 24);
        assert_eq!(decoded.anchor, [3.0, 10.0, 2.0]);
        assert_eq!(decoded.yaw_radians.to_bits(), 1.134_464f32.to_bits());
        assert_eq!(decoded.slopes[0].to_bits(), 0.048_976f32.to_bits());
        assert_eq!(decoded.slopes[1].to_bits(), (-0.125_650_3f32).to_bits());
    }

    #[test]
    fn tree_anchor_uses_engine_arithmetic_order_and_stagger() {
        assert_eq!(tree_cell_anchor_xz(1, 0, 4, 2, 8, 4).unwrap(), [2.0, 2.0]);
        assert_eq!(tree_cell_anchor_xz(1, 1, 4, 2, 8, 4).unwrap(), [3.0, 0.0]);
    }

    #[test]
    fn tree_generation_fails_closed_without_yaw_and_slope_producers() {
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
        let error = synthesize_tree_instances_from_images(&trees, &heightmap, 4, 4)
            .expect_err("unproven generators must fail closed");
        assert!(error.to_string().contains(TREE_INSTANCE_GENERATOR_OMISSION));
    }
}
