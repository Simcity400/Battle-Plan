use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TREE_MESH_VERTEX_STRIDE_BYTES: u32 = 56;
pub const TREES_BMP_SOURCE: &str = "map/trees.bmp";
pub const TREE_HEIGHTMAP_SOURCE: &str = "map/heightmap.bmp";
pub const TREE_DEFAULT_MAP_SOURCE: &str = "map/default.map";
pub const TREE_GFX_BINDING_SOURCE: &str = "gfx/entities/trees.gfx";
pub const TREE_TINT_TEXTURE_SOURCE: &str = "map/terrain/Tree_tint.bmp";
pub const TREE_SEASON_TEXTURE_SOURCE: &str = "map/terrain/Tree_season.bmp";
pub const TREE_SHADER_SOURCE: &str = "gfx/FX/tree.shader";
pub const TREE_SHADER_EFFECT: &str = "tree";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeGfxBinding {
    pub object_index: u8,
    pub name: String,
    pub mesh: String,
    pub scale: String,
    pub source: String,
    pub block_line: usize,
    pub name_line: usize,
    pub file_line: usize,
    pub scale_line: Option<usize>,
}

impl TreeGfxBinding {
    pub fn scale_value(&self) -> Result<f32> {
        self.scale.parse::<f32>().map_err(|err| {
            format!(
                "{}:{} scale for {} is not a float: {err}",
                self.source,
                self.scale_line.unwrap_or(self.block_line),
                self.name
            )
            .into()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeShaderPreparation {
    pub shader_source: String,
    pub effect_name: String,
    pub vertex_hlsl_bytes: usize,
    pub pixel_hlsl_bytes: usize,
    pub runtime_bytecode_source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeFileTexture {
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub bgra8: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeMeshVertexBuffer {
    pub source_mesh: String,
    pub source_binding: TreeGfxBinding,
    pub material_shader: String,
    pub material_diffuse: Option<String>,
    pub material_normal_map: Option<String>,
    pub vertex_stride_bytes: u32,
    pub vertex_count: usize,
    pub index_count: usize,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeFileDerivedLayerStats {
    pub schema: String,
    pub selected_tree_index: u8,
    pub selected_binding: String,
    pub selected_mesh: String,
    pub default_map_tree_indices: Vec<u8>,
    pub tree_index_counts: BTreeMap<u8, usize>,
    pub unbound_tree_index_counts: BTreeMap<u8, usize>,
    pub mesh_vertex_count: usize,
    pub mesh_index_count: usize,
    pub instance_count: usize,
    pub tint_texture: [u32; 2],
    pub season_texture: [u32; 2],
    pub shader: TreeShaderPreparation,
    pub convention: String,
    pub gaps: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TreeFileDerivedLayer {
    pub binding: TreeGfxBinding,
    pub mesh: TreeMeshVertexBuffer,
    pub instances: TreeInstanceSynthesis,
    pub tint: TreeFileTexture,
    pub season: TreeFileTexture,
    pub stats: TreeFileDerivedLayerStats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreePositionDistanceStats {
    pub count: usize,
    pub min: f32,
    pub mean: f32,
    pub p50: f32,
    pub p95: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeInstanceComparisonReport {
    pub schema: String,
    pub status: String,
    pub capture_dir: String,
    pub generated_instance_count: usize,
    pub captured_instance_count: Option<usize>,
    pub captured_draw_instance_count: Option<u32>,
    pub captured_per_instance_count: Option<u32>,
    pub captured_instance_buffer_slot: Option<u32>,
    pub captured_instance_stride_bytes: Option<u32>,
    pub captured_to_generated_world_xyz: Option<TreePositionDistanceStats>,
    pub generated_to_captured_world_xyz: Option<TreePositionDistanceStats>,
    pub note: String,
}

impl TreeInstanceComparisonReport {
    pub fn print(&self) {
        println!(
            "tree instance comparison: status {}, generated {}, captured_buffer {:?}, captured_draw {:?}, per_instance {:?}",
            self.status,
            self.generated_instance_count,
            self.captured_instance_count,
            self.captured_draw_instance_count,
            self.captured_per_instance_count
        );
        if let Some(stats) = &self.captured_to_generated_world_xyz {
            println!(
                "captured->generated world_xyz distance: count {}, min {:.3}, mean {:.3}, p50 {:.3}, p95 {:.3}, max {:.3}",
                stats.count, stats.min, stats.mean, stats.p50, stats.p95, stats.max
            );
        }
        if let Some(stats) = &self.generated_to_captured_world_xyz {
            println!(
                "generated->captured world_xyz distance: count {}, min {:.3}, mean {:.3}, p50 {:.3}, p95 {:.3}, max {:.3}",
                stats.count, stats.min, stats.mean, stats.p50, stats.p95, stats.max
            );
        }
        println!("tree instance comparison note: {}", self.note);
    }
}

pub fn tree_file_derived_layer_from_resolver(
    resolver: &GameFileResolver,
    map_width: usize,
    map_height: usize,
) -> Result<TreeFileDerivedLayer> {
    let instances = synthesize_tree_instances_from_resolver(resolver, map_width, map_height)?;
    let bindings = tree_gfx_bindings_from_resolver(resolver)?;
    let default_map_tree_indices = default_map_tree_indices_from_resolver(resolver)?;
    let binding = select_tree_gfx_binding(&bindings, &instances.stats.tree_index_counts)?;
    let model = pdx_model_from_resolver(resolver, &binding.mesh)?;
    let mesh = tree_mesh_vertex_buffer_from_model(&model, binding.clone())?;
    let tint = tree_texture_from_resolver(resolver, TREE_TINT_TEXTURE_SOURCE)?;
    let season = tree_texture_from_resolver(resolver, TREE_SEASON_TEXTURE_SOURCE)?;
    let shader = prepare_tree_shader_from_resolver(resolver)?;
    let unbound_tree_index_counts = instances
        .stats
        .tree_index_counts
        .iter()
        .filter(|(index, _)| {
            !bindings
                .iter()
                .any(|binding| binding.object_index == **index)
        })
        .map(|(index, count)| (*index, *count))
        .collect::<BTreeMap<_, _>>();
    let gaps = vec![
        // TREES2: not a gap. 6b77b6df is gfx/FX/tree.shader:tree_low_gfx (the
        // LOW_END_GFX variant, proven byte-exact), compiled but bound to no
        // captured draw — the full-detail `tree` family is the rendered path.
        // See tree_second_subdraw.rs.
        "the low-gfx tree pixel shader 6b77b6df9ba8f03d (gfx/FX/tree.shader:tree_low_gfx) is compiled but unused at the captured graphics settings; the full-detail tree family is the rendered path (TREES2)".to_string(),
        "the exact a729 24-byte instance producer is retained by palette/chunk bucket, but this development layer still collapses those batches to one selected mesh draw".to_string(),
        "per-palette model choice, per-frame visible bucket set/order, and mesh LOD choice are not yet carried into the production draw graph".to_string(),
        "tree diffuse/normal material atlas is not replaced yet; D3D runtime keeps the captured tree pass material SRVs".to_string(),
    ];
    let stats = TreeFileDerivedLayerStats {
        schema: "battle-plan-tree-file-derived-layer-v2".to_string(),
        selected_tree_index: binding.object_index,
        selected_binding: format!("{}:{}", binding.source, binding.name_line),
        selected_mesh: binding.mesh.clone(),
        default_map_tree_indices,
        tree_index_counts: instances.stats.tree_index_counts.clone(),
        unbound_tree_index_counts,
        mesh_vertex_count: mesh.vertex_count,
        mesh_index_count: mesh.index_count,
        instance_count: instances.stats.instance_count,
        tint_texture: [tint.width, tint.height],
        season_texture: [season.width, season.height],
        shader,
        gaps,
    };
    Ok(TreeFileDerivedLayer {
        binding,
        mesh,
        instances,
        tint,
        season,
        stats,
    })
}

pub fn tree_gfx_bindings_from_resolver(resolver: &GameFileResolver) -> Result<Vec<TreeGfxBinding>> {
    let path = resolver.resolve_required(TREE_GFX_BINDING_SOURCE, "tree gfx binding")?;
    let text = read_latin1(&path)?;
    parse_tree_gfx_bindings(&text, TREE_GFX_BINDING_SOURCE)
}

pub fn parse_tree_gfx_bindings(text: &str, source: &str) -> Result<Vec<TreeGfxBinding>> {
    let mut bindings = Vec::new();
    let mut block: Vec<(usize, String)> = Vec::new();
    let mut depth = 0i32;
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_no = line_index + 1;
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .to_string();
        if depth == 0 {
            if find_assignment(&line, "pdxmesh", 0).is_none() {
                continue;
            }
            block.clear();
        }
        if depth > 0 || find_assignment(&line, "pdxmesh", 0).is_some() {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            block.push((line_no, line));
            if depth <= 0 {
                bindings.push(parse_tree_gfx_binding_block(&block, source)?);
                block.clear();
                depth = 0;
            }
        }
    }
    if depth != 0 {
        return Err(format!("{source}: unterminated pdxmesh block").into());
    }
    if bindings.is_empty() {
        return Err(format!("{source}: no pdxmesh tree bindings found").into());
    }
    Ok(bindings)
}

pub fn default_map_tree_indices_from_resolver(resolver: &GameFileResolver) -> Result<Vec<u8>> {
    let path = resolver.resolve_required(TREE_DEFAULT_MAP_SOURCE, "default.map tree indices")?;
    let text = strip_comments(&read_latin1(&path)?);
    let (block, _) = extract_block_after(&text, "tree", 0)
        .ok_or_else(|| format!("{TREE_DEFAULT_MAP_SOURCE}: missing tree = {{ ... }} block"))?;
    parse_u32s(&block)
        .into_iter()
        .map(|value| {
            u8::try_from(value).map_err(|_| {
                format!("{TREE_DEFAULT_MAP_SOURCE}: tree palette index {value} exceeds u8").into()
            })
        })
        .collect()
}

pub fn tree_mesh_vertex_buffer_from_model(
    model: &PdxModel,
    binding: TreeGfxBinding,
) -> Result<TreeMeshVertexBuffer> {
    let scale = binding.scale_value()?;
    let mesh = model
        .shapes
        .iter()
        .flat_map(|shape| shape.meshes.iter())
        .find(|mesh| !mesh.material.is_collision() && !mesh.verts.is_empty())
        .ok_or_else(|| {
            format!(
                "tree file layer blocked: {} has no non-collision mesh with vertices",
                binding.mesh
            )
        })?;
    validate_tree_mesh_arrays(mesh, &binding.mesh)?;
    let mut vertices =
        Vec::with_capacity(mesh.verts.len() * TREE_MESH_VERTEX_STRIDE_BYTES as usize);
    for index in 0..mesh.verts.len() {
        let position = mesh.verts[index];
        push_f32s(
            &mut vertices,
            &[
                position[0] * scale,
                position[1] * scale,
                position[2] * scale,
            ],
        );
        push_f32s(
            &mut vertices,
            &mesh
                .normals
                .as_ref()
                .map(|normals| normals[index])
                .unwrap_or([0.0, 1.0, 0.0]),
        );
        push_f32s(
            &mut vertices,
            &mesh
                .tangents
                .as_ref()
                .map(|tangents| tangents[index])
                .unwrap_or([1.0, 0.0, 0.0, 1.0]),
        );
        push_f32s(
            &mut vertices,
            &mesh
                .uv0
                .as_ref()
                .map(|uvs| uvs[index])
                .unwrap_or([0.0, 0.0]),
        );
        push_f32s(
            &mut vertices,
            &mesh
                .uv1
                .as_ref()
                .map(|uvs| uvs[index])
                .unwrap_or([0.0, 0.0]),
        );
    }
    Ok(TreeMeshVertexBuffer {
        source_mesh: binding.mesh.clone(),
        source_binding: binding,
        material_shader: mesh.material.shader.clone(),
        material_diffuse: mesh.material.diffuse.clone(),
        material_normal_map: mesh.material.normal_map.clone(),
        vertex_stride_bytes: TREE_MESH_VERTEX_STRIDE_BYTES,
        vertex_count: mesh.verts.len(),
        index_count: mesh.indices.len(),
        vertices,
        indices: mesh.indices.clone(),
    })
}

pub fn compare_tree_instances_to_capture(
    generated: &TreeInstanceSynthesis,
    capture_dir: &Path,
) -> Result<TreeInstanceComparisonReport> {
    let bundle_path = capture_dir.join(TREES_FRAME_BUNDLE_FILE);
    let bundle = NativeD3d11TerrainBundle::load_trees(&bundle_path)?;
    let draw = bundle
        .frame
        .draws
        .iter()
        .find(|draw| draw.draw_index == bundle.frame.active_draw.draw_index)
        .or_else(|| bundle.frame.draws.first())
        .ok_or("tree instance comparison blocked: tree bundle has no draws")?;
    let generated_positions =
        decode_tree_instance_positions(&generated.instances, TREE_INSTANCE_STRIDE_BYTES as usize)?;
    let captured = captured_instance_positions(&bundle, draw)?;
    let Some(captured) = captured else {
        return Ok(TreeInstanceComparisonReport {
            schema: "battle-plan-tree-instance-comparison-v1".to_string(),
            status: "unavailable".to_string(),
            capture_dir: capture_dir.display().to_string(),
            generated_instance_count: generated_positions.len(),
            captured_instance_count: None,
            captured_draw_instance_count: draw.instance_count,
            captured_per_instance_count: draw.per_instance_count,
            captured_instance_buffer_slot: None,
            captured_instance_stride_bytes: None,
            captured_to_generated_world_xyz: None,
            generated_to_captured_world_xyz: None,
            note: "captured tree draw has no separate IA instance buffer; only DrawIndexedInstanced args are available".to_string(),
        });
    };
    let captured_to_generated = nearest_distance_stats(&captured.positions, &generated_positions);
    let generated_to_captured = nearest_distance_stats(&generated_positions, &captured.positions);
    Ok(TreeInstanceComparisonReport {
        schema: "battle-plan-tree-instance-comparison-v1".to_string(),
        status: "compared".to_string(),
        capture_dir: capture_dir.display().to_string(),
        generated_instance_count: generated_positions.len(),
        captured_instance_count: Some(captured.positions.len()),
        captured_draw_instance_count: draw.instance_count,
        captured_per_instance_count: draw.per_instance_count,
        captured_instance_buffer_slot: Some(captured.slot),
        captured_instance_stride_bytes: Some(captured.stride),
        captured_to_generated_world_xyz: captured_to_generated,
        generated_to_captured_world_xyz: generated_to_captured,
        note: "nearest-neighbor distance uses world xyz from shader slot-1 float4 vPos_YRot first three floats".to_string(),
    })
}

fn prepare_tree_shader_from_resolver(resolver: &GameFileResolver) -> Result<TreeShaderPreparation> {
    let vertex_hlsl = build_stage_hlsl(
        resolver,
        TREE_SHADER_SOURCE,
        TREE_SHADER_EFFECT,
        ShaderStage::Vertex,
    )?;
    let pixel_hlsl = build_stage_hlsl(
        resolver,
        TREE_SHADER_SOURCE,
        TREE_SHADER_EFFECT,
        ShaderStage::Pixel,
    )?;
    Ok(TreeShaderPreparation {
        shader_source: TREE_SHADER_SOURCE.to_string(),
        effect_name: TREE_SHADER_EFFECT.to_string(),
        vertex_hlsl_bytes: vertex_hlsl.len(),
        pixel_hlsl_bytes: pixel_hlsl.len(),
        runtime_bytecode_source:
            "captured tree pass bytecode template; OFF-3 proved tree:tree VS+PS compile byte-exact"
                .to_string(),
    })
}

fn parse_tree_gfx_binding_block(block: &[(usize, String)], source: &str) -> Result<TreeGfxBinding> {
    let block_line = block.first().map(|(line, _)| *line).unwrap_or(1);
    let mut name = None;
    let mut name_line = None;
    let mut mesh = None;
    let mut file_line = None;
    let mut scale = None;
    let mut scale_line = None;
    for (line_no, line) in block {
        if name.is_none() {
            if let Some(value) = parse_quoted_value(line, "name") {
                name = Some(value);
                name_line = Some(*line_no);
            }
        }
        if mesh.is_none() {
            if let Some(value) = parse_quoted_value(line, "file") {
                mesh = Some(value.replace('\\', "/"));
                file_line = Some(*line_no);
            }
        }
        if scale.is_none() {
            if let Some(value) = parse_token_value(line, "scale") {
                scale = Some(value);
                scale_line = Some(*line_no);
            }
        }
    }
    let name = name.ok_or_else(|| format!("{source}:{block_line}: pdxmesh block has no name"))?;
    let object_index = name
        .strip_prefix("mapobject_")
        .ok_or_else(|| {
            format!("{source}:{block_line}: tree binding name {name:?} is not mapobject_N")
        })?
        .parse::<u8>()
        .map_err(|err| {
            format!("{source}:{block_line}: tree binding {name:?} has invalid index: {err}")
        })?;
    let mesh = mesh.ok_or_else(|| format!("{source}:{block_line}: pdxmesh block has no file"))?;
    Ok(TreeGfxBinding {
        object_index,
        name,
        mesh,
        scale: scale.unwrap_or_else(|| "1".to_string()),
        source: source.to_string(),
        block_line,
        name_line: name_line.unwrap_or(block_line),
        file_line: file_line.unwrap_or(block_line),
        scale_line,
    })
}

fn select_tree_gfx_binding(
    bindings: &[TreeGfxBinding],
    counts: &BTreeMap<u8, usize>,
) -> Result<TreeGfxBinding> {
    counts
        .iter()
        .filter_map(|(index, count)| {
            bindings
                .iter()
                .find(|binding| binding.object_index == *index)
                .map(|binding| (*count, binding))
        })
        .max_by_key(|(count, binding)| (*count, std::cmp::Reverse(binding.object_index)))
        .map(|(_, binding)| binding.clone())
        .or_else(|| bindings.first().cloned())
        .ok_or_else(|| "tree file layer blocked: no tree gfx bindings found".into())
}

fn validate_tree_mesh_arrays(mesh: &PdxMeshData, source: &str) -> Result<()> {
    let vertex_count = mesh.verts.len();
    if let Some(normals) = &mesh.normals {
        if normals.len() != vertex_count {
            return Err(format!(
                "{source}: mesh normal count {} does not match vertex count {vertex_count}",
                normals.len()
            )
            .into());
        }
    }
    if let Some(tangents) = &mesh.tangents {
        if tangents.len() != vertex_count {
            return Err(format!(
                "{source}: mesh tangent count {} does not match vertex count {vertex_count}",
                tangents.len()
            )
            .into());
        }
    }
    if let Some(uv0) = &mesh.uv0 {
        if uv0.len() != vertex_count {
            return Err(format!(
                "{source}: mesh uv0 count {} does not match vertex count {vertex_count}",
                uv0.len()
            )
            .into());
        }
    }
    if let Some(uv1) = &mesh.uv1 {
        if uv1.len() != vertex_count {
            return Err(format!(
                "{source}: mesh uv1 count {} does not match vertex count {vertex_count}",
                uv1.len()
            )
            .into());
        }
    }
    for (index, vertex_index) in mesh.indices.iter().copied().enumerate() {
        if usize::try_from(vertex_index)
            .ok()
            .filter(|v| *v < vertex_count)
            .is_none()
        {
            return Err(format!(
                "{source}: mesh index {index} references vertex {vertex_index}, but vertex count is {vertex_count}"
            )
            .into());
        }
    }
    Ok(())
}

fn tree_texture_from_resolver(
    resolver: &GameFileResolver,
    rel_path: &str,
) -> Result<TreeFileTexture> {
    let path = resolver.resolve_required(rel_path, "tree texture")?;
    let image = load_bmp_rgbx(&path)?;
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    let mut bgra8 = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in image.pixels {
        let (r, g, b) = rgb_tuple(pixel);
        bgra8.extend_from_slice(&[b, g, r, 255]);
    }
    Ok(TreeFileTexture {
        source: rel_path.to_string(),
        width,
        height,
        dxgi_format: DXGI_FORMAT_B8G8R8X8_UNORM,
        bgra8,
    })
}

fn push_f32s<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

struct CapturedTreeInstancePositions {
    slot: u32,
    stride: u32,
    positions: Vec<[f32; 3]>,
}

fn captured_instance_positions(
    bundle: &NativeD3d11TerrainBundle,
    draw: &MapFrameDraw,
) -> Result<Option<CapturedTreeInstancePositions>> {
    for (slot, binding) in &draw.vertex_buffers {
        if slot == "0" || binding.slot == 0 {
            continue;
        }
        let Some(stride) = binding.stride else {
            continue;
        };
        if stride < TREE_INSTANCE_STRIDE_BYTES {
            continue;
        }
        let bytes = read_map_frame_buffer_bytes(
            bundle.bundle_dir(),
            binding,
            "tree captured instance buffer",
        )?;
        let positions = decode_tree_instance_positions(&bytes, stride as usize)?;
        return Ok(Some(CapturedTreeInstancePositions {
            slot: binding.slot,
            stride,
            positions,
        }));
    }
    Ok(None)
}

fn decode_tree_instance_positions(bytes: &[u8], stride: usize) -> Result<Vec<[f32; 3]>> {
    if stride < 16 {
        return Err(
            format!("tree instance decode blocked: stride {stride} is below vPos_YRot").into(),
        );
    }
    if bytes.len() % stride != 0 {
        return Err(format!(
            "tree instance decode blocked: {} bytes is not a multiple of stride {stride}",
            bytes.len()
        )
        .into());
    }
    let mut positions = Vec::with_capacity(bytes.len() / stride);
    for chunk in bytes.chunks_exact(stride) {
        positions.push([
            f32::from_le_bytes(chunk[0..4].try_into()?),
            f32::from_le_bytes(chunk[4..8].try_into()?),
            f32::from_le_bytes(chunk[8..12].try_into()?),
        ]);
    }
    Ok(positions)
}

fn nearest_distance_stats(from: &[[f32; 3]], to: &[[f32; 3]]) -> Option<TreePositionDistanceStats> {
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let mut distances = Vec::with_capacity(from.len());
    for point in from {
        let nearest = to
            .iter()
            .map(|other| {
                let dx = point[0] - other[0];
                let dy = point[1] - other[1];
                let dz = point[2] - other[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .fold(f32::INFINITY, f32::min);
        distances.push(nearest);
    }
    distances.sort_by(|a, b| a.total_cmp(b));
    let count = distances.len();
    let mean = distances.iter().sum::<f32>() / count as f32;
    Some(TreePositionDistanceStats {
        count,
        min: distances[0],
        mean,
        p50: percentile_sorted(&distances, 0.50),
        p95: percentile_sorted(&distances, 0.95),
        max: distances[count - 1],
    })
}

fn percentile_sorted(values: &[f32], percentile: f32) -> f32 {
    let index = ((values.len() - 1) as f32 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::io::Write;

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    /// Resolve the `live_trees_measure` trees-pass capture directory. Prefers an
    /// explicit `TREES_MEASURE_CAPTURE_DIR` override, then the live capture dir,
    /// then the durable preserved corpus copy
    /// (`hoi4_capture_corpus_preserved\live_trees_measure`, the read-only B4
    /// evidence corpus). Returns `None` (skip) when none exist, so CI without the
    /// corpus stays green.
    fn trees_measure_capture_dir() -> Option<std::path::PathBuf> {
        [
            std::env::var("TREES_MEASURE_CAPTURE_DIR").ok(),
            Some(r"C:\Users\Example\AppData\Local\hoi4_captures\live_trees_measure".to_string()),
            Some(
                r"C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure".to_string(),
            ),
            Some(
                r"C:\Users\Example\AppData\Local\Claude_pzs8sxrjxfjjc.data-backup-20260709-103135\LocalCache\Local\hoi4_captures\live_trees_measure"
                    .to_string(),
            ),
        ]
        .into_iter()
        .flatten()
        .map(std::path::PathBuf::from)
        .find(|dir| dir.join(TREES_FRAME_BUNDLE_FILE).is_file())
    }

    /// TEMPLATE-MATCH gate (MAP-OBJECTS lane, 2026-07-09): the captured PAN2
    /// trees_pass template (IA slot 0: 6720 bytes / stride 56 = 120 vertices;
    /// index buffer 432 bytes R16 = 216 indices; DrawIndexedInstanced 216x37)
    /// is BYTE-EXACT equal to the shipped
    /// `gfx/models/mapitems/trees/forest_dunge_blend.mesh` shape
    /// `polySurfaceShape875` (the third shape, loddist [0,100,200,300] slot 2)
    /// interleaved as p(3f) n(3f) ta(4f) u0(2f) u1(2f) — 14 f32 = 56 bytes per
    /// vertex — with indices verbatim. That mesh is `gfx/entities/trees.gfx`
    /// binding `mapobject_5` (scale = 1). This closes template selection AND
    /// proves the engine uploads pdxasset streams verbatim in that interleave
    /// order. Skips (with a note) when neither the live capture dir nor the
    /// backup copy is present.
    #[test]
    fn gate_captured_tree_template_is_forest_dunge_blend_lod2() {
        let capture_dir = [
            std::env::var("TREES2_CAPTURE_DIR").ok(),
            Some(r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2".to_string()),
            Some(
                r"C:\Users\Example\AppData\Local\Claude_pzs8sxrjxfjjc.data-backup-20260709-103135\LocalCache\Local\hoi4_captures\live_PAN2"
                    .to_string(),
            ),
        ]
        .into_iter()
        .flatten()
        .map(std::path::PathBuf::from)
        .find(|dir| {
            dir.join("trees_frame_draw000_ia_vertex_buffer_slot0.bin")
                .is_file()
        });
        let Some(capture_dir) = capture_dir else {
            eprintln!("skipping tree template match gate: no live_PAN2 tree capture available");
            return;
        };
        let game_dir = std::path::PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping tree template match gate: {VANILLA_GAME_DIR} is absent");
            return;
        }

        let captured_vb =
            fs::read(capture_dir.join("trees_frame_draw000_ia_vertex_buffer_slot0.bin"))
                .expect("captured tree template VB");
        let captured_ib = fs::read(capture_dir.join("trees_frame_draw000_ia_index_buffer.bin"))
            .expect("captured tree template IB");
        assert_eq!(captured_vb.len(), 120 * 56, "captured template VB size");
        assert_eq!(captured_ib.len(), 216 * 2, "captured template IB size");

        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let model = pdx_model_from_resolver(
            &resolver,
            "gfx/models/mapitems/trees/forest_dunge_blend.mesh",
        )
        .expect("forest_dunge_blend decode");
        assert_eq!(model.lod_distances, vec![0.0, 100.0, 200.0, 300.0]);
        let shape = &model.shapes[2];
        assert_eq!(shape.name, "polySurfaceShape875");
        let mesh = &shape.meshes[0];
        assert_eq!(mesh.verts.len(), 120);
        assert_eq!(mesh.indices.len(), 216);

        // Interleave exactly as the engine template VB: p n ta u0 u1.
        let mut generated = Vec::with_capacity(captured_vb.len());
        for i in 0..mesh.verts.len() {
            for f in mesh.verts[i]
                .iter()
                .chain(mesh.normals.as_ref().expect("n")[i].iter())
                .chain(mesh.tangents.as_ref().expect("ta")[i].iter())
                .chain(mesh.uv0.as_ref().expect("u0")[i].iter())
                .chain(mesh.uv1.as_ref().expect("u1")[i].iter())
            {
                generated.extend_from_slice(&f.to_le_bytes());
            }
        }
        let vb_mismatch = generated
            .iter()
            .zip(captured_vb.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            vb_mismatch, 0,
            "captured tree template VB must be byte-exact forest_dunge_blend polySurfaceShape875"
        );

        let captured_indices = captured_ib
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
            .collect::<Vec<_>>();
        assert_eq!(
            captured_indices, mesh.indices,
            "captured tree template IB must equal shipped tri list verbatim"
        );
        println!(
            "gate_tree_template: {} VB bytes + {} indices byte-exact vs {}",
            captured_vb.len(),
            captured_indices.len(),
            capture_dir.display()
        );
    }

    fn interleave_pdx_mesh_56(mesh: &PdxMeshData) -> Vec<u8> {
        let mut out = Vec::with_capacity(mesh.verts.len() * 56);
        for i in 0..mesh.verts.len() {
            for f in mesh.verts[i]
                .iter()
                .chain(
                    mesh.normals
                        .as_ref()
                        .map(|v| v[i].iter())
                        .unwrap_or([0.0f32; 3].iter()),
                )
                .chain(
                    mesh.tangents
                        .as_ref()
                        .map(|v| v[i].iter())
                        .unwrap_or([0.0f32; 4].iter()),
                )
                .chain(
                    mesh.uv0
                        .as_ref()
                        .map(|v| v[i].iter())
                        .unwrap_or([0.0f32; 2].iter()),
                )
                .chain(
                    mesh.uv1
                        .as_ref()
                        .map(|v| v[i].iter())
                        .unwrap_or([0.0f32; 2].iter()),
                )
            {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        out
    }

    /// SCATTER-LAW gate (MAP-OBJECTS lane, 2026-07-09): reproduces every
    /// instance count of the captured live_trees_measure trees_pass (8 draws)
    /// from shipped files alone under the chunk law
    /// (`TREE_CHUNK_WORLD_UNITS` = 256):
    ///
    /// 1. each draw's IA slot-0 template is byte-exact-identified against the
    ///    `gfx/entities/trees.gfx` mapobject meshes (model + LOD shape);
    /// 2. the LOD shape equals the `loddist` bracket containing the captured
    ///    CB0 camera height;
    /// 3. the camera frustum's ground footprint (CB0 InvViewProjMatrix)
    ///    selects the visible 256-world-unit chunks;
    /// 4. for each model (ascending trees.bmp palette index) and each
    ///    footprint chunk in captured order (chunk x ascending, chunk z
    ///    descending, zero-count chunks skipped), the per-chunk count of that
    ///    palette index in map/trees.bmp equals the captured
    ///    DrawIndexedInstanced instance count — 8/8 exact.
    #[test]
    fn gate_tree_scatter_chunk_law_vs_live_trees_measure() {
        let Some(capture_dir) = trees_measure_capture_dir() else {
            eprintln!("skipping tree scatter chunk law gate: no live_trees_measure capture");
            return;
        };
        let game_dir = std::path::PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping tree scatter chunk law gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");

        let bundle =
            NativeD3d11TerrainBundle::load_trees(capture_dir.join(TREES_FRAME_BUNDLE_FILE))
                .expect("live_trees_measure trees bundle");
        let draws = &bundle.frame.draws;
        assert_eq!(draws.len(), 8, "live_trees_measure trees_pass draw count");

        // 1. Identify each draw's template mesh byte-exactly.
        let bindings = tree_gfx_bindings_from_resolver(&resolver).expect("trees.gfx bindings");
        struct TemplateId {
            object_index: u8,
            mesh_rel: String,
            shape_index: usize,
            shape_name: String,
            lod_distances: Vec<f32>,
        }
        let mut templates: Vec<(Vec<u8>, Vec<u8>, TemplateId)> = Vec::new();
        for binding in &bindings {
            let model =
                pdx_model_from_resolver(&resolver, &binding.mesh).expect("tree model decode");
            for (shape_index, shape) in model.shapes.iter().enumerate() {
                for mesh in &shape.meshes {
                    if mesh.material.is_collision() {
                        continue;
                    }
                    let ib = mesh
                        .indices
                        .iter()
                        .flat_map(|i| u16::try_from(*i).expect("R16 index").to_le_bytes())
                        .collect::<Vec<u8>>();
                    templates.push((
                        interleave_pdx_mesh_56(mesh),
                        ib,
                        TemplateId {
                            object_index: binding.object_index,
                            mesh_rel: binding.mesh.clone(),
                            shape_index,
                            shape_name: shape.name.clone(),
                            lod_distances: model.lod_distances.clone(),
                        },
                    ));
                }
            }
        }
        let mut identified: Vec<(&TemplateId, u32)> = Vec::new();
        for draw in draws {
            let vb_binding = draw.vertex_buffers.get("0").expect("IA slot 0");
            let vb = read_map_frame_buffer_bytes(&capture_dir, vb_binding, "tree template VB")
                .expect("tree template VB bytes");
            let ib = read_map_frame_buffer_bytes(
                &capture_dir,
                &{
                    let binding = draw.index_buffer.as_ref().expect("tree IB");
                    MapFrameBufferBinding {
                        stage: "IA".to_string(),
                        slot: 0,
                        declared_name: None,
                        file: binding.file.clone(),
                        byte_len: binding.byte_len,
                        fnv1a64: binding.fnv1a64.clone(),
                        stride: None,
                        offset: None,
                        dxgi_format: Some(binding.dxgi_format),
                        bind_flags: 0,
                        usage: 0,
                        cpu_access_flags: 0,
                    }
                },
                "tree template IB",
            )
            .expect("tree template IB bytes");
            let hit = templates
                .iter()
                .find(|(tvb, tib, _)| *tvb == vb && *tib == ib)
                .map(|(_, _, id)| id)
                .unwrap_or_else(|| {
                    panic!(
                        "captured tree draw {} template ({} bytes) matches no trees.gfx mesh shape",
                        draw.draw_index,
                        vb.len()
                    )
                });
            identified.push((hit, draw.instance_count.expect("instance count")));
        }

        // 2. Camera from the canonical scene CB0 (constants.fxh layout:
        //    InvViewProjMatrix at byte 64, vCamPos at byte 208).
        let cb0_binding = draws[0]
            .constant_buffers
            .iter()
            .find(|cb| cb.stage == "VS" && cb.slot == 0)
            .expect("VS CB0");
        let cb0 = read_map_frame_buffer_bytes(&capture_dir, cb0_binding, "scene CB0")
            .expect("scene CB0 bytes");
        assert_eq!(cb0.len(), 416, "canonical scene CB0 size");
        let f32_at = |off: usize| f32::from_le_bytes(cb0[off..off + 4].try_into().unwrap());
        let cam_y = f32_at(208 + 4);
        let mut inv_vp = [[0f64; 4]; 4];
        for (i, cell) in inv_vp.iter_mut().flatten().enumerate() {
            *cell = f32_at(64 + i * 4) as f64;
        }
        // Captured matrices are stored transposed relative to
        // row-vector*matrix; unproject clip -> world via M^T.
        let unproject = |x: f64, y: f64, z: f64| -> [f64; 3] {
            let clip = [x, y, z, 1.0];
            let mut world = [0f64; 4];
            for (row, out) in world.iter_mut().enumerate() {
                for (col, c) in clip.iter().enumerate() {
                    *out += inv_vp[col][row] * c;
                }
            }
            [
                world[0] / world[3],
                world[1] / world[3],
                world[2] / world[3],
            ]
        };
        let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY);
        for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let near = unproject(cx, cy, 0.0);
            let far = unproject(cx, cy, 1.0);
            let dy = far[1] - near[1];
            assert!(dy.abs() > 1e-9, "degenerate ground ray");
            let t = -near[1] / dy;
            let hit_x = near[0] + t * (far[0] - near[0]);
            let hit_z = near[2] + t * (far[2] - near[2]);
            min_x = min_x.min(hit_x);
            max_x = max_x.max(hit_x);
            min_z = min_z.min(hit_z);
            max_z = max_z.max(hit_z);
        }
        let chunk = TREE_CHUNK_WORLD_UNITS as f64;
        let (cx0, cx1) = ((min_x / chunk) as u32, (max_x / chunk) as u32);
        let (cz0, cz1) = ((min_z / chunk) as u32, (max_z / chunk) as u32);

        // 3. Shipped trees.bmp per-chunk counts.
        let trees_path = resolver
            .resolve_required(TREES_BMP_SOURCE, "trees.bmp")
            .expect("trees.bmp");
        let heightmap_path = resolver
            .resolve_required(TREE_HEIGHTMAP_SOURCE, "heightmap.bmp")
            .expect("heightmap.bmp");
        let trees = load_bmp_indexed(&trees_path).expect("trees.bmp decode");
        let heightmap = load_bmp_indexed(&heightmap_path).expect("heightmap.bmp decode");
        let chunk_counts = tree_chunk_index_counts(&trees, heightmap.width, heightmap.height);

        // 4. Expected draw list: models ascending, chunk x ascending, chunk z
        //    descending, zero-count chunks skipped (captured order law).
        let mut model_indices = identified
            .iter()
            .map(|(id, _)| id.object_index)
            .collect::<Vec<_>>();
        model_indices.dedup();
        let mut sorted_check = model_indices.clone();
        sorted_check.sort_unstable();
        assert_eq!(
            model_indices, sorted_check,
            "captured draws must group models in ascending palette order"
        );
        let mut expected: Vec<(u8, u32, u32, usize)> = Vec::new();
        for &model in &model_indices {
            for cx in cx0..=cx1 {
                for cz in (cz0..=cz1).rev() {
                    if let Some(count) = chunk_counts.get(&(cx, cz, model)) {
                        expected.push((model, cx, cz, *count));
                    }
                }
            }
        }
        assert_eq!(
            expected.len(),
            identified.len(),
            "visible-chunk draw count mismatch: expected {expected:?}"
        );
        let mut exact = 0usize;
        for ((id, captured_count), (model, cx, cz, file_count)) in identified.iter().zip(&expected)
        {
            assert_eq!(
                id.object_index, *model,
                "draw model order vs chunk law expectation"
            );
            assert_eq!(
                *captured_count as usize, *file_count,
                "chunk ({cx},{cz}) index {model}: captured {} vs trees.bmp {}",
                captured_count, file_count
            );
            exact += 1;
            // LOD bracket: largest k with loddist[k] <= camera height.
            let lod = id
                .lod_distances
                .iter()
                .rposition(|d| *d <= cam_y)
                .expect("loddist bracket");
            assert_eq!(
                id.shape_index, lod,
                "{} LOD shape {} ({}) vs camera height {cam_y} bracket {lod}",
                id.mesh_rel, id.shape_index, id.shape_name
            );
        }
        println!(
            "gate_tree_scatter: {exact}/8 captured instance counts reproduced exactly from \
             map/trees.bmp chunk law (chunks x {cx0}..={cx1}, z {cz0}..={cz1}, cam_y {cam_y})"
        );
    }

    /// SEASON CB1 gate (B4): the tree VS constant buffer 1 declared by
    /// `gfx/FX/tree.shader` `ConstantBuffer( 1, 32 )` is
    /// `float4x4 ShadowMapTextureMatrix; float vSeasonLerp; float vSeasonColumn;
    /// float vTreeFade;` (64 + 4 + 4 + 4 = 76 B, padded to 80). This gate proves
    /// the three season/fade scalars against the captured `live_trees_measure`
    /// trees_pass and derives `vTreeFade` from shipped files:
    ///
    /// * `vTreeFade` (FILE): the camera-height fade the PS emits as the tree
    ///   output alpha (`return float4( vColor, vTreeFade )`). It equals
    ///   `saturate( (TREE_FADE_FAR - vCamPos.y) / (TREE_FADE_FAR - TREE_FADE_NEAR) )`
    ///   with `TREE_FADE_NEAR = 250.0`, `TREE_FADE_FAR = 350.0`
    ///   (`common/defines/00_graphics.lua:1068-1069`) and `vCamPos.y` from the
    ///   scene CB0. At the captured `cam_y = 348.0697` that is
    ///   `(350 - 348.0697)/100 = 0.019303`, matched to < 1e-4 against every draw.
    /// * `vSeasonLerp` / `vSeasonColumn` (CAPTURE): the season-atlas blend factor
    ///   and column index. Constant across all 8 draws (a single dated snapshot);
    ///   both are 0.0 in this capture. The date -> (column, lerp) map is CPU-side
    ///   and remains UNKNOWN (see LANE_REPORT.md / FIDELITY_LEDGER §2.11).
    #[test]
    fn gate_tree_season_cb1_vs_live_trees_measure() {
        // File-cited fade window (00_graphics.lua:1068-1069).
        const TREE_FADE_NEAR: f32 = 250.0;
        const TREE_FADE_FAR: f32 = 350.0;

        let Some(capture_dir) = trees_measure_capture_dir() else {
            eprintln!("skipping tree season CB1 gate: no live_trees_measure capture");
            return;
        };
        let bundle =
            NativeD3d11TerrainBundle::load_trees(capture_dir.join(TREES_FRAME_BUNDLE_FILE))
                .expect("live_trees_measure trees bundle");
        let draws = &bundle.frame.draws;
        assert_eq!(draws.len(), 8, "live_trees_measure trees_pass draw count");

        // Camera height from the canonical scene CB0 (vCamPos.y at byte 212).
        let cb0_binding = draws[0]
            .constant_buffers
            .iter()
            .find(|cb| cb.stage == "VS" && cb.slot == 0)
            .expect("VS CB0");
        let cb0 = read_map_frame_buffer_bytes(&capture_dir, cb0_binding, "scene CB0")
            .expect("scene CB0 bytes");
        assert_eq!(cb0.len(), 416, "canonical scene CB0 size");
        let cam_y = f32::from_le_bytes(cb0[212..216].try_into().unwrap());
        let expected_fade =
            ((TREE_FADE_FAR - cam_y) / (TREE_FADE_FAR - TREE_FADE_NEAR)).clamp(0.0, 1.0);

        let mut season_lerp: Option<f32> = None;
        let mut season_column: Option<f32> = None;
        let mut checked = 0usize;
        for draw in draws {
            let cb1_binding = draw
                .constant_buffers
                .iter()
                .find(|cb| cb.stage == "VS" && cb.slot == 1)
                .expect("VS CB1");
            let cb1 = read_map_frame_buffer_bytes(&capture_dir, cb1_binding, "tree season CB1")
                .expect("tree season CB1 bytes");
            assert_eq!(
                cb1.len(),
                80,
                "tree ConstantBuffer(1) is 80 bytes (mat4 + 3 floats padded)"
            );
            // ShadowMapTextureMatrix = bytes 0..64, then the 3 scalars.
            let v_season_lerp = f32::from_le_bytes(cb1[64..68].try_into().unwrap());
            let v_season_column = f32::from_le_bytes(cb1[68..72].try_into().unwrap());
            let v_tree_fade = f32::from_le_bytes(cb1[72..76].try_into().unwrap());

            // vTreeFade is FILE-derivable from the camera height + fade window.
            assert!(
                (v_tree_fade - expected_fade).abs() < 1e-4,
                "draw {}: captured vTreeFade {v_tree_fade} vs file law {expected_fade} \
                 (cam_y {cam_y}, window [{TREE_FADE_NEAR},{TREE_FADE_FAR}])",
                draw.draw_index
            );

            // Season scalars are one dated snapshot: identical across every draw.
            match season_lerp {
                None => season_lerp = Some(v_season_lerp),
                Some(prev) => assert_eq!(
                    prev.to_bits(),
                    v_season_lerp.to_bits(),
                    "vSeasonLerp must be constant across the trees pass"
                ),
            }
            match season_column {
                None => season_column = Some(v_season_column),
                Some(prev) => assert_eq!(
                    prev.to_bits(),
                    v_season_column.to_bits(),
                    "vSeasonColumn must be constant across the trees pass"
                ),
            }
            checked += 1;
        }
        assert_eq!(checked, 8, "all 8 tree draws gated");
        println!(
            "gate_tree_season_cb1: 8/8 draws vTreeFade == file law {expected_fade:.6} \
             (cam_y {cam_y}); vSeasonLerp {:?} vSeasonColumn {:?} (CAPTURE snapshot; \
             date->season map UNKNOWN)",
            season_lerp.unwrap(),
            season_column.unwrap()
        );
    }

    #[test]
    fn parses_tree_gfx_bindings_with_lines() {
        let text = r#"
        pdxmesh = {
            name = "mapobject_6"
            file = "gfx/models/mapitems/trees/forest_dunge.mesh"
            scale = 1
        }
        "#;
        let bindings = parse_tree_gfx_bindings(text, TREE_GFX_BINDING_SOURCE).expect("bindings");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].object_index, 6);
        assert_eq!(
            bindings[0].mesh,
            "gfx/models/mapitems/trees/forest_dunge.mesh"
        );
        assert_eq!(bindings[0].name_line, 3);
        assert_eq!(bindings[0].file_line, 4);
    }

    #[test]
    fn mesh_vertex_buffer_rejects_out_of_bounds_indices() {
        let model = PdxModel {
            lod_distances: Vec::new(),
            shapes: vec![PdxShape {
                name: "shape".to_string(),
                lod: None,
                meshes: vec![PdxMeshData {
                    verts: vec![[0.0, 0.0, 0.0]],
                    normals: Some(vec![[0.0, 1.0, 0.0]]),
                    tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]]),
                    uv0: Some(vec![[0.0, 0.0]]),
                    uv1: Some(vec![[0.0, 0.0]]),
                    indices: vec![0, 2, 0],
                    aabb_min: [0.0; 3],
                    aabb_max: [0.0; 3],
                    bounding_sphere: None,
                    material: PdxMaterial {
                        shader: "PdxMeshStandard".to_string(),
                        diffuse: None,
                        normal_map: None,
                        specular: None,
                    },
                    skin: None,
                }],
                skeleton: Vec::new(),
            }],
            locators: Vec::new(),
        };
        let err = tree_mesh_vertex_buffer_from_model(&model, test_binding())
            .expect_err("out of bounds index must fail");
        assert!(err.to_string().contains("references vertex 2"), "{err}");
    }

    #[test]
    fn mesh_vertex_buffer_uses_shader_stride_and_bounds() {
        let model = PdxModel {
            lod_distances: Vec::new(),
            shapes: vec![PdxShape {
                name: "shape".to_string(),
                lod: None,
                meshes: vec![PdxMeshData {
                    verts: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
                    normals: None,
                    tangents: None,
                    uv0: Some(vec![[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]),
                    uv1: None,
                    indices: vec![0, 1, 2],
                    aabb_min: [0.0; 3],
                    aabb_max: [0.0; 3],
                    bounding_sphere: None,
                    material: PdxMaterial {
                        shader: "PdxMeshStandard".to_string(),
                        diffuse: Some("diffuse.dds".to_string()),
                        normal_map: Some("normal.dds".to_string()),
                        specular: None,
                    },
                    skin: None,
                }],
                skeleton: Vec::new(),
            }],
            locators: Vec::new(),
        };
        let mesh = tree_mesh_vertex_buffer_from_model(&model, test_binding()).expect("mesh");
        assert_eq!(mesh.vertex_stride_bytes, TREE_MESH_VERTEX_STRIDE_BYTES);
        assert_eq!(
            mesh.vertices.len(),
            3 * TREE_MESH_VERTEX_STRIDE_BYTES as usize
        );
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.vertex_count, 3);
        assert_eq!(mesh.index_count, 3);
    }

    #[test]
    fn resolver_tree_instance_generation_honors_vfs_with_exact_producer() {
        let root = temp_test_dir("tree_file_resolver_overrides");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_indexed_bmp_values(&vanilla.join(TREES_BMP_SOURCE), 2, 1, &[1, 1]);
        write_indexed_bmp_values(
            &vanilla.join(TREE_HEIGHTMAP_SOURCE),
            256,
            256,
            &vec![10; 256 * 256],
        );
        write_indexed_bmp_values(&mod_root.join(TREES_BMP_SOURCE), 2, 1, &[0, 6]);
        let resolver =
            GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).expect("resolver");
        let generated = synthesize_tree_instances_from_resolver(&resolver, 256, 256)
            .expect("exact generator");
        assert_eq!(generated.stats.instance_count, 1);
        assert_eq!(generated.stats.tree_index_counts.get(&6), Some(&1));
        assert_eq!(generated.batches.len(), 1);
        assert_eq!(generated.batches[0].palette_index, 6);
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(root);
    }

    fn test_binding() -> TreeGfxBinding {
        TreeGfxBinding {
            object_index: 6,
            name: "mapobject_6".to_string(),
            mesh: "gfx/models/mapitems/trees/forest_dunge.mesh".to_string(),
            scale: "1".to_string(),
            source: TREE_GFX_BINDING_SOURCE.to_string(),
            block_line: 1,
            name_line: 2,
            file_line: 3,
            scale_line: Some(4),
        }
    }

    fn write_indexed_bmp_values(path: &Path, width: usize, height: usize, pixels: &[u8]) {
        assert_eq!(pixels.len(), width * height);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        let row_stride = (width + 3) & !3;
        let palette_bytes = 256usize * 4;
        let pixel_offset = 14usize + 40usize + palette_bytes;
        let image_bytes = row_stride * height;
        let file_size = pixel_offset + image_bytes;
        let mut file = fs::File::create(path).expect("create BMP");
        file.write_all(b"BM").expect("sig");
        file.write_all(&(file_size as u32).to_le_bytes())
            .expect("size");
        file.write_all(&[0u8; 4]).expect("reserved");
        file.write_all(&(pixel_offset as u32).to_le_bytes())
            .expect("offset");
        file.write_all(&40u32.to_le_bytes()).expect("dib");
        file.write_all(&(width as i32).to_le_bytes()).expect("w");
        file.write_all(&(height as i32).to_le_bytes()).expect("h");
        file.write_all(&1u16.to_le_bytes()).expect("planes");
        file.write_all(&8u16.to_le_bytes()).expect("bpp");
        file.write_all(&0u32.to_le_bytes()).expect("compression");
        file.write_all(&(image_bytes as u32).to_le_bytes())
            .expect("image size");
        file.write_all(&2835u32.to_le_bytes()).expect("x ppm");
        file.write_all(&2835u32.to_le_bytes()).expect("y ppm");
        file.write_all(&256u32.to_le_bytes()).expect("color count");
        file.write_all(&256u32.to_le_bytes()).expect("important");
        for palette in 0..=255u8 {
            file.write_all(&[palette, palette, palette, 0])
                .expect("palette");
        }
        for y in (0..height).rev() {
            let mut row = vec![0u8; row_stride];
            row[..width].copy_from_slice(&pixels[y * width..(y + 1) * width]);
            file.write_all(&row).expect("row");
        }
    }
}
