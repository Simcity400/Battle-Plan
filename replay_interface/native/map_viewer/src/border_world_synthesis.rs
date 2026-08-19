
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const BORDER_DEFAULT_MAP_SOURCE: &str = "map/default.map";
pub const BORDER_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";
pub const BORDER_PROVINCES_BMP_SOURCE: &str = "map/provinces.bmp";
pub const BORDER_HEIGHTMAP_BMP_SOURCE: &str = "map/heightmap.bmp";
pub const BORDER_ADJACENCIES_CSV_SOURCE: &str = "map/adjacencies.csv";
pub const BORDER_SHADER_SOURCE: &str = "gfx/FX/border.shader";
pub const BORDER_CONSTANTS_SOURCE: &str = "gfx/FX/constants.fxh";
pub const BORDER_HISTORY_STATES_DIR: &str = "history/states";
pub const BORDER_VERTEX_STRIDE_BYTES: usize = 16;

const BORDER_TEXTURES: &[(BorderMaterial, &str)] = &[
    (
        BorderMaterial::Province,
        "map/terrain/border_province_0.dds",
    ),
    (BorderMaterial::State, "map/terrain/border_state_0.dds"),
    (BorderMaterial::Country, "map/terrain/border_country_0.dds"),
    (
        BorderMaterial::Impassable,
        "map/terrain/border_impassable_0.dds",
    ),
    (BorderMaterial::SeaCoast, "map/terrain/border_sea_0.dds"),
    (
        BorderMaterial::SeaRegion,
        "map/terrain/border_sea_region_0.dds",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderRank {
    Province,
    State,
    Country,
}

impl BorderRank {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Province => "province",
            Self::State => "state",
            Self::Country => "country",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderWaterClass {
    LandLand,
    LandWater,
    WaterWater,
}

impl BorderWaterClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LandLand => "land_land",
            Self::LandWater => "land_water",
            Self::WaterWater => "water_water",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderMaterial {
    Province,
    State,
    Country,
    Impassable,
    SeaCoast,
    SeaRegion,
}

impl BorderMaterial {
    pub const ALL: [Self; 6] = [
        Self::Province,
        Self::State,
        Self::Country,
        Self::Impassable,
        Self::SeaCoast,
        Self::SeaRegion,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Province => "province",
            Self::State => "state",
            Self::Country => "country",
            Self::Impassable => "impassable",
            Self::SeaCoast => "sea_coast",
            Self::SeaRegion => "sea_region",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderChain {
    pub province_a: u32,
    pub province_b: u32,
    pub state_a: Option<u32>,
    pub state_b: Option<u32>,
    pub rank: BorderRank,
    pub water_class: BorderWaterClass,
    pub material: BorderMaterial,
    pub closed: bool,
    pub manhattan_length: u32,
    pub rows: u32,
    pub first_vertex: u32,
    pub first_index: u32,
}

impl BorderChain {
    pub fn index_count(&self) -> u32 {
        6 * (self.rows - 1)
    }
    pub fn vertex_count(&self) -> u32 {
        self.rows * 2
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BorderWorldVertex {
    pub position: [f32; 3],
    pub uv: [i32; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderWorldMesh {
    pub map_width: usize,
    pub map_height: usize,
    pub chains: Vec<BorderChain>,
    pub vertices: Vec<BorderWorldVertex>,
    pub indices: Vec<u32>,
    pub stats: BorderWorldSynthesisStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BorderClassIndexRange {
    pub material: BorderMaterial,
    pub start_index: u32,
    pub index_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderClassIndexPartition {
    pub indices: Vec<u32>,
    pub ranges: Vec<BorderClassIndexRange>,
}

impl BorderClassIndexPartition {
    pub const MATERIAL_DRAW_ORDER: [BorderMaterial; 6] = [
        BorderMaterial::Province,
        BorderMaterial::State,
        BorderMaterial::Country,
        BorderMaterial::Impassable,
        BorderMaterial::SeaCoast,
        BorderMaterial::SeaRegion,
    ];
    pub const EARLY_BASE_DRAW_ORDER: [BorderMaterial; 4] = [
        BorderMaterial::Province,
        BorderMaterial::State,
        BorderMaterial::Country,
        BorderMaterial::Impassable,
    ];
    pub const LATE_BASE_DRAW_ORDER: [BorderMaterial; 2] =
        [BorderMaterial::SeaCoast, BorderMaterial::SeaRegion];
    pub const SECONDARY_BANK_DRAW_ORDER: [BorderMaterial; 6] = Self::MATERIAL_DRAW_ORDER;
}

impl BorderWorldMesh {
    pub fn to_shader_vertex_layout_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.vertices.len() * BORDER_VERTEX_STRIDE_BYTES);
        for vertex in &self.vertices {
            for value in vertex.position {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                let packed = i16::try_from(value).map_err(|_| {
                    format!(
                        "border world synthesis blocked: vertex uv lane {value} exceeds captured R16G16_SINT range"
                    )
                })?;
                out.extend_from_slice(&packed.to_le_bytes());
            }
        }
        Ok(out)
    }

    pub fn class_index_partition(&self) -> BorderClassIndexPartition {
        let mut indices = Vec::with_capacity(self.indices.len());
        let mut ranges = Vec::new();
        for material in BorderClassIndexPartition::MATERIAL_DRAW_ORDER {
            let start_index = indices.len() as u32;
            for chain in &self.chains {
                if chain.material != material {
                    continue;
                }
                let start = chain.first_index as usize;
                let end = start + chain.index_count() as usize;
                indices.extend_from_slice(&self.indices[start..end]);
            }
            let index_count = indices.len() as u32 - start_index;
            if index_count > 0 {
                ranges.push(BorderClassIndexRange {
                    material,
                    start_index,
                    index_count,
                });
            }
        }
        BorderClassIndexPartition { indices, ranges }
    }

    pub fn chain_centerline(&self, chain: &BorderChain) -> Vec<[f32; 3]> {
        let mut out = Vec::with_capacity(chain.rows as usize);
        for row in 0..chain.rows as usize {
            let left = &self.vertices[chain.first_vertex as usize + 2 * row].position;
            let right = &self.vertices[chain.first_vertex as usize + 2 * row + 1].position;
            out.push([
                (left[0] + right[0]) * 0.5,
                (left[1] + right[1]) * 0.5,
                (left[2] + right[2]) * 0.5,
            ]);
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderWorldSynthesisStats {
    pub map_width: usize,
    pub map_height: usize,
    pub definition_rows: usize,
    pub state_files: usize,
    pub states: usize,
    pub state_owners_1936: usize,
    pub unknown_pixels: usize,
    pub raster_edges: usize,
    pub chains: usize,
    pub closed_loop_chains: usize,
    pub vertices: usize,
    pub indices: usize,
    pub total_length: f64,
    pub border_width: f32,
    pub width_source: String,
    pub vertex_stride_bytes: usize,
    pub edge_count_by_rank: BTreeMap<String, usize>,
    pub edge_count_by_water_class: BTreeMap<String, usize>,
    pub edge_count_by_material: BTreeMap<String, usize>,
    pub chain_count_by_material: BTreeMap<String, usize>,
    pub length_by_rank: BTreeMap<String, f64>,
    pub width_by_material: BTreeMap<String, f32>,
    pub coastline_edges: usize,
    pub water_mask_coastline_edges: usize,
    pub coastline_disagreement_edges: usize,
    pub x_wrap_edges: usize,
    pub impassable_adjacency_pairs: usize,
    pub impassable_adjacency_edges: usize,
    pub impassable_adjacency_non_land_rows: usize,
    pub renderer_note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderMaterialSource {
    pub material: String,
    pub source: String,
    pub width: usize,
    pub height: usize,
    pub format: String,
    pub mip_count: u32,
    pub fnv1a64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderSourceExcerpt {
    pub source: String,
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderSourceEvidenceReport {
    pub excerpts: Vec<BorderSourceExcerpt>,
    pub material_sources: Vec<BorderMaterialSource>,
}

struct BorderSynthesisInputs {
    province_ids: Vec<u32>,
    width: usize,
    height: usize,
    definitions: BTreeMap<u32, ProvinceDefinition>,
    state_meta: BTreeMap<u32, StateMeta>,
    province_to_state: BTreeMap<u32, u32>,
    state_owners: BTreeMap<u32, String>,
    heightmap: IndexImage,
    border_width: f32,
    unknown_pixels: usize,
    state_files: usize,
    impassable_pairs: BTreeSet<(u32, u32)>,
    impassable_adjacency_non_land_rows: usize,
}

#[derive(Clone, Copy)]
struct ClassifiedBorderEdge {
    rank: BorderRank,
    water_class: BorderWaterClass,
    material: BorderMaterial,
    state_a: Option<u32>,
    state_b: Option<u32>,
}

pub fn synthesize_border_world_mesh_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BorderWorldMesh> {
    let inputs = border_synthesis_inputs_from_resolver(resolver)?;
    synthesize_border_world_mesh_from_inputs(&inputs)
}

pub fn synthesize_border_world_mesh_with_keying(
    resolver: &GameFileResolver,
    keying: &KeyingTables,
) -> Result<BorderWorldMesh> {
    let mut inputs = border_synthesis_inputs_from_resolver(resolver)?;
    inputs.state_owners = keying.state_owners.clone();
    synthesize_border_world_mesh_from_inputs(&inputs)
}

pub fn border_material_sources_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BorderMaterialSource>> {
    let mut out = Vec::new();
    for (material, source) in BORDER_TEXTURES {
        let path = resolver.resolve_required(source, "border material DDS")?;
        let info = dds_info(&path)?;
        out.push(BorderMaterialSource {
            material: material.as_str().to_string(),
            source: (*source).to_string(),
            width: info.dimensions.width,
            height: info.dimensions.height,
            format: info.format,
            mip_count: info.mip_count,
            fnv1a64: fnv1a64_file(&path)?,
        });
    }
    Ok(out)
}

pub fn border_source_evidence_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BorderSourceEvidenceReport> {
    let mut excerpts = Vec::new();
    let mut seen = BTreeSet::new();

    push_line_range(
        resolver,
        BORDER_DEFAULT_MAP_SOURCE,
        1,
        16,
        &mut excerpts,
        &mut seen,
    )?;
    push_first_matching_line(
        resolver,
        BORDER_DEFINITION_CSV_SOURCE,
        |line| line.starts_with("0;"),
        &mut excerpts,
        &mut seen,
    )?;
    push_first_matching_line(
        resolver,
        BORDER_DEFINITION_CSV_SOURCE,
        |line| line.contains(";sea;"),
        &mut excerpts,
        &mut seen,
    )?;
    push_first_matching_line(
        resolver,
        BORDER_DEFINITION_CSV_SOURCE,
        |line| line.contains(";lake;"),
        &mut excerpts,
        &mut seen,
    )?;
    push_first_matching_line(
        resolver,
        BORDER_ADJACENCIES_CSV_SOURCE,
        |line| line.starts_with("From;To;Type;"),
        &mut excerpts,
        &mut seen,
    )?;
    push_first_matching_line(
        resolver,
        BORDER_ADJACENCIES_CSV_SOURCE,
        |line| line.contains(";impassable;"),
        &mut excerpts,
        &mut seen,
    )?;
    for pattern in [
        "VertexStruct VS_INPUT_BORDER",
        "VertexStruct VS_OUTPUT_BORDER",
        "ConstantBuffer( 2",
        "BorderDiffuse",
        "SnowMudData",
        "IntelMap",
        "ShadowMap",
        "vColor = tex2D( BorderDiffuse",
        "Effect border",
    ] {
        push_context_for_pattern(
            resolver,
            BORDER_SHADER_SOURCE,
            pattern,
            0,
            6,
            &mut excerpts,
            &mut seen,
        )?;
    }
    for pattern in [
        "BORDER_TILE",
        "BORDER_WIDTH",
        "SHADOW_WEIGHT_BORDER",
        "BORDER_MAP_TILE",
    ] {
        push_context_for_pattern(
            resolver,
            BORDER_CONSTANTS_SOURCE,
            pattern,
            0,
            1,
            &mut excerpts,
            &mut seen,
        )?;
    }
    for pattern in [
        "BORDER_WIDTH",
        "PROVINCE_BORDER_FADE_NEAR",
        "PROVINCE_BORDER_FADE_FAR",
        "STATE_BORDER_FADE_NEAR",
        "STATE_BORDER_FADE_FAR",
    ] {
        push_context_for_pattern(
            resolver,
            BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
            pattern,
            0,
            0,
            &mut excerpts,
            &mut seen,
        )?;
    }
    push_state_file_example(resolver, &mut excerpts, &mut seen)?;

    Ok(BorderSourceEvidenceReport {
        excerpts,
        material_sources: border_material_sources_from_resolver(resolver)?,
    })
}

fn border_synthesis_inputs_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BorderSynthesisInputs> {
    let default_map = resolver.resolve_required(BORDER_DEFAULT_MAP_SOURCE, "map/default.map")?;
    validate_default_map_border_bindings(&default_map)?;

    let definition_csv =
        resolver.resolve_required(BORDER_DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(BORDER_PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let heightmap_bmp = resolver.resolve_required(BORDER_HEIGHTMAP_BMP_SOURCE, "heightmap.bmp")?;
    let graphics_defines =
        resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let adjacencies_csv =
        resolver.resolve_required(BORDER_ADJACENCIES_CSV_SOURCE, "adjacencies.csv")?;
    let state_paths = resolver.resolve_dir_files(BORDER_HISTORY_STATES_DIR, Some("txt"))?;

    let (definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let adjacency_impassables = parse_impassable_adjacencies(&adjacencies_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    if bmp.unknown_pixels != 0 {
        return Err(format!(
            "border world synthesis blocked: {} unknown pixel(s) in {} are not present in {}",
            bmp.unknown_pixels, BORDER_PROVINCES_BMP_SOURCE, BORDER_DEFINITION_CSV_SOURCE
        )
        .into());
    }
    let heightmap = load_bmp_indexed(&heightmap_bmp)?;
    let (state_meta, province_to_state, state_owners) = load_states(&state_paths)?;
    let border_width = border_width_from_graphics_defines(&graphics_defines)?;

    let impassable_adjacency_non_land_rows = adjacency_impassables
        .iter()
        .filter(|&&(a, b)| !adjacency_pair_is_land_land(&definitions, a, b).unwrap_or(false))
        .count();

    Ok(BorderSynthesisInputs {
        province_ids: bmp.ids,
        width: bmp.width,
        height: bmp.height,
        definitions,
        state_meta,
        province_to_state,
        state_owners,
        heightmap,
        border_width,
        unknown_pixels: bmp.unknown_pixels,
        state_files: state_paths.len(),
        impassable_pairs: adjacency_impassables,
        impassable_adjacency_non_land_rows,
    })
}

fn parse_impassable_adjacencies(path: &std::path::Path) -> Result<BTreeSet<(u32, u32)>> {
    let text = read_latin1(path)?;
    let mut impassable = BTreeSet::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split(';').collect();
        if fields.len() < 3 {
            return Err(format!(
                "border world synthesis blocked: {} line {} has {} columns; From/To/Type are required",
                BORDER_ADJACENCIES_CSV_SOURCE,
                line_index + 1,
                fields.len()
            )
            .into());
        }
        let from_field = fields[0].trim();
        let to_field = fields[1].trim();
        let type_field = fields[2].trim();

        if line == "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment"
        {
            continue;
        }
        if from_field == "-1" && to_field == "-1" && type_field.is_empty() {
            break;
        }
        let from = from_field.parse::<u32>().map_err(|_| {
            format!(
                "border world synthesis blocked: {} line {} has non-numeric From {:?}",
                BORDER_ADJACENCIES_CSV_SOURCE,
                line_index + 1,
                from_field
            )
        })?;
        let to = to_field.parse::<u32>().map_err(|_| {
            format!(
                "border world synthesis blocked: {} line {} has non-numeric To {:?}",
                BORDER_ADJACENCIES_CSV_SOURCE,
                line_index + 1,
                to_field
            )
        })?;

        match type_field.to_ascii_lowercase().as_str() {
            "impassable" => {
                let pair = if from <= to { (from, to) } else { (to, from) };
                impassable.insert(pair);
            }
            "" => {}
            "sea" => {}
            other => {
                return Err(format!(
                    "border world synthesis blocked: {} line {} declares unknown adjacency type {:?} \
                     for province pair {from};{to}",
                    BORDER_ADJACENCIES_CSV_SOURCE,
                    line_index + 1,
                    other
                )
                .into());
            }
        }
    }
    Ok(impassable)
}

fn adjacency_pair_is_land_land(
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    a: u32,
    b: u32,
) -> Option<bool> {
    let type_a = definitions.get(&a)?.province_type.as_str();
    let type_b = definitions.get(&b)?.province_type.as_str();
    Some(type_a == "land" && type_b == "land")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Corner {
    j: i32,
    i: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LatticeEdge {
    Vertical { i: u32, j: i32 },
    Horizontal { i: u32, j: i32 },
}

impl LatticeEdge {
    fn corners(self, width: u32) -> (Corner, Corner) {
        match self {
            Self::Vertical { i, j } => (Corner { j, i }, Corner { j: j + 1, i }),
            Self::Horizontal { i, j } => (
                Corner { j, i },
                Corner {
                    j,
                    i: (i + 1) % width,
                },
            ),
        }
    }

    fn other_corner(self, from: Corner, width: u32) -> Corner {
        let (a, b) = self.corners(width);
        if a == from {
            b
        } else {
            a
        }
    }
}

fn synthesize_border_world_mesh_from_inputs(
    inputs: &BorderSynthesisInputs,
) -> Result<BorderWorldMesh> {
    synthesize_border_world_mesh_from_inputs_with_trace_law(inputs, BorderTraceLaw::RegionContours)
}

fn synthesize_border_world_mesh_from_inputs_with_trace_law(
    inputs: &BorderSynthesisInputs,
    trace_law: BorderTraceLaw,
) -> Result<BorderWorldMesh> {
    synthesize_border_world_mesh_from_inputs_with_laws(
        inputs,
        trace_law,
        ClosedLoopSeamLaw::CompleteStations,
    )
}

fn synthesize_border_world_mesh_from_inputs_with_laws(
    inputs: &BorderSynthesisInputs,
    trace_law: BorderTraceLaw,
    closed_loop_seam_law: ClosedLoopSeamLaw,
) -> Result<BorderWorldMesh> {
    if inputs.width == 0 || inputs.height == 0 {
        return Err("border world synthesis blocked: provinces.bmp dimensions are empty".into());
    }
    if inputs.province_ids.len() != inputs.width * inputs.height {
        return Err(format!(
            "border world synthesis blocked: province id payload has {} pixels but dimensions are {}x{}",
            inputs.province_ids.len(),
            inputs.width,
            inputs.height
        )
        .into());
    }
    if inputs.heightmap.width != inputs.width || inputs.heightmap.height != inputs.height {
        return Err(format!(
            "border world synthesis blocked: {} is {}x{} but {} is {}x{}",
            BORDER_HEIGHTMAP_BMP_SOURCE,
            inputs.heightmap.width,
            inputs.heightmap.height,
            BORDER_PROVINCES_BMP_SOURCE,
            inputs.width,
            inputs.height
        )
        .into());
    }
    if !inputs.border_width.is_finite() || inputs.border_width <= 0.0 {
        return Err(format!(
            "border world synthesis blocked: NGraphics.BORDER_WIDTH must be positive finite, got {}",
            inputs.border_width
        )
        .into());
    }

    let mut mesh = BorderWorldMesh {
        map_width: inputs.width,
        map_height: inputs.height,
        chains: Vec::new(),
        vertices: Vec::new(),
        indices: Vec::new(),
        stats: BorderWorldSynthesisStats {
            map_width: inputs.width,
            map_height: inputs.height,
            definition_rows: inputs.definitions.len(),
            state_files: inputs.state_files,
            states: inputs.state_meta.len(),
            state_owners_1936: inputs.state_owners.len(),
            unknown_pixels: inputs.unknown_pixels,
            raster_edges: 0,
            chains: 0,
            closed_loop_chains: 0,
            vertices: 0,
            indices: 0,
            total_length: 0.0,
            border_width: inputs.border_width,
            width_source: format!("{BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE}:NGraphics.BORDER_WIDTH"),
            vertex_stride_bytes: BORDER_VERTEX_STRIDE_BYTES,
            edge_count_by_rank: BTreeMap::new(),
            edge_count_by_water_class: BTreeMap::new(),
            edge_count_by_material: BTreeMap::new(),
            chain_count_by_material: BTreeMap::new(),
            length_by_rank: BTreeMap::new(),
            width_by_material: BTreeMap::new(),
            coastline_edges: 0,
            water_mask_coastline_edges: 0,
            coastline_disagreement_edges: 0,
            x_wrap_edges: 0,
            impassable_adjacency_pairs: inputs.impassable_pairs.len(),
            impassable_adjacency_edges: 0,
            impassable_adjacency_non_land_rows: inputs.impassable_adjacency_non_land_rows,
                            labels + 4x3 contour follow + neighbor-pair split + seam rotation + \
                            junction-corner anchor competition; preserved-pool 40,678 matches, \
                            924/928 closed loops exact"
                .to_string(),
        },
    };

    for (material, _) in BORDER_TEXTURES {
        mesh.stats
            .width_by_material
            .insert(material.as_str().to_string(), inputs.border_width);
    }

    let width = inputs.width;
    let height = inputs.height;
    let mut pair_edges: BTreeMap<(u32, u32), Vec<LatticeEdge>> = BTreeMap::new();
    let mut classified: BTreeMap<(u32, u32), ClassifiedBorderEdge> = BTreeMap::new();
    let mut synth_vert = vec![false; width * height];
    let mut synth_horiz = vec![false; width * height];

    let classify_pair = |classified: &mut BTreeMap<(u32, u32), ClassifiedBorderEdge>,
                         a: u32,
                         b: u32|
     -> Result<ClassifiedBorderEdge> {
        let key = normalized_pair(a, b);
        if let Some(existing) = classified.get(&key) {
            return Ok(*existing);
        }
        let edge = classify_border_edge(inputs, key.0, key.1)?;
        classified.insert(key, edge);
        Ok(edge)
    };

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let province = inputs.province_ids[idx];

            let right_x = (x + 1) % width;
            let right = inputs.province_ids[y * width + right_x];
            if province != right {
                if right_x == 0 {
                    mesh.stats.x_wrap_edges += 1;
                }
                let edge = classify_pair(&mut classified, province, right)?;
                record_edge_stats(&mut mesh.stats, inputs, province, right, edge);
                if edge.water_class == BorderWaterClass::LandWater {
                    synth_vert[idx] = true;
                }
                pair_edges
                    .entry(normalized_pair(province, right))
                    .or_default()
                    .push(LatticeEdge::Vertical {
                        i: x as u32,
                        j: y as i32 - 1,
                    });
            }

            if y + 1 < height {
                let down = inputs.province_ids[(y + 1) * width + x];
                if province != down {
                    let edge = classify_pair(&mut classified, province, down)?;
                    record_edge_stats(&mut mesh.stats, inputs, province, down, edge);
                    if edge.water_class == BorderWaterClass::LandWater {
                        synth_horiz[idx] = true;
                    }
                    pair_edges
                        .entry(normalized_pair(province, down))
                        .or_default()
                        .push(LatticeEdge::Horizontal {
                            i: ((x + width - 1) % width) as u32,
                            j: y as i32,
                        });
                }
            }
        }
    }

    let traced_chains = match trace_law {
        BorderTraceLaw::RegionContours => {
            chains_from_region_contours(&inputs.province_ids, width, height)?
        }
        #[cfg(test)]
        BorderTraceLaw::PairEdgesBaseline => {
            let mut baseline = Vec::new();
            for (&(province_a, province_b), edges) in &pair_edges {
                for chain in chains_from_pair_edges(edges, width as u32)? {
                    baseline.push(TracedBorderChain {
                        province_a,
                        province_b,
                        chain,
                    });
                }
            }
            baseline
        }
    };
    for traced in traced_chains {
        let pair = normalized_pair(traced.province_a, traced.province_b);
        let edge_class = classified[&pair];
        emit_chain_ribbon(
            &mut mesh,
            inputs,
            pair.0,
            pair.1,
            edge_class,
            &traced.chain,
            closed_loop_seam_law,
        )?;
    }

    mesh.stats.chains = mesh.chains.len();
    mesh.stats.vertices = mesh.vertices.len();
    mesh.stats.indices = mesh.indices.len();
    cross_check_coastline_against_water_mask(&mut mesh, inputs, &synth_vert, &synth_horiz);
    Ok(mesh)
}

fn record_edge_stats(
    stats: &mut BorderWorldSynthesisStats,
    inputs: &BorderSynthesisInputs,
    province_a: u32,
    province_b: u32,
    edge: ClassifiedBorderEdge,
) {
    stats.raster_edges += 1;
    stats.total_length += 1.0;
    bump_count(&mut stats.edge_count_by_rank, edge.rank.as_str());
    bump_count(
        &mut stats.edge_count_by_water_class,
        edge.water_class.as_str(),
    );
    bump_count(&mut stats.edge_count_by_material, edge.material.as_str());
    *stats
        .length_by_rank
        .entry(edge.rank.as_str().to_string())
        .or_insert(0.0) += 1.0;
    if edge.water_class == BorderWaterClass::LandWater {
        stats.coastline_edges += 1;
    }
    if edge.material == BorderMaterial::Impassable
        && inputs
            .impassable_pairs
            .contains(&normalized_pair(province_a, province_b))
    {
        stats.impassable_adjacency_edges += 1;
    }
}

struct ChainCorners {
    corners: Vec<Corner>,
    junctions: Vec<bool>,
    open_endpoint_stations: [bool; 2],
    engine_record_stations: bool,
    closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BorderTraceLaw {
    RegionContours,
    #[cfg(test)]
    PairEdgesBaseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClosedLoopSeamLaw {
    CompleteStations,
    #[cfg(test)]
    MidpointOnly,
}

struct TracedBorderChain {
    province_a: u32,
    province_b: u32,
    chain: ChainCorners,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
enum ContourDirection {
    Left = 0,
    Top = 1,
    Right = 2,
    Bottom = 3,
}

#[derive(Clone, Copy, Debug)]
struct ContourProbe {
    dx: i32,
    dy: i32,
    next_direction: ContourDirection,
}

const CONTOUR_TURN_PROBES: [[ContourProbe; 3]; 4] = [
    [
        ContourProbe {
            dx: -1,
            dy: -1,
            next_direction: ContourDirection::Bottom,
        },
        ContourProbe {
            dx: 0,
            dy: -1,
            next_direction: ContourDirection::Left,
        },
        ContourProbe {
            dx: 0,
            dy: 0,
            next_direction: ContourDirection::Top,
        },
    ],
    [
        ContourProbe {
            dx: 1,
            dy: -1,
            next_direction: ContourDirection::Left,
        },
        ContourProbe {
            dx: 1,
            dy: 0,
            next_direction: ContourDirection::Top,
        },
        ContourProbe {
            dx: 0,
            dy: 0,
            next_direction: ContourDirection::Right,
        },
    ],
    [
        ContourProbe {
            dx: 1,
            dy: 1,
            next_direction: ContourDirection::Top,
        },
        ContourProbe {
            dx: 0,
            dy: 1,
            next_direction: ContourDirection::Right,
        },
        ContourProbe {
            dx: 0,
            dy: 0,
            next_direction: ContourDirection::Bottom,
        },
    ],
    [
        ContourProbe {
            dx: -1,
            dy: 1,
            next_direction: ContourDirection::Right,
        },
        ContourProbe {
            dx: -1,
            dy: 0,
            next_direction: ContourDirection::Bottom,
        },
        ContourProbe {
            dx: 0,
            dy: 0,
            next_direction: ContourDirection::Left,
        },
    ],
];

#[derive(Clone, Copy, Debug)]
struct ContourSegmentEdge {
    current_province: u32,
    neighbor_province: u32,
    start: Corner,
    end: Corner,
}

#[derive(Debug)]
struct ContourRecord {
    key: (u32, u32),
    next: Option<usize>,
    corners: Vec<Corner>,
    closed: bool,
}

#[derive(Default)]
struct ContourRecordLedger {
    records: Vec<ContourRecord>,
    heads: BTreeMap<(u32, u32), Option<usize>>,
}

impl ContourRecordLedger {
    fn create_record(&mut self, key: (u32, u32), edge: ContourSegmentEdge) -> usize {
        let record_index = self.records.len();
        let next = self.heads.get(&key).copied().flatten();
        self.records.push(ContourRecord {
            key,
            next,
            corners: vec![edge.start, edge.end],
            closed: false,
        });
        self.heads.insert(key, Some(record_index));
        record_index
    }

    fn append_edge(&mut self, record_index: usize, edge: ContourSegmentEdge) -> Result<()> {
        let record = &mut self.records[record_index];
        if record.corners.last().copied() != Some(edge.start) {
            return Err(format!(
                "border world synthesis blocked: RE contour record for ordered pair ({},{}) is not corner-contiguous",
                record.key.0, record.key.1
            )
            .into());
        }
        record.corners.push(edge.end);
        Ok(())
    }

    fn append_contour(
        &mut self,
        contour: &[Option<ContourSegmentEdge>],
        labels: &[u32],
        width: usize,
        height: usize,
    ) -> Result<()> {
        let mut active_key = None;
        let mut active_record = None;
        let mut first_record = None;
        let mut closure_eligible = false;

        for (contour_index, edge) in contour.iter().copied().enumerate() {
            let Some(edge) = edge else {
                active_key = None;
                active_record = None;
                continue;
            };
            let key = (edge.current_province, edge.neighbor_province);
            if active_key == Some(key) {
                self.append_edge(
                    active_record.expect("an active key always has an active record"),
                    edge,
                )?;
                continue;
            }

            let record_index = self.create_record(key, edge);
            active_key = Some(key);
            active_record = Some(record_index);
            if first_record.is_none() {
                first_record = Some(record_index);
                closure_eligible = contour_index == 0
                    && !corner_is_multi_region_junction(edge.start, labels, width, height);
            }
        }

        if !closure_eligible {
            return Ok(());
        }
        let Some(first_record_index) = first_record else {
            return Ok(());
        };
        let Some(last_record_index) = active_record else {
            return Ok(());
        };
        if self.records[first_record_index].key != self.records[last_record_index].key {
            return Err(format!(
                "border world synthesis blocked: eligible RE contour seam changed ordered pair from ({},{}) to ({},{})",
                self.records[first_record_index].key.0,
                self.records[first_record_index].key.1,
                self.records[last_record_index].key.0,
                self.records[last_record_index].key.1
            )
            .into());
        }

        if first_record_index == last_record_index {
            let record = &mut self.records[first_record_index];
            if record.corners.first() != record.corners.last() {
                return Err(format!(
                    "border world synthesis blocked: one-record RE contour ({},{}) did not close",
                    record.key.0, record.key.1
                )
                .into());
            }
            record.closed = true;
            return Ok(());
        }

        if last_record_index + 1 != self.records.len() {
            return Err(
                "border world synthesis blocked: RE contour seam record was not the allocation tail"
                    .into(),
            );
        }
        let last_key = self.records[last_record_index].key;
        if self.heads.get(&last_key).copied().flatten() != Some(last_record_index) {
            return Err(format!(
                "border world synthesis blocked: RE contour seam record for ordered pair ({},{}) was not the tree-list head",
                last_key.0, last_key.1
            )
            .into());
        }
        let last_record = self
            .records
            .pop()
            .expect("the checked allocation-tail seam record exists");
        self.heads.insert(last_key, last_record.next);

        let first_record = &mut self.records[first_record_index];
        if last_record.corners.last().copied() != first_record.corners.first().copied() {
            return Err(format!(
                "border world synthesis blocked: RE contour seam for ordered pair ({},{}) is not corner-contiguous",
                last_key.0, last_key.1
            )
            .into());
        }
        let mut merged = last_record.corners;
        merged.extend(first_record.corners.iter().copied().skip(1));
        first_record.corners = merged;
        Ok(())
    }

    fn into_chains(self) -> Result<Vec<TracedBorderChain>> {
        let mut chains = Vec::with_capacity(self.records.len());
        let mut emitted = vec![false; self.records.len()];
        for (key, head) in self.heads {
            let mut record_index = head;
            while let Some(index) = record_index {
                if index >= self.records.len() {
                    return Err(format!(
                        "border world synthesis blocked: RE contour ledger key ({},{}) references missing record {index}",
                        key.0, key.1
                    )
                    .into());
                }
                if emitted[index] {
                    return Err(format!(
                        "border world synthesis blocked: RE contour ledger key ({},{}) contains a record cycle or duplicate",
                        key.0, key.1
                    )
                    .into());
                }
                let record = &self.records[index];
                if record.key != key {
                    return Err(format!(
                        "border world synthesis blocked: RE contour ledger key ({},{}) links record owned by ({},{})",
                        key.0, key.1, record.key.0, record.key.1
                    )
                    .into());
                }
                emitted[index] = true;
                let pair = normalized_pair(record.key.0, record.key.1);
                chains.push(TracedBorderChain {
                    province_a: pair.0,
                    province_b: pair.1,
                    chain: ChainCorners {
                        junctions: vec![false; record.corners.len()],
                        corners: record.corners.clone(),
                        open_endpoint_stations: [false; 2],
                        engine_record_stations: true,
                        closed: record.closed,
                    },
                });
                record_index = record.next;
            }
        }
        if let Some(index) = emitted.iter().position(|was_emitted| !was_emitted) {
            let record = &self.records[index];
            return Err(format!(
                "border world synthesis blocked: RE contour ledger record {index} for ordered pair ({},{}) is unreachable",
                record.key.0, record.key.1
            )
            .into());
        }
        Ok(chains)
    }
}

/// deliberately absent here, matching the four bounds checks at
fn connected_region_labels(
    province_ids: &[u32],
    width: usize,
    height: usize,
) -> Result<(Vec<u32>, u32)> {
    let mut labels = vec![0u32; province_ids.len()];
    let mut label_count = 0u32;
    let mut stack = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let seed = y * width + x;
            if labels[seed] != 0 {
                continue;
            }
            label_count = label_count.checked_add(1).ok_or(
                "border world synthesis blocked: connected-region label count overflowed u32",
            )?;
            let province = province_ids[seed];
            labels[seed] = label_count;
            stack.push((x, y));

            while let Some((cx, cy)) = stack.pop() {
                let neighbors = [
                    (cx as i64, cy as i64 - 1),
                    (cx as i64, cy as i64 + 1),
                    (cx as i64 - 1, cy as i64),
                    (cx as i64 + 1, cy as i64),
                ];
                for (nx, ny) in neighbors {
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    let index = ny * width + nx;
                    if labels[index] == 0 && province_ids[index] == province {
                        labels[index] = label_count;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
    Ok((labels, label_count))
}

fn connected_region_boundary_flags(labels: &[u32], width: usize, height: usize) -> Vec<[bool; 4]> {
    let mut flags = vec![[false; 4]; labels.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let label = labels[index];
            if x > 0 && labels[index - 1] != label {
                flags[index][ContourDirection::Left as usize] = true;
            }
            if y > 0 && labels[index - width] != label {
                flags[index][ContourDirection::Top as usize] = true;
            }
            if x + 1 < width && labels[index + 1] != label {
                flags[index][ContourDirection::Right as usize] = true;
            }
            if y + 1 < height && labels[index + width] != label {
                flags[index][ContourDirection::Bottom as usize] = true;
            }
        }
    }
    flags
}

fn contour_start_direction(flags: [bool; 4]) -> Option<ContourDirection> {
    [
        ContourDirection::Top,
        ContourDirection::Right,
        ContourDirection::Bottom,
        ContourDirection::Left,
    ]
    .into_iter()
    .find(|direction| flags[*direction as usize])
}

fn contour_across_pixel(
    x: usize,
    y: usize,
    direction: ContourDirection,
    width: usize,
    height: usize,
) -> Option<(usize, usize)> {
    match direction {
        ContourDirection::Left => Some(((x + width - 1) % width, y)),
        ContourDirection::Top => y.checked_sub(1).map(|ny| (x, ny)),
        ContourDirection::Right => Some(((x + 1) % width, y)),
        ContourDirection::Bottom => (y + 1 < height).then_some((x, y + 1)),
    }
}

fn directed_lattice_edge(
    x: usize,
    y: usize,
    direction: ContourDirection,
    width: u32,
) -> (Corner, Corner) {
    let x = x as u32;
    let y = y as i32;
    let west_i = (x + width - 1) % width;
    let edge = match direction {
        ContourDirection::Left => LatticeEdge::Vertical {
            i: west_i,
            j: y - 1,
        },
        ContourDirection::Top => LatticeEdge::Horizontal {
            i: west_i,
            j: y - 1,
        },
        ContourDirection::Right => LatticeEdge::Vertical { i: x, j: y - 1 },
        ContourDirection::Bottom => LatticeEdge::Horizontal { i: west_i, j: y },
    };
    let (a, b) = edge.corners(width);
    match direction {
        ContourDirection::Top | ContourDirection::Right => (a, b),
        ContourDirection::Bottom | ContourDirection::Left => (b, a),
    }
}

fn trace_connected_region_contour(
    province_ids: &[u32],
    labels: &[u32],
    width: usize,
    height: usize,
    start_x: usize,
    start_y: usize,
    start_direction: ContourDirection,
) -> Result<Vec<Option<ContourSegmentEdge>>> {
    use std::collections::HashSet;

    let region_label = labels[start_y * width + start_x];
    let region_province = province_ids[start_y * width + start_x];
    let start = (start_x, start_y, start_direction);
    let mut state = start;
    let mut seen = HashSet::from([start]);
    let mut contour = Vec::new();

    loop {
        let (x, y, direction) = state;
        let mut next_state = None;
        for probe in CONTOUR_TURN_PROBES[direction as usize] {
            let nx = x as i64 + probe.dx as i64;
            let ny = y as i64 + probe.dy as i64;
            if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if labels[ny * width + nx] == region_label {
                next_state = Some((nx, ny, probe.next_direction));
                break;
            }
        }
        let Some(next_state) = next_state else {
            return Err(format!(
                "border world synthesis blocked: RE contour walk found no turn probe for region {region_label} at pixel ({x},{y}) direction {direction:?}"
            )
            .into());
        };

        if let Some((across_x, across_y)) = contour_across_pixel(x, y, direction, width, height) {
            let across_index = across_y * width + across_x;
            let neighbor_label = labels[across_index];
            if neighbor_label == region_label {
                contour.push(None);
            } else if neighbor_label <= region_label {
                let neighbor_province = province_ids[across_index];
                let (start, end) = directed_lattice_edge(x, y, direction, width as u32);
                contour.push(Some(ContourSegmentEdge {
                    current_province: region_province,
                    neighbor_province,
                    start,
                    end,
                }));
            } else {
                contour.push(None);
            }
        } else {
            contour.push(None);
        }

        state = next_state;
        if state == start {
            break;
        }
        if !seen.insert(state) {
            return Err(format!(
                "border world synthesis blocked: RE contour walk repeated a non-start state for region {region_label} at pixel ({},{}) direction {:?}",
                state.0, state.1, state.2
            )
            .into());
        }
    }
    Ok(contour)
}

fn corner_is_multi_region_junction(
    corner: Corner,
    labels: &[u32],
    width: usize,
    height: usize,
) -> bool {
    let columns = [corner.i as usize, (corner.i as usize + 1) % width];
    let rows = [corner.j, corner.j + 1];
    let mut unique = [0u32; 4];
    let mut unique_count = 0usize;
    for row in rows {
        if row < 0 || row >= height as i32 {
            continue;
        }
        for column in columns {
            let label = labels[row as usize * width + column];
            if !unique[..unique_count].contains(&label) {
                unique[unique_count] = label;
                unique_count += 1;
            }
        }
    }
    unique_count > 2
}

fn chains_from_region_contours(
    province_ids: &[u32],
    width: usize,
    height: usize,
) -> Result<Vec<TracedBorderChain>> {
    // equivalent because it changes component discovery and contour ownership.
    let engine_province_ids = province_ids
        .chunks_exact(width)
        .rev()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let (labels, label_count) = connected_region_labels(&engine_province_ids, width, height)?;
    let boundary_flags = connected_region_boundary_flags(&labels, width, height);
    let mut traced = vec![false; label_count as usize + 1];
    let mut ledger = ContourRecordLedger::default();

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let label = labels[index] as usize;
            if traced[label] {
                continue;
            }
            let Some(start_direction) = contour_start_direction(boundary_flags[index]) else {
                continue;
            };
            traced[label] = true;
            let contour = trace_connected_region_contour(
                &engine_province_ids,
                &labels,
                width,
                height,
                x,
                y,
                start_direction,
            )?;
            ledger.append_contour(&contour, &labels, width, height)?;
        }
    }
    let mut chains = ledger.into_chains()?;
    for segment in &mut chains {
        segment.chain.junctions = segment
            .chain
            .corners
            .iter()
            .map(|&corner| corner_is_multi_region_junction(corner, &labels, width, height))
            .collect();
        segment.chain.open_endpoint_stations = [
            segment.chain.junctions.first().copied().unwrap_or(false),
            segment.chain.junctions.last().copied().unwrap_or(false),
        ];
        for corner in &mut segment.chain.corners {
            corner.j = height as i32 - 2 - corner.j;
        }
    }
    Ok(chains)
}

fn chains_from_pair_edges(edges: &[LatticeEdge], width: u32) -> Result<Vec<ChainCorners>> {
    use std::collections::HashMap;
    let mut adjacency: HashMap<Corner, Vec<usize>> = HashMap::new();
    for (edge_index, edge) in edges.iter().enumerate() {
        let (a, b) = edge.corners(width);
        adjacency.entry(a).or_default().push(edge_index);
        adjacency.entry(b).or_default().push(edge_index);
    }

    let mut visited = vec![false; edges.len()];
    let mut chains = Vec::new();

    let walk = |start: Corner,
                first_edge: usize,
                visited: &mut Vec<bool>,
                adjacency: &HashMap<Corner, Vec<usize>>|
     -> Vec<Corner> {
        let mut corners = vec![start];
        let mut cur = start;
        let mut edge_index = first_edge;
        loop {
            visited[edge_index] = true;
            cur = edges[edge_index].other_corner(cur, width);
            corners.push(cur);
            let neighbors = &adjacency[&cur];
            if neighbors.len() != 2 {
                break;
            }
            let Some(&next) = neighbors.iter().find(|&&e| !visited[e]) else {
                break;
            };
            edge_index = next;
        }
        corners
    };

    let mut start_corners: Vec<Corner> = adjacency
        .iter()
        .filter(|(_, edge_list)| edge_list.len() != 2)
        .map(|(&corner, _)| corner)
        .collect();
    start_corners.sort();
    for start in start_corners {
        let outgoing: Vec<usize> = adjacency[&start].clone();
        for edge_index in outgoing {
            if visited[edge_index] {
                continue;
            }
            let corners = walk(start, edge_index, &mut visited, &adjacency);
            chains.push(ChainCorners {
                junctions: vec![false; corners.len()],
                corners,
                open_endpoint_stations: [true; 2],
                engine_record_stations: false,
                closed: false,
            });
        }
    }

    let mut loop_starts: Vec<(Corner, usize)> = Vec::new();
    for (edge_index, edge) in edges.iter().enumerate() {
        if !visited[edge_index] {
            let (a, b) = edge.corners(width);
            loop_starts.push((a.min(b), edge_index));
        }
    }
    loop_starts.sort();
    for (start, _) in loop_starts {
        let outgoing: Vec<usize> = adjacency[&start]
            .iter()
            .copied()
            .filter(|&e| !visited[e])
            .collect();
        let Some(&first_edge) = outgoing.first() else {
            continue;
        };
        let corners = walk(start, first_edge, &mut visited, &adjacency);
        if corners.len() < 2 {
            continue;
        }
        if corners.first() != corners.last() {
            return Err(format!(
                "border world synthesis blocked: leftover chain starting at corner \
                 ({}, {}) did not close into a loop",
                start.i, start.j
            )
            .into());
        }
        chains.push(ChainCorners {
            junctions: vec![false; corners.len()],
            corners,
            open_endpoint_stations: [true; 2],
            engine_record_stations: false,
            closed: true,
        });
    }

    for chain in &mut chains {
        if !chain.closed {
            let first = chain.corners.first().expect("chain has corners");
            let last = chain.corners.last().expect("chain has corners");
            if last < first {
                chain.corners.reverse();
                chain.junctions.reverse();
            }
        }
    }
    chains.sort_by(|a, b| a.corners.first().cmp(&b.corners.first()));
    Ok(chains)
}

fn box3_fixed_pass(rows: &mut [[f64; 2]]) {
    if rows.len() < 3 {
        return;
    }
    let src: Vec<[f64; 2]> = rows.to_vec();
    for i in 1..src.len() - 1 {
        rows[i] = [
            (src[i - 1][0] + src[i][0] + src[i + 1][0]) / 3.0,
            (src[i - 1][1] + src[i][1] + src[i + 1][1]) / 3.0,
        ];
    }
}

fn smooth_open_rows(rows: &mut [[f64; 2]], wide: bool) {
    if rows.len() < 3 {
        return;
    }
    for pass in 0..2 {
        if pass != 0 && !wide {
            continue;
        }
        let src: Vec<[f64; 2]> = rows.to_vec();
        for i in 1..src.len() - 1 {
            let radius = if wide {
                i.min(src.len() - 1 - i).min(6)
            } else {
                1
            };
            let count = 2 * radius + 1;
            if wide {
                let mut sum = [0.0f32, 0.0f32];
                for p in &src[i - radius..=i + radius] {
                    sum[0] += p[0] as f32;
                    sum[1] += p[1] as f32;
                }
                let count = count as f32;
                rows[i] = [f64::from(sum[0] / count), f64::from(sum[1] / count)];
            } else {
                let mut sum = [0.0f64, 0.0f64];
                for p in &src[i - radius..=i + radius] {
                    sum[0] += p[0];
                    sum[1] += p[1];
                }
                rows[i] = [sum[0] / count as f64, sum[1] / count as f64];
            }
        }
    }
}

fn engine_record_stations(
    points: &[[f64; 2]],
    junctions: &[bool],
    endpoint_stations: [bool; 2],
) -> Vec<[f64; 2]> {
    let edge_count = points.len() - 1;
    debug_assert_eq!(points.len(), junctions.len());
    let interior_junctions = junctions[1..edge_count]
        .iter()
        .filter(|&&value| value)
        .count();
    let mut stations: Vec<[f64; 2]> = Vec::with_capacity(
        edge_count
            + usize::from(endpoint_stations[0])
            + usize::from(endpoint_stations[1])
            + 2 * interior_junctions,
    );
    if endpoint_stations[0] {
        stations.push(points[0]);
    }
    for k in 0..edge_count {
        let a = points[k];
        let b = points[k + 1];
        stations.push([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
        if k + 1 < edge_count && junctions[k + 1] {
            stations.push(points[k + 1]);
            stations.push(points[k + 1]);
        }
    }
    if endpoint_stations[1] {
        stations.push(points[edge_count]);
    }
    stations
}

fn engine_record_rows(
    points: &[[f64; 2]],
    junctions: &[bool],
    water_class: BorderWaterClass,
    endpoint_stations: [bool; 2],
    closed: bool,
) -> Vec<[f64; 2]> {
    let mut stations = engine_record_stations(points, junctions, endpoint_stations);
    if closed {
        stations.push(stations[0]);
    }
    smooth_open_rows(&mut stations, water_class == BorderWaterClass::WaterWater);
    stations
}

fn canonicalize_engine_record_x(points: &mut [[f64; 2]], endpoint_stations: [bool; 2], width: f64) {
    let first_station_x = if endpoint_stations[0] {
        points[0][0]
    } else {
        (points[0][0] + points[1][0]) * 0.5
    };
    let wrap_turns = ((first_station_x + 0.5) / width).floor();
    if wrap_turns != 0.0 {
        let shift = -wrap_turns * width;
        for point in points {
            point[0] += shift;
        }
    }
}

fn open_chain_rows_with_endpoints(
    points: &[[f64; 2]],
    junctions: &[bool],
    water_class: BorderWaterClass,
    endpoint_stations: [bool; 2],
) -> Vec<[f64; 2]> {
    engine_record_rows(points, junctions, water_class, endpoint_stations, false)
}

fn open_chain_rows(
    points: &[[f64; 2]],
    junctions: &[bool],
    water_class: BorderWaterClass,
) -> Vec<[f64; 2]> {
    open_chain_rows_with_endpoints(points, junctions, water_class, [true; 2])
}

fn closed_loop_rows_for_class(
    points: &[[f64; 2]],
    junctions: &[bool],
    water_class: BorderWaterClass,
) -> Vec<[f64; 2]> {
    let m = points.len() - 1;
    if m < 2 {
        return points.to_vec();
    }
    debug_assert_eq!(points.len(), junctions.len());
    let cycle = &points[..m];
    let mut area = 0.0f64;
    for k in 0..m {
        let p = cycle[k];
        let q = cycle[(k + 1) % m];
        area += p[0] * q[1] - q[0] * p[1];
    }
    let (cyc, junction_cycle): (Vec<[f64; 2]>, Vec<bool>) = if area < 0.0 {
        (
            cycle.iter().rev().copied().collect(),
            junctions[..m].iter().rev().copied().collect(),
        )
    } else {
        (cycle.to_vec(), junctions[..m].to_vec())
    };
    let mids: Vec<[f64; 2]> = (0..m)
        .map(|k| {
            let p = cyc[k];
            let q = cyc[(k + 1) % m];
            [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5]
        })
        .collect();
    let mut anchor = 0usize;
    let mut anchor_is_junction = false;
    for k in 1..m {
        let (z, x) = (mids[k][1], mids[k][0]);
        let (bz, bx) = (mids[anchor][1], mids[anchor][0]);
        if z < bz || (z == bz && x < bx) {
            anchor = k;
        }
    }
    for k in 0..m {
        if !junction_cycle[k] {
            continue;
        }
        let (z, x) = (cyc[k][1], cyc[k][0]);
        let best = if anchor_is_junction {
            cyc[anchor]
        } else {
            mids[anchor]
        };
        if z < best[1] || (z == best[1] && x < best[0]) {
            anchor = k;
            anchor_is_junction = true;
        }
    }
    let junction_count = junction_cycle.iter().filter(|&&value| value).count();
    let mut rows = Vec::with_capacity(m + 2 * junction_count + 1);
    if anchor_is_junction {
        rows.push(cyc[anchor]);
    }
    for step in 0..m {
        let edge = (anchor + step) % m;
        rows.push(mids[edge]);
        let corner_after_edge = (edge + 1) % m;
        if junction_cycle[corner_after_edge] && !(anchor_is_junction && corner_after_edge == anchor)
        {
            rows.push(cyc[corner_after_edge]);
            rows.push(cyc[corner_after_edge]);
        }
    }
    rows.push(if anchor_is_junction {
        cyc[anchor]
    } else {
        rows[0]
    });
    if water_class == BorderWaterClass::WaterWater {
        smooth_open_rows(&mut rows, true);
    } else {
        box3_fixed_pass(&mut rows);
    }
    rows
}

fn closed_loop_rows(points: &[[f64; 2]], junctions: &[bool]) -> Vec<[f64; 2]> {
    closed_loop_rows_for_class(points, junctions, BorderWaterClass::LandLand)
}

#[cfg(test)]
fn midpoint_anchor_closed_loop_rows(points: &[[f64; 2]], junctions: &[bool]) -> Vec<[f64; 2]> {
    let m = points.len() - 1;
    let cycle = &points[..m];
    let area = (0..m)
        .map(|k| {
            let p = cycle[k];
            let q = cycle[(k + 1) % m];
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>();
    let (cyc, junction_cycle): (Vec<[f64; 2]>, Vec<bool>) = if area < 0.0 {
        (
            cycle.iter().rev().copied().collect(),
            junctions[..m].iter().rev().copied().collect(),
        )
    } else {
        (cycle.to_vec(), junctions[..m].to_vec())
    };
    let mids: Vec<[f64; 2]> = (0..m)
        .map(|k| {
            let p = cyc[k];
            let q = cyc[(k + 1) % m];
            [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5]
        })
        .collect();
    let anchor = (1..m).fold(0usize, |best, k| {
        let (z, x) = (mids[k][1], mids[k][0]);
        let (best_z, best_x) = (mids[best][1], mids[best][0]);
        if z < best_z || (z == best_z && x < best_x) {
            k
        } else {
            best
        }
    });
    let mut rows = Vec::new();
    for step in 0..m {
        let edge = (anchor + step) % m;
        rows.push(mids[edge]);
        let corner = (edge + 1) % m;
        if junction_cycle[corner] {
            rows.push(cyc[corner]);
            rows.push(cyc[corner]);
        }
    }
    rows.push(rows[0]);
    box3_fixed_pass(&mut rows);
    rows
}

fn emit_chain_ribbon(
    mesh: &mut BorderWorldMesh,
    inputs: &BorderSynthesisInputs,
    province_a: u32,
    province_b: u32,
    edge_class: ClassifiedBorderEdge,
    chain: &ChainCorners,
    closed_loop_seam_law: ClosedLoopSeamLaw,
) -> Result<()> {
    let width = inputs.width as f64;
    let height = inputs.height as f64;

    let mut points: Vec<[f64; 2]> = Vec::with_capacity(chain.corners.len());
    let mut x_offset = 0.0f64;
    for (index, corner) in chain.corners.iter().enumerate() {
        let base_x = corner.i as f64 + 0.5;
        let z = (height - 1.5) - corner.j as f64;
        let mut x = base_x + x_offset;
        if index > 0 {
            let prev_x = points[index - 1][0];
            if x - prev_x > width * 0.5 {
                x_offset -= width;
                x -= width;
            } else if prev_x - x > width * 0.5 {
                x_offset += width;
                x += width;
            }
        }
        points.push([x, z]);
    }
    if chain.engine_record_stations {
        canonicalize_engine_record_x(&mut points, chain.open_endpoint_stations, width);
    }
    for window in points.windows(2) {
        let manhattan = (window[1][0] - window[0][0]).abs() + (window[1][1] - window[0][1]).abs();
        if (manhattan - 1.0).abs() > 1e-9 {
            return Err(format!(
                "border world synthesis blocked: chain for pair ({province_a},{province_b}) has a \
                 non-unit lattice step ({manhattan})"
            )
            .into());
        }
    }

    let edge_count = points.len() - 1;
    let rows: Vec<[f64; 2]> = if chain.engine_record_stations
        && (!chain.closed || closed_loop_seam_law == ClosedLoopSeamLaw::CompleteStations)
    {
        engine_record_rows(
            &points,
            &chain.junctions,
            edge_class.water_class,
            chain.open_endpoint_stations,
            chain.closed,
        )
    } else if chain.closed {
        match closed_loop_seam_law {
            ClosedLoopSeamLaw::CompleteStations => {
                closed_loop_rows_for_class(&points, &chain.junctions, edge_class.water_class)
            }
            #[cfg(test)]
            ClosedLoopSeamLaw::MidpointOnly => {
                midpoint_anchor_closed_loop_rows(&points, &chain.junctions)
            }
        }
    } else {
        open_chain_rows_with_endpoints(
            &points,
            &chain.junctions,
            edge_class.water_class,
            chain.open_endpoint_stations,
        )
    };

    let row_count = rows.len();
    let half_width = f64::from(inputs.border_width) * 0.5;
    let half_width_f32 = inputs.border_width * 0.5;
    let first_vertex = u32::try_from(mesh.vertices.len())
        .map_err(|_| "border world synthesis blocked: too many vertices for u32 indices")?;
    let first_index = u32::try_from(mesh.indices.len())
        .map_err(|_| "border world synthesis blocked: too many indices for u32")?;

    for k in 0..row_count {
        let prev = &rows[k.saturating_sub(1)];
        let next = &rows[(k + 1).min(row_count - 1)];
        let water_float_emit = edge_class.water_class == BorderWaterClass::WaterWater;
        if water_float_emit {
            let dz = next[1] as f32 - prev[1] as f32;
            let neg_dx = -(next[0] as f32 - prev[0] as f32);
            let len = (neg_dx * neg_dx + dz * dz).sqrt();
            if len <= f32::EPSILON {
                return Err(format!(
                    "border world synthesis blocked: degenerate ribbon direction at row {k} of the \
                     chain for pair ({province_a},{province_b})"
                )
                .into());
            }
            let row_x = rows[k][0] as f32;
            let row_z = rows[k][1] as f32;
            let perp_x = (half_width_f32 * dz) / len;
            let perp_z = (half_width_f32 * neg_dx) / len;
            for (side, x, z) in [
                (0i32, row_x + perp_x, row_z + perp_z),
                (1i32, row_x - perp_x, row_z - perp_z),
            ] {
                mesh.vertices.push(BorderWorldVertex {
                    position: [x, sample_border_height(&inputs.heightmap, x, z), z],
                    uv: [side, k as i32],
                });
            }
            continue;
        }
        let dx = next[0] - prev[0];
        let dz = next[1] - prev[1];
        let len = (dx * dx + dz * dz).sqrt();
        if len <= f64::EPSILON {
            return Err(format!(
                "border world synthesis blocked: degenerate ribbon direction at row {k} of the \
                 chain for pair ({province_a},{province_b})"
            )
            .into());
        }
        let row_x = rows[k][0];
        let row_z = rows[k][1];
        let perp_x = dz / len;
        let perp_z = -dx / len;
        for (side, sign) in [(0i32, 1.0f64), (1i32, -1.0f64)] {
            let x = row_x + perp_x * half_width * sign;
            let z = row_z + perp_z * half_width * sign;
            mesh.vertices.push(BorderWorldVertex {
                position: [
                    x as f32,
                    sample_border_height(&inputs.heightmap, x as f32, z as f32),
                    z as f32,
                ],
                uv: [side, k as i32],
            });
        }
    }

    for k in 0..row_count - 1 {
        let base = first_vertex + 2 * k as u32;
        mesh.indices
            .extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);
    }

    let chain_record = BorderChain {
        province_a,
        province_b,
        state_a: edge_class.state_a,
        state_b: edge_class.state_b,
        rank: edge_class.rank,
        water_class: edge_class.water_class,
        material: edge_class.material,
        closed: chain.closed,
        manhattan_length: edge_count as u32,
        rows: row_count as u32,
        first_vertex,
        first_index,
    };
    if chain.closed {
        mesh.stats.closed_loop_chains += 1;
    }
    bump_count(
        &mut mesh.stats.chain_count_by_material,
        chain_record.material.as_str(),
    );
    mesh.chains.push(chain_record);
    Ok(())
}

fn cross_check_coastline_against_water_mask(
    mesh: &mut BorderWorldMesh,
    inputs: &BorderSynthesisInputs,
    synth_vert: &[bool],
    synth_horiz: &[bool],
) {
    let width = inputs.width;
    let height = inputs.height;
    let water_mask: Vec<bool> = inputs
        .province_ids
        .iter()
        .map(|province| renderer_pixel_is_water(&inputs.definitions, *province))
        .collect();

    let mut water_mask_coastline_edges = 0usize;
    let mut disagreement = 0usize;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let here_water = water_mask[idx];
            let province = inputs.province_ids[idx];

            let right_x = (x + 1) % width;
            let right = inputs.province_ids[y * width + right_x];
            if province != right {
                let mask_coast = here_water != water_mask[y * width + right_x];
                if mask_coast {
                    water_mask_coastline_edges += 1;
                }
                if mask_coast != synth_vert[idx] {
                    disagreement += 1;
                }
            }

            if y + 1 < height {
                let down = inputs.province_ids[(y + 1) * width + x];
                if province != down {
                    let mask_coast = here_water != water_mask[(y + 1) * width + x];
                    if mask_coast {
                        water_mask_coastline_edges += 1;
                    }
                    if mask_coast != synth_horiz[idx] {
                        disagreement += 1;
                    }
                }
            }
        }
    }

    mesh.stats.water_mask_coastline_edges = water_mask_coastline_edges;
    mesh.stats.coastline_disagreement_edges = disagreement;
}

fn renderer_pixel_is_water(definitions: &BTreeMap<u32, ProvinceDefinition>, province: u32) -> bool {
    definitions
        .get(&province)
        .map(|definition| definition.province_type == "sea" || definition.province_type == "lake")
        .unwrap_or(true)
}

fn normalized_pair(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn classify_border_edge(
    inputs: &BorderSynthesisInputs,
    province_a: u32,
    province_b: u32,
) -> Result<ClassifiedBorderEdge> {
    let water_a = province_is_water(inputs, province_a)?;
    let water_b = province_is_water(inputs, province_b)?;
    let water_class = match (water_a, water_b) {
        (false, false) => BorderWaterClass::LandLand,
        (true, true) => BorderWaterClass::WaterWater,
        _ => BorderWaterClass::LandWater,
    };

    if water_class != BorderWaterClass::LandLand {
        let material = match water_class {
            BorderWaterClass::LandWater => BorderMaterial::SeaCoast,
            BorderWaterClass::WaterWater => BorderMaterial::SeaRegion,
            BorderWaterClass::LandLand => unreachable!(),
        };
        return Ok(ClassifiedBorderEdge {
            rank: BorderRank::Province,
            water_class,
            material,
            state_a: None,
            state_b: None,
        });
    }

    let state_a = land_province_state(inputs, province_a)?;
    let state_b = land_province_state(inputs, province_b)?;

    let pair = normalized_pair(province_a, province_b);
    if inputs.impassable_pairs.contains(&pair) {
        return Ok(ClassifiedBorderEdge {
            rank: BorderRank::State,
            water_class,
            material: BorderMaterial::Impassable,
            state_a: Some(state_a),
            state_b: Some(state_b),
        });
    }

    if state_a == state_b {
        return Ok(ClassifiedBorderEdge {
            rank: BorderRank::Province,
            water_class,
            material: BorderMaterial::Province,
            state_a: Some(state_a),
            state_b: Some(state_b),
        });
    }

    let impassable = state_impassable(inputs, state_a)? || state_impassable(inputs, state_b)?;
    if impassable {
        return Ok(ClassifiedBorderEdge {
            rank: BorderRank::State,
            water_class,
            material: BorderMaterial::Impassable,
            state_a: Some(state_a),
            state_b: Some(state_b),
        });
    }

    let owner_a = state_owner(inputs, state_a)?;
    let owner_b = state_owner(inputs, state_b)?;
    let rank = if owner_a != owner_b {
        BorderRank::Country
    } else {
        BorderRank::State
    };
    Ok(ClassifiedBorderEdge {
        rank,
        water_class,
        material: match rank {
            BorderRank::Province => BorderMaterial::Province,
            BorderRank::State => BorderMaterial::State,
            BorderRank::Country => BorderMaterial::Country,
        },
        state_a: Some(state_a),
        state_b: Some(state_b),
    })
}

fn province_is_water(inputs: &BorderSynthesisInputs, province: u32) -> Result<bool> {
    let definition = inputs.definitions.get(&province).ok_or_else(|| {
        format!(
            "border world synthesis blocked: province id {province} has no {} row",
            BORDER_DEFINITION_CSV_SOURCE
        )
    })?;
    match definition.province_type.as_str() {
        "land" => Ok(false),
        "sea" | "lake" => Ok(true),
        other => Err(format!(
            "border world synthesis blocked: province id {province} has unsupported definition.csv province type {other:?}"
        )
        .into()),
    }
}

fn land_province_state(inputs: &BorderSynthesisInputs, province: u32) -> Result<u32> {
    inputs
        .province_to_state
        .get(&province)
        .copied()
        .ok_or_else(|| {
            format!(
                "border world synthesis blocked: land province id {province} has no 1936 state in {BORDER_HISTORY_STATES_DIR}"
            )
            .into()
        })
}

fn state_impassable(inputs: &BorderSynthesisInputs, state: u32) -> Result<bool> {
    inputs
        .state_meta
        .get(&state)
        .map(|meta| meta.impassable)
        .ok_or_else(|| {
            format!(
                "border world synthesis blocked: province_to_state references missing state {state}"
            )
            .into()
        })
}

fn state_owner(inputs: &BorderSynthesisInputs, state: u32) -> Result<&str> {
    inputs
        .state_owners
        .get(&state)
        .map(String::as_str)
        .ok_or_else(|| {
            format!("border world synthesis blocked: passable land state {state} has no 1936 owner")
                .into()
        })
}

fn sample_border_height(heightmap: &IndexImage, x: f32, world_z: f32) -> f32 {
    let width = heightmap.width as f32;
    let mut map_x = x % width;
    if map_x < 0.0 {
        map_x += width;
    }
    let row_f = ((heightmap.height - 1) as f32 - world_z).clamp(0.0, (heightmap.height - 1) as f32);
    let x0 = (map_x.floor() as usize).min(heightmap.width - 1);
    let y0 = (row_f.floor() as usize).min(heightmap.height - 1);
    let x1 = (x0 + 1) % heightmap.width;
    let y1 = (y0 + 1).min(heightmap.height - 1);
    let fx = map_x - x0 as f32;
    let fy = row_f - y0 as f32;
    let sample = |px: usize, py: usize| heightmap.pixels[py * heightmap.width + px] as f32;
    let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
    let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
    (top * (1.0 - fy) + bottom * fy) * 0.1
}

fn validate_default_map_border_bindings(path: &std::path::Path) -> Result<()> {
    let meta = load_default_map_meta(path)?;
    for (key, expected) in [
        ("definitions", "definition.csv"),
        ("provinces", "provinces.bmp"),
        ("heightmap", "heightmap.bmp"),
    ] {
        let actual = meta.file_references.get(key).ok_or_else(|| {
            format!("border world synthesis blocked: map/default.map missing {key} binding")
        })?;
        if actual != expected {
            return Err(format!(
                "border world synthesis blocked: map/default.map {key} = {actual:?}, expected {expected:?}"
            )
            .into());
        }
    }
    Ok(())
}

fn border_width_from_graphics_defines(path: &std::path::Path) -> Result<f32> {
    let text = read_latin1(path)?;
    let width = parse_lua_number_define(&text, "BORDER_WIDTH")?;
    if !width.is_finite() || width <= 0.0 {
        return Err(format!(
            "border world synthesis blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} BORDER_WIDTH has invalid value {width}"
        )
        .into());
    }
    Ok(width)
}

fn bump_count(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}

fn push_line_range(
    resolver: &GameFileResolver,
    rel: &str,
    first_line: usize,
    last_line: usize,
    out: &mut Vec<BorderSourceExcerpt>,
    seen: &mut BTreeSet<(String, usize)>,
) -> Result<()> {
    let path = resolver.resolve_required(rel, "source excerpt")?;
    let text = read_latin1(&path)?;
    for (line_index, line) in text.lines().enumerate() {
        let line_no = line_index + 1;
        if line_no >= first_line && line_no <= last_line {
            push_excerpt(rel, line_no, line, out, seen);
        }
    }
    Ok(())
}

fn push_first_matching_line(
    resolver: &GameFileResolver,
    rel: &str,
    predicate: impl Fn(&str) -> bool,
    out: &mut Vec<BorderSourceExcerpt>,
    seen: &mut BTreeSet<(String, usize)>,
) -> Result<()> {
    let path = resolver.resolve_required(rel, "source excerpt")?;
    let text = read_latin1(&path)?;
    for (line_index, line) in text.lines().enumerate() {
        if predicate(line) {
            push_excerpt(rel, line_index + 1, line, out, seen);
            return Ok(());
        }
    }
    Err(format!("border source evidence blocked: no matching line in {rel}").into())
}

fn push_context_for_pattern(
    resolver: &GameFileResolver,
    rel: &str,
    pattern: &str,
    before: usize,
    after: usize,
    out: &mut Vec<BorderSourceExcerpt>,
    seen: &mut BTreeSet<(String, usize)>,
) -> Result<()> {
    let path = resolver.resolve_required(rel, "source excerpt")?;
    let text = read_latin1(&path)?;
    let lines = text.lines().collect::<Vec<_>>();
    let Some(index) = lines.iter().position(|line| line.contains(pattern)) else {
        return Err(
            format!("border source evidence blocked: {rel} does not contain {pattern:?}").into(),
        );
    };
    let start = index.saturating_sub(before);
    let end = (index + after).min(lines.len().saturating_sub(1));
    for line_index in start..=end {
        push_excerpt(rel, line_index + 1, lines[line_index], out, seen);
    }
    Ok(())
}

fn push_state_file_example(
    resolver: &GameFileResolver,
    out: &mut Vec<BorderSourceExcerpt>,
    seen: &mut BTreeSet<(String, usize)>,
) -> Result<()> {
    for (rel, path) in resolver.resolve_dir_files(BORDER_HISTORY_STATES_DIR, Some("txt"))? {
        let text = read_latin1(&path)?;
        if text.contains("owner") && text.contains("provinces") {
            let lines = text.lines().collect::<Vec<_>>();
            for pattern in ["id", "owner", "provinces"] {
                if let Some(index) = lines.iter().position(|line| line.contains(pattern)) {
                    let start = index.saturating_sub(1);
                    let end = (index + 2).min(lines.len().saturating_sub(1));
                    for line_index in start..=end {
                        push_excerpt(&rel, line_index + 1, lines[line_index], out, seen);
                    }
                }
            }
            return Ok(());
        }
    }
    Err("border source evidence blocked: no history/states file with owner and provinces".into())
}

fn push_excerpt(
    rel: &str,
    line: usize,
    text: &str,
    out: &mut Vec<BorderSourceExcerpt>,
    seen: &mut BTreeSet<(String, usize)>,
) {
    let key = (rel.to_string(), line);
    if seen.insert(key) {
        out.push(BorderSourceExcerpt {
            source: rel.to_string(),
            line,
            text: text.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    fn definition(kind: &str) -> ProvinceDefinition {
        ProvinceDefinition {
            rgb: [0, 0, 0],
            province_type: kind.to_string(),
            coastal: false,
            terrain: "plains".to_string(),
            continent: 1,
        }
    }

    fn state(provinces: Vec<u32>, impassable: bool) -> StateMeta {
        StateMeta {
            name_key: "STATE_TEST".to_string(),
            provinces,
            victory_points: BTreeMap::new(),
            vp_total: 0,
            state_category: None,
            manpower: 0,
            impassable,
        }
    }

    fn synthetic_inputs() -> BorderSynthesisInputs {
        let width = 4;
        let height = 3;
        let province_ids = vec![1, 1, 3, 4, 2, 2, 3, 4, 5, 5, 5, 4];
        BorderSynthesisInputs {
            province_ids,
            width,
            height,
            definitions: BTreeMap::from([
                (1, definition("land")),
                (2, definition("land")),
                (3, definition("land")),
                (4, definition("land")),
                (5, definition("sea")),
            ]),
            state_meta: BTreeMap::from([
                (10, state(vec![1, 2], false)),
                (20, state(vec![3], false)),
                (30, state(vec![4], false)),
            ]),
            province_to_state: BTreeMap::from([(1, 10), (2, 10), (3, 20), (4, 30)]),
            state_owners: BTreeMap::from([
                (10, "AAA".to_string()),
                (20, "AAA".to_string()),
                (30, "BBB".to_string()),
            ]),
            heightmap: IndexImage {
                width,
                height,
                pixels: vec![100; width * height],
            },
            border_width: 1.5,
            unknown_pixels: 0,
            state_files: 3,
            impassable_pairs: BTreeSet::new(),
            impassable_adjacency_non_land_rows: 0,
        }
    }

    #[test]
    fn synthetic_fixture_classifies_rank_and_water_edges() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        assert!(mesh.chains.iter().any(|chain| chain.province_a == 1
            && chain.province_b == 2
            && chain.rank == BorderRank::Province
            && chain.material == BorderMaterial::Province));
        assert!(mesh.chains.iter().any(|chain| chain.province_a == 1
            && chain.province_b == 3
            && chain.rank == BorderRank::State
            && chain.material == BorderMaterial::State));
        assert!(mesh.chains.iter().any(|chain| chain.province_a == 3
            && chain.province_b == 4
            && chain.rank == BorderRank::Country
            && chain.material == BorderMaterial::Country));
        assert!(mesh.chains.iter().any(|chain| chain.province_a == 2
            && chain.province_b == 5
            && chain.water_class == BorderWaterClass::LandWater
            && chain.material == BorderMaterial::SeaCoast));
        assert_eq!(mesh.stats.coastline_disagreement_edges, 0);
        assert!(mesh.stats.coastline_edges > 0);
        assert_eq!(
            mesh.stats.coastline_edges,
            mesh.stats.water_mask_coastline_edges
        );
    }

    #[test]
    fn shader_vertex_bytes_use_captured_stride_16_r16g16_sint_layout() {
        let mut mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        let bytes = mesh
            .to_shader_vertex_layout_bytes()
            .expect("captured vertex packing");
        assert_eq!(bytes.len(), mesh.vertices.len() * 16);
        assert_eq!(
            &bytes[0..12],
            mesh.vertices[0]
                .position
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            i16::from_le_bytes(bytes[12..14].try_into().unwrap()),
            mesh.vertices[0].uv[0] as i16
        );
        assert_eq!(
            i16::from_le_bytes(bytes[14..16].try_into().unwrap()),
            mesh.vertices[0].uv[1] as i16
        );

        mesh.vertices[0].uv[1] = i32::from(i16::MAX) + 1;
        let error = mesh
            .to_shader_vertex_layout_bytes()
            .expect_err("out-of-range captured uv lane must fail closed");
        assert!(error.to_string().contains("exceeds captured R16G16_SINT"));
    }

    #[test]
    fn chain_row_counts_follow_recovered_record_station_census() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        assert!(!mesh.chains.is_empty());
        for chain in &mesh.chains {
            let minimum_rows = chain.manhattan_length + u32::from(chain.closed);
            assert!(
                chain.rows >= minimum_rows,
                "chain for pair ({},{}) has {} rows below its {}-edge exact station minimum",
                chain.province_a,
                chain.province_b,
                chain.rows,
                chain.manhattan_length
            );
            assert_eq!(chain.vertex_count(), chain.rows * 2);
            assert_eq!(chain.index_count(), 6 * (chain.rows - 1));
        }
        let total_vertices: u32 = mesh.chains.iter().map(BorderChain::vertex_count).sum();
        let total_indices: u32 = mesh.chains.iter().map(BorderChain::index_count).sum();
        assert_eq!(mesh.vertices.len(), total_vertices as usize);
        assert_eq!(mesh.indices.len(), total_indices as usize);
    }

    #[test]
    fn ribbon_width_is_border_width_everywhere() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        for chain in &mesh.chains {
            for row in 0..chain.rows as usize {
                let left = &mesh.vertices[chain.first_vertex as usize + 2 * row];
                let right = &mesh.vertices[chain.first_vertex as usize + 2 * row + 1];
                assert_eq!(left.uv, [0, row as i32]);
                assert_eq!(right.uv, [1, row as i32]);
                let dx = f64::from(right.position[0] - left.position[0]);
                let dz = f64::from(right.position[2] - left.position[2]);
                let width = (dx * dx + dz * dz).sqrt();
                assert!(
                    (width - 1.5).abs() < 1e-5,
                    "row {row} of pair ({},{}) width {width}",
                    chain.province_a,
                    chain.province_b
                );
            }
        }
    }

    #[test]
    fn captured_brittany_centerline_law_reproduced_on_synthetic_staircase() {
        const CAPTURED: [[f64; 2]; 16] = [
            [2781.5, 1483.5],
            [2781.5, 1482.833_3],
            [2781.666_7, 1482.166_7],
            [2782.0, 1481.5],
            [2782.333_3, 1480.833_3],
            [2782.666_7, 1480.166_7],
            [2783.0, 1479.5],
            [2783.333_3, 1478.833_3],
            [2783.333_3, 1478.166_7],
            [2783.0, 1477.5],
            [2782.666_7, 1476.833_3],
            [2782.666_7, 1476.166_7],
            [2783.0, 1475.5],
            [2783.333_3, 1474.833_3],
            [2783.5, 1474.166_7],
            [2783.5, 1473.5],
        ];
        let corners_world: [[f64; 2]; 10] = [
            [2781.5, 1483.5],
            [2781.5, 1481.5],
            [2782.5, 1481.5],
            [2782.5, 1479.5],
            [2783.5, 1479.5],
            [2783.5, 1477.5],
            [2782.5, 1477.5],
            [2782.5, 1475.5],
            [2783.5, 1475.5],
            [2783.5, 1473.5],
        ];
        let mut points: Vec<[f64; 2]> = Vec::new();
        for window in corners_world.windows(2) {
            let (a, b) = (window[0], window[1]);
            let steps = ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()).round() as usize;
            for s in 0..steps {
                let f = s as f64 / steps as f64;
                points.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
            }
        }
        points.push(corners_world[corners_world.len() - 1]);

        let edge_count = points.len() - 1;
        assert_eq!(edge_count, 14, "Brittany chain Manhattan length");
        let mut stations: Vec<[f64; 2]> = Vec::with_capacity(edge_count + 2);
        stations.push(points[0]);
        for k in 0..edge_count {
            let a = points[k];
            let b = points[k + 1];
            stations.push([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
        }
        stations.push(points[edge_count]);
        let mut rows = stations.clone();
        for i in 1..stations.len() - 1 {
            rows[i] = [
                (stations[i - 1][0] + stations[i][0] + stations[i + 1][0]) / 3.0,
                (stations[i - 1][1] + stations[i][1] + stations[i + 1][1]) / 3.0,
            ];
        }
        assert_eq!(rows.len(), CAPTURED.len());
        for (row, captured) in rows.iter().zip(CAPTURED.iter()) {
            assert!(
                (row[0] - captured[0]).abs() < 1e-3 && (row[1] - captured[1]).abs() < 1e-3,
                "law row {row:?} != captured {captured:?}"
            );
        }
    }

    #[test]
    fn captured_closed_loop_law_reproduced() {
        const CAPTURED: [[f64; 2]; 15] = [
            [3105.0000, 1993.5000],
            [3105.8333, 1993.6666],
            [3106.3333, 1994.1666],
            [3106.5000, 1995.0000],
            [3106.5000, 1996.0000],
            [3106.3333, 1996.8334],
            [3106.0000, 1997.1666],
            [3105.5000, 1997.0000],
            [3104.8333, 1996.6666],
            [3104.1667, 1996.3334],
            [3103.6667, 1995.8334],
            [3103.6667, 1995.1666],
            [3104.0000, 1994.5000],
            [3104.5000, 1994.0000],
            [3105.0000, 1993.5000],
        ];
        let corners_world: [[f64; 2]; 14] = [
            [3104.5, 1993.5],
            [3104.5, 1994.5],
            [3103.5, 1994.5],
            [3103.5, 1995.5],
            [3103.5, 1996.5],
            [3104.5, 1996.5],
            [3105.5, 1996.5],
            [3105.5, 1997.5],
            [3106.5, 1997.5],
            [3106.5, 1996.5],
            [3106.5, 1995.5],
            [3106.5, 1994.5],
            [3106.5, 1993.5],
            [3105.5, 1993.5],
        ];
        let mut points: Vec<[f64; 2]> = corners_world.to_vec();
        points.push(corners_world[0]);
        let rows = closed_loop_rows(&points, &vec![false; points.len()]);
        assert_eq!(rows.len(), CAPTURED.len(), "loop rows = M + 1");
        for (row, captured) in rows.iter().zip(CAPTURED.iter()) {
            assert!(
                (row[0] - captured[0]).abs() < 1e-3 && (row[1] - captured[1]).abs() < 1e-3,
                "loop law row {row:?} != captured {captured:?}"
            );
        }
    }

    #[test]
    fn water_water_open_rows_use_decompiled_wide_smoothing() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [2.0, 1.0],
            [2.0, 2.0],
            [3.0, 2.0],
            [3.0, 3.0],
        ];
        let no_junctions = vec![false; points.len()];
        let land = open_chain_rows(&points, &no_junctions, BorderWaterClass::LandLand);
        let water = open_chain_rows(&points, &no_junctions, BorderWaterClass::WaterWater);
        assert_ne!(land, water);

        let mut expected = Vec::with_capacity(points.len() + 1);
        expected.push(points[0]);
        for pair in points.windows(2) {
            expected.push([
                (pair[0][0] + pair[1][0]) * 0.5,
                (pair[0][1] + pair[1][1]) * 0.5,
            ]);
        }
        expected.push(*points.last().unwrap());
        for _ in 0..2 {
            let src = expected.clone();
            for i in 1..src.len() - 1 {
                let radius = i.min(src.len() - 1 - i).min(6);
                let mut sum = [0.0f32, 0.0f32];
                let count = 2 * radius + 1;
                for p in &src[i - radius..=i + radius] {
                    sum[0] += p[0] as f32;
                    sum[1] += p[1] as f32;
                }
                expected[i] = [
                    f64::from(sum[0] / count as f32),
                    f64::from(sum[1] / count as f32),
                ];
            }
        }
        assert_eq!(water, expected);
    }

    #[test]
    fn connected_region_labelling_does_not_wrap_x() {
        let province_ids = [7, 8, 8, 7];
        let (labels, count) = connected_region_labels(&province_ids, 4, 1).expect("labels");
        assert_eq!(count, 3);
        assert_ne!(labels[0], labels[3]);
        assert_eq!(labels[1], labels[2]);
    }

    fn ledger_edge(
        current_province: u32,
        neighbor_province: u32,
        start: Corner,
        end: Corner,
    ) -> Option<ContourSegmentEdge> {
        Some(ContourSegmentEdge {
            current_province,
            neighbor_province,
            start,
            end,
        })
    }

    #[test]
    fn contour_record_ledger_revokes_seam_merge_at_start_junction() {
        let c0 = Corner { i: 0, j: 0 };
        let c1 = Corner { i: 1, j: 0 };
        let c2 = Corner { i: 0, j: 1 };
        let contour = [ledger_edge(7, 8, c0, c1), None, ledger_edge(7, 8, c2, c0)];

        let mut ordinary = ContourRecordLedger::default();
        ordinary
            .append_contour(&contour, &[1, 1, 1, 1], 2, 2)
            .expect("ordinary seam");
        assert_eq!(ordinary.records.len(), 1);
        assert_eq!(ordinary.records[0].corners, vec![c2, c0, c1]);
        assert!(!ordinary.records[0].closed);

        let mut junction = ContourRecordLedger::default();
        junction
            .append_contour(&contour, &[1, 2, 3, 4], 2, 2)
            .expect("junction seam");
        assert_eq!(junction.records.len(), 2);
        assert_eq!(junction.records[0].corners, vec![c0, c1]);
        assert_eq!(junction.records[1].corners, vec![c2, c0]);
    }

    #[test]
    fn contour_record_ledger_closes_a_single_eligible_record() {
        let c0 = Corner { i: 0, j: 0 };
        let c1 = Corner { i: 1, j: 0 };
        let c2 = Corner { i: 1, j: 1 };
        let c3 = Corner { i: 0, j: 1 };
        let contour = [
            ledger_edge(5, 6, c0, c1),
            ledger_edge(5, 6, c1, c2),
            ledger_edge(5, 6, c2, c3),
            ledger_edge(5, 6, c3, c0),
        ];
        let mut ledger = ContourRecordLedger::default();
        ledger
            .append_contour(&contour, &[1, 1, 1, 1], 2, 2)
            .expect("closed record");
        assert_eq!(ledger.records.len(), 1);
        assert!(ledger.records[0].closed);
        assert_eq!(
            ledger.records[0].corners.first(),
            ledger.records[0].corners.last()
        );
    }

    #[test]
    fn contour_record_ledger_emits_sorted_keys_newest_record_first() {
        let mut ledger = ContourRecordLedger::default();
        let labels = [1, 1, 1, 1];
        let old_start = Corner { i: 0, j: 0 };
        let newer_start = Corner { i: 0, j: 1 };
        for contour in [
            [None, ledger_edge(9, 2, old_start, Corner { i: 1, j: 0 })],
            [
                None,
                ledger_edge(1, 8, Corner { i: 1, j: 0 }, Corner { i: 1, j: 1 }),
            ],
            [None, ledger_edge(9, 2, newer_start, Corner { i: 1, j: 1 })],
        ] {
            ledger
                .append_contour(&contour, &labels, 2, 2)
                .expect("open record");
        }

        let chains = ledger.into_chains().expect("linked-list emission");
        assert_eq!(
            chains
                .iter()
                .map(|chain| (chain.province_a, chain.province_b))
                .collect::<Vec<_>>(),
            vec![(1, 8), (2, 9), (2, 9)]
        );
        assert_eq!(chains[1].chain.corners[0], newer_start);
        assert_eq!(chains[2].chain.corners[0], old_start);
    }

    #[test]
    fn multi_region_junction_contributes_two_station_rows() {
        let open_points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let open_rows = open_chain_rows(
            &open_points,
            &[false, true, false],
            BorderWaterClass::LandWater,
        );
        assert_eq!(open_rows.len(), 6, "M+2 ordinary rows + 2 junction rows");

        let closed_points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let closed_rows = closed_loop_rows(&closed_points, &[false, true, false, false, false]);
        assert_eq!(closed_rows.len(), 7, "M+1 ordinary rows + 2 junction rows");
    }

    #[test]
    fn suppressed_record_boundaries_do_not_invent_endpoint_stations() {
        let points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        let rows = open_chain_rows_with_endpoints(
            &points,
            &[false, false, false],
            BorderWaterClass::LandWater,
            [false; 2],
        );
        assert_eq!(rows, vec![[0.5, 0.0], [1.0, 0.5]]);
    }

    #[test]
    fn closed_record_duplicates_its_first_stored_station() {
        let points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let rows = engine_record_rows(
            &points,
            &[false, false, false, false, false],
            BorderWaterClass::LandWater,
            [false; 2],
            true,
        );
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.first(), Some(&[0.5, 0.0]));
        assert_eq!(rows.last(), Some(&[0.5, 0.0]));
    }

    #[test]
    fn engine_record_wrap_is_canonicalized_before_f32_smoothing() {
        let mut seam = [[5631.5, 10.0], [5631.5, 11.0], [5632.5, 11.0]];
        canonicalize_engine_record_x(&mut seam, [false; 2], 5632.0);
        assert_eq!(seam, [[-0.5, 10.0], [-0.5, 11.0], [0.5, 11.0]]);

        let mut ordinary = [[5631.0, 10.0], [5631.0, 11.0]];
        canonicalize_engine_record_x(&mut ordinary, [false; 2], 5632.0);
        assert_eq!(ordinary, [[5631.0, 10.0], [5631.0, 11.0]]);
    }

    #[test]
    fn closed_loop_junction_can_own_the_seam_anchor() {
        let points = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let rows = closed_loop_rows(&points, &[true, false, false, false, true]);
        assert_eq!(rows.len(), 6, "M midpoints + duplicated junction seam");
        assert_eq!(rows.first(), Some(&[0.0, 0.0]));
        assert_eq!(rows.last(), Some(&[0.0, 0.0]));
    }

    #[derive(Clone)]
    struct PoolStrip {
        index: usize,
        rows: usize,
        pts: Vec<[f64; 2]>,
        sides: Vec<[[f32; 3]; 2]>,
        bytes: Vec<u8>,
    }

    #[derive(Clone)]
    struct PoolCandidate {
        index: usize,
        rows: usize,
        pts: Vec<[f64; 2]>,
        province_a: u32,
        province_b: u32,
        rank: BorderRank,
        water_class: BorderWaterClass,
        material: BorderMaterial,
        closed: bool,
        manhattan_length: u32,
        sides: Vec<[[f32; 3]; 2]>,
    }

    struct PoolProposal {
        err: f64,
        strip: usize,
        chain: usize,
        rev: bool,
        x_shift: f64,
    }

    struct PoolMatch {
        err: f64,
        strip: usize,
        chain: usize,
        rev: bool,
        x_shift: f64,
    }

    fn read_pool_f32(data: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
    }

    fn read_pool_i16(data: &[u8], offset: usize) -> i16 {
        i16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
    }

    fn water_rows_with_f32_window_order(
        chain: &ChainCorners,
        width: usize,
        height: usize,
        reverse_window: bool,
    ) -> Vec<[f32; 2]> {
        let width = width as f64;
        let height = height as f64;
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(chain.corners.len());
        let mut x_offset = 0.0f64;
        for (index, corner) in chain.corners.iter().enumerate() {
            let base_x = corner.i as f64 + 0.5;
            let z = (height - 1.5) - corner.j as f64;
            let mut x = base_x + x_offset;
            if index > 0 {
                let prev_x = points[index - 1][0];
                if x - prev_x > width * 0.5 {
                    x_offset -= width;
                    x -= width;
                } else if prev_x - x > width * 0.5 {
                    x_offset += width;
                    x += width;
                }
            }
            points.push([x, z]);
        }

        let edge_count = points.len() - 1;
        let interior_junctions = chain.junctions[1..edge_count]
            .iter()
            .filter(|&&value| value)
            .count();
        let mut rows = Vec::with_capacity(edge_count + 2 + 2 * interior_junctions);
        rows.push([points[0][0] as f32, points[0][1] as f32]);
        for k in 0..edge_count {
            let a = points[k];
            let b = points[k + 1];
            rows.push([((a[0] + b[0]) * 0.5) as f32, ((a[1] + b[1]) * 0.5) as f32]);
            if k + 1 < edge_count && chain.junctions[k + 1] {
                let junction = [points[k + 1][0] as f32, points[k + 1][1] as f32];
                rows.push(junction);
                rows.push(junction);
            }
        }
        rows.push([points[edge_count][0] as f32, points[edge_count][1] as f32]);

        for _ in 0..2 {
            let src = rows.clone();
            for i in 1..src.len() - 1 {
                let radius = i.min(src.len() - 1 - i).min(6);
                let window = &src[i - radius..=i + radius];
                let mut sum = [0.0f32, 0.0f32];
                if reverse_window {
                    for point in window.iter().rev() {
                        sum[0] += point[0];
                        sum[1] += point[1];
                    }
                } else {
                    for point in window {
                        sum[0] += point[0];
                        sum[1] += point[1];
                    }
                }
                let count = window.len() as f32;
                rows[i] = [sum[0] / count, sum[1] / count];
            }
        }
        rows
    }

    fn emitted_water_centerline(rows: &[[f32; 2]], half_width: f32) -> Vec<[f64; 2]> {
        let mut centerline = Vec::with_capacity(rows.len());
        for k in 0..rows.len() {
            let prev = rows[k.saturating_sub(1)];
            let next = rows[(k + 1).min(rows.len() - 1)];
            let dz = next[1] - prev[1];
            let neg_dx = -(next[0] - prev[0]);
            let len = (neg_dx * neg_dx + dz * dz).sqrt();
            let perp_x = (half_width * dz) / len;
            let perp_z = (half_width * neg_dx) / len;
            let side0 = [rows[k][0] + perp_x, rows[k][1] + perp_z];
            let side1 = [rows[k][0] - perp_x, rows[k][1] - perp_z];
            centerline.push([
                f64::from((side0[0] + side1[0]) * 0.5),
                f64::from((side0[1] + side1[1]) * 0.5),
            ]);
        }
        centerline
    }

    fn emitted_land_centerline(rows: &[[f64; 2]], half_width: f64) -> Vec<[f64; 2]> {
        let mut centerline = Vec::with_capacity(rows.len());
        for k in 0..rows.len() {
            let prev = rows[k.saturating_sub(1)];
            let next = rows[(k + 1).min(rows.len() - 1)];
            let dx = next[0] - prev[0];
            let dz = next[1] - prev[1];
            let len = (dx * dx + dz * dz).sqrt();
            let perp_x = dz / len;
            let perp_z = -dx / len;
            let side0 = [
                (rows[k][0] + perp_x * half_width) as f32,
                (rows[k][1] + perp_z * half_width) as f32,
            ];
            let side1 = [
                (rows[k][0] - perp_x * half_width) as f32,
                (rows[k][1] - perp_z * half_width) as f32,
            ];
            centerline.push([
                f64::from((side0[0] + side1[0]) * 0.5),
                f64::from((side0[1] + side1[1]) * 0.5),
            ]);
        }
        centerline
    }

    fn emitted_land_sides(
        rows: &[[f64; 2]],
        half_width: f64,
        heightmap: &IndexImage,
    ) -> Vec<[[f32; 3]; 2]> {
        let mut sides = Vec::with_capacity(rows.len());
        for k in 0..rows.len() {
            let prev = rows[k.saturating_sub(1)];
            let next = rows[(k + 1).min(rows.len() - 1)];
            let dx = next[0] - prev[0];
            let dz = next[1] - prev[1];
            let len = (dx * dx + dz * dz).sqrt();
            let perp_x = dz / len;
            let perp_z = -dx / len;
            let mut row = [[0.0f32; 3]; 2];
            for (side, sign) in [(0usize, 1.0f64), (1usize, -1.0f64)] {
                let x = (rows[k][0] + perp_x * half_width * sign) as f32;
                let z = (rows[k][1] + perp_z * half_width * sign) as f32;
                row[side] = [x, sample_border_height(heightmap, x, z), z];
            }
            sides.push(row);
        }
        sides
    }

    fn engine_vertex_bytes(sides: &[[[f32; 3]; 2]], reverse: bool, x_shift: f64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(sides.len() * 32);
        for output_row in 0..sides.len() {
            let input_row = if reverse {
                sides.len() - 1 - output_row
            } else {
                output_row
            };
            for output_side in 0..2usize {
                let input_side = if reverse {
                    1 - output_side
                } else {
                    output_side
                };
                let mut position = sides[input_row][input_side];
                position[0] = (f64::from(position[0]) + x_shift) as f32;
                for value in position {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes.extend_from_slice(&(output_side as i16).to_le_bytes());
                bytes.extend_from_slice(&(output_row as i16).to_le_bytes());
            }
        }
        bytes
    }

    fn byte_diff_count(a: &[u8], b: &[u8]) -> usize {
        a.iter()
            .zip(b)
            .filter(|(left, right)| left != right)
            .count()
            + a.len().abs_diff(b.len())
    }

    fn centerline_bytes(points: &[[f64; 2]], reverse: bool, x_shift: f64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(points.len() * 8);
        for output_row in 0..points.len() {
            let input_row = if reverse {
                points.len() - 1 - output_row
            } else {
                output_row
            };
            let x = (points[input_row][0] + x_shift) as f32;
            let z = points[input_row][1] as f32;
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&z.to_le_bytes());
        }
        bytes
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum SideAttribution {
        Unique {
            provinces: (u32, u32),
            regions: (u32, u32),
            count: usize,
        },
        Empty,
        Tie {
            count: usize,
            candidates: Vec<((u32, u32), (u32, u32))>,
        },
    }

    fn majority_side_attribution(
        sides: &[[[f32; 3]; 2]],
        province_ids: &[u32],
        region_labels: &[u32],
        width: usize,
        height: usize,
    ) -> SideAttribution {
        let mut counts: BTreeMap<((u32, u32), (u32, u32)), usize> = BTreeMap::new();
        for row in sides {
            let mut indices = [0usize; 2];
            let mut valid = true;
            for side in 0..2 {
                let x = (row[side][0].round() as i64).rem_euclid(width as i64) as usize;
                let y = ((height as f32 - 1.0) - row[side][2]).round() as i64;
                if y < 0 || y >= height as i64 {
                    valid = false;
                    break;
                }
                indices[side] = y as usize * width + x;
            }
            if !valid || province_ids[indices[0]] == province_ids[indices[1]] {
                continue;
            }
            *counts
                .entry((
                    (province_ids[indices[0]], province_ids[indices[1]]),
                    (region_labels[indices[0]], region_labels[indices[1]]),
                ))
                .or_insert(0) += 1;
        }
        let Some(max_count) = counts.values().copied().max() else {
            return SideAttribution::Empty;
        };
        let candidates: Vec<_> = counts
            .into_iter()
            .filter_map(|(identity, count)| (count == max_count).then_some(identity))
            .collect();
        if candidates.len() == 1 {
            let (provinces, regions) = candidates[0];
            SideAttribution::Unique {
                provinces,
                regions,
                count: max_count,
            }
        } else {
            SideAttribution::Tie {
                count: max_count,
                candidates,
            }
        }
    }

    fn decode_border_pool_strips(vb_path: &Path) -> Vec<PoolStrip> {
        let data = fs::read(vb_path).expect("read captured border VB pool");
        assert_eq!(data.len() % 16, 0, "captured VB must be stride-16 aligned");
        let vertex_count = data.len() / 16;
        let mut starts = Vec::new();
        for i in (0..vertex_count).step_by(2) {
            let off0 = i * 16;
            let off1 = (i + 1) * 16;
            let uv0 = [
                read_pool_i16(&data, off0 + 12),
                read_pool_i16(&data, off0 + 14),
            ];
            let uv1 = [
                read_pool_i16(&data, off1 + 12),
                read_pool_i16(&data, off1 + 14),
            ];
            assert_eq!(uv0[0], 0, "captured row {i} uv.x side 0");
            assert_eq!(uv1[0], 1, "captured row {i} uv.x side 1");
            assert_eq!(uv0[1], uv1[1], "captured row {i} uv.y pair");
            if uv0[1] == 0 {
                starts.push(i);
            }
        }

        let mut strips = Vec::with_capacity(starts.len());
        for (strip_index, &start) in starts.iter().enumerate() {
            let end = starts.get(strip_index + 1).copied().unwrap_or(vertex_count);
            let rows = (end - start) / 2;
            let mut pts = Vec::with_capacity(rows);
            let mut sides = Vec::with_capacity(rows);
            for row in 0..rows {
                let off0 = (start + 2 * row) * 16;
                let off1 = (start + 2 * row + 1) * 16;
                let side0 = [
                    read_pool_f32(&data, off0),
                    read_pool_f32(&data, off0 + 4),
                    read_pool_f32(&data, off0 + 8),
                ];
                let side1 = [
                    read_pool_f32(&data, off1),
                    read_pool_f32(&data, off1 + 4),
                    read_pool_f32(&data, off1 + 8),
                ];
                let x = (side0[0] + side1[0]) as f64 * 0.5;
                let z = (side0[2] + side1[2]) as f64 * 0.5;
                pts.push([x, z]);
                sides.push([side0, side1]);
            }
            strips.push(PoolStrip {
                index: strip_index,
                rows,
                pts,
                sides,
                bytes: data[start * 16..end * 16].to_vec(),
            });
        }
        strips
    }

    fn border_pool_dir_for_test() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("BATTLE_PLAN_BORDER_POOL_DIR").or_else(|_| std::env::var("HOI4_BORDER_POOL_DIR")) {
            return Some(PathBuf::from(path));
        }
        let preserved =
            PathBuf::from(r"C:\Users\Example\hoi4_border_pool_preserved\session_BORDERZOOM_3");
        preserved.exists().then_some(preserved)
    }

    fn pts_bbox_center(pts: &[[f64; 2]]) -> [f64; 2] {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for p in pts {
            min_x = min_x.min(p[0]);
            min_z = min_z.min(p[1]);
            max_x = max_x.max(p[0]);
            max_z = max_z.max(p[1]);
        }
        [(min_x + max_x) * 0.5, (min_z + max_z) * 0.5]
    }

    fn centerline_error(a: &[[f64; 2]], b: &[[f64; 2]], x_shift: f64) -> f64 {
        a.iter()
            .zip(b.iter())
            .map(|(pa, pb)| {
                let dx = pa[0] - (pb[0] + x_shift);
                let dz = pa[1] - pb[1];
                (dx * dx + dz * dz).sqrt()
            })
            .fold(0.0f64, f64::max)
    }

    fn best_centerline_error(
        cap: &[[f64; 2]],
        syn: &[[f64; 2]],
        map_width: f64,
    ) -> (f64, bool, f64) {
        let mean_cap = cap.iter().map(|p| p[0]).sum::<f64>() / cap.len() as f64;
        let mut best = (f64::INFINITY, false, 0.0);
        for rev in [false, true] {
            let pts: Vec<[f64; 2]> = if rev {
                syn.iter().rev().copied().collect()
            } else {
                syn.to_vec()
            };
            let mean_syn = pts.iter().map(|p| p[0]).sum::<f64>() / pts.len() as f64;
            let base_shift = ((mean_cap - mean_syn) / map_width).round() * map_width;
            for x_shift in [base_shift - map_width, base_shift, base_shift + map_width] {
                let err = centerline_error(cap, &pts, x_shift);
                if err < best.0 {
                    best = (err, rev, x_shift);
                }
            }
        }
        best
    }

    fn pool_candidates(mesh: &BorderWorldMesh) -> Vec<PoolCandidate> {
        mesh.chains
            .iter()
            .enumerate()
            .map(|(index, chain)| PoolCandidate {
                index,
                rows: chain.rows as usize,
                pts: mesh
                    .chain_centerline(chain)
                    .into_iter()
                    .map(|p| [p[0] as f64, p[2] as f64])
                    .collect(),
                water_class: chain.water_class,
                material: chain.material,
                closed: chain.closed,
                province_a: chain.province_a,
                province_b: chain.province_b,
                rank: chain.rank,
                manhattan_length: chain.manhattan_length,
                sides: (0..chain.rows as usize)
                    .map(|row| {
                        let base = chain.first_vertex as usize + 2 * row;
                        [
                            mesh.vertices[base].position,
                            mesh.vertices[base + 1].position,
                        ]
                    })
                    .collect(),
            })
            .collect()
    }

    fn assign_pool_matches(
        pool: &[PoolStrip],
        synth: &[PoolCandidate],
        map_width: f64,
    ) -> Vec<PoolMatch> {
        let cell = 8.0f64;
        let radius_cells = 3i32;
        let mut index: HashMap<(usize, i32, i32), Vec<usize>> = HashMap::new();
        for candidate in synth {
            let c = pts_bbox_center(&candidate.pts);
            index
                .entry((
                    candidate.rows,
                    (c[0] / cell).floor() as i32,
                    (c[1] / cell).floor() as i32,
                ))
                .or_default()
                .push(candidate.index);
        }

        let mut proposals = Vec::new();
        for strip in pool {
            let c = pts_bbox_center(&strip.pts);
            let gx = (c[0] / cell).floor() as i32;
            let gz = (c[1] / cell).floor() as i32;
            let mut scored = Vec::new();
            for dz in -radius_cells..=radius_cells {
                for dx in -radius_cells..=radius_cells {
                    if let Some(candidates) = index.get(&(strip.rows, gx + dx, gz + dz)) {
                        for &chain_index in candidates {
                            let chain = &synth[chain_index];
                            let (err, rev, x_shift) =
                                best_centerline_error(&strip.pts, &chain.pts, map_width);
                            scored.push(PoolProposal {
                                err,
                                strip: strip.index,
                                chain: chain_index,
                                rev,
                                x_shift,
                            });
                        }
                    }
                }
            }
            scored.sort_by(|a, b| a.err.partial_cmp(&b.err).unwrap());
            proposals.extend(scored);
        }
        proposals.sort_by(|a, b| a.err.partial_cmp(&b.err).unwrap());

        let mut assigned_strips = vec![false; pool.len()];
        let mut assigned_chains = vec![false; synth.len()];
        let mut matches = Vec::new();
        for proposal in proposals {
            if assigned_strips[proposal.strip] || assigned_chains[proposal.chain] {
                continue;
            }
            assigned_strips[proposal.strip] = true;
            assigned_chains[proposal.chain] = true;
            matches.push(PoolMatch {
                err: proposal.err,
                strip: proposal.strip,
                chain: proposal.chain,
                rev: proposal.rev,
                x_shift: proposal.x_shift,
            });
        }
        matches
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR and the preserved session_BORDERZOOM_3 pool"]
    fn gate_vanilla_preserved_pool_bord9_water_smoothing_counts() {
        let game_dir =
            std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")).expect("ignored evidence gate requires BATTLE_PLAN_GAME_DIR");
        let pool_dir = border_pool_dir_for_test().expect(
            "ignored evidence gate requires BATTLE_PLAN_BORDER_POOL_DIR or the preserved default pool",
        );
        let vb_path = pool_dir.join("borders_frame_draw000_ia_vertex_buffer_slot0.bin");
        assert!(
            vb_path.is_file(),
            "ignored evidence gate requires preserved pool blob {}",
            vb_path.display()
        );

        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let inputs = border_synthesis_inputs_from_resolver(&resolver).expect("border inputs");
        let mesh = synthesize_border_world_mesh_from_inputs_with_trace_law(
            &inputs,
            BorderTraceLaw::RegionContours,
        )
        .expect("exact region-contour mesh");
        let pool = decode_border_pool_strips(&vb_path);
        let synth = pool_candidates(&mesh);
        assert_eq!(pool.len(), 40_959, "captured engine record census");
        assert_eq!(
            synth.len(),
            pool.len(),
            "global contour ledger must preserve every engine record identity"
        );

        let mut exact = 0usize;
        let mut reversed = 0usize;
        let mut worst = 0.0f64;
        let mut residuals = Vec::new();
        for (record_index, (captured, candidate)) in pool.iter().zip(&synth).enumerate() {
            assert_eq!(
                captured.rows, candidate.rows,
                "ordered record {record_index} row census"
            );
            let (error, reverse, x_shift) =
                best_centerline_error(&captured.pts, &candidate.pts, mesh.map_width as f64);
            worst = worst.max(error);
            if error < 1e-3 {
                exact += 1;
                reversed += usize::from(reverse);
            } else {
                let aligned = if reverse {
                    candidate.pts.iter().rev().copied().collect::<Vec<_>>()
                } else {
                    candidate.pts.clone()
                };
                let (worst_row, worst_delta, _) = captured
                    .pts
                    .iter()
                    .zip(&aligned)
                    .enumerate()
                    .map(|(row, (cap, syn))| {
                        let delta = [cap[0] - (syn[0] + x_shift), cap[1] - syn[1]];
                        let row_error = delta[0].hypot(delta[1]);
                        (row, delta, row_error)
                    })
                    .max_by(|left, right| left.2.total_cmp(&right.2))
                    .expect("nonempty border record");
                residuals.push((
                    record_index,
                    (candidate.province_a, candidate.province_b),
                    candidate.water_class,
                    captured.rows,
                    candidate.rows,
                    error,
                    worst_row,
                    worst_delta,
                    captured.pts.first().copied(),
                    captured.pts.last().copied(),
                    aligned.first().map(|point| [point[0] + x_shift, point[1]]),
                    aligned.last().map(|point| [point[0] + x_shift, point[1]]),
                ));
            }
        }
        eprintln!(
            "XMAP06 exact ordered-pool referee: exact={}/{} reversed={} worst_centerline_error={:.9} residuals={:?}",
            exact,
            pool.len(),
            reversed,
            worst,
            residuals,
        );
        assert!(
            residuals.is_empty(),
            "ordered exact-a729 border records diverged from the retained pool"
        );
        assert_eq!(exact, 40_959);
    }

    #[test]
    #[ignore = "historical PairEdges/BORD12/BORD16 comparison; the ordered-pool gate is authoritative"]
    fn historical_pair_edges_bord12_bord16_pool_diagnostic() {
        let game_dir =
            std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")).expect("ignored evidence gate requires BATTLE_PLAN_GAME_DIR");
        let pool_dir = border_pool_dir_for_test().expect(
            "ignored evidence gate requires BATTLE_PLAN_BORDER_POOL_DIR or the preserved default pool",
        );
        let vb_path = pool_dir.join("borders_frame_draw000_ia_vertex_buffer_slot0.bin");
        assert!(
            vb_path.is_file(),
            "ignored evidence gate requires preserved pool blob {}",
            vb_path.display()
        );

        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let inputs = border_synthesis_inputs_from_resolver(&resolver).expect("border inputs");
        let mesh = synthesize_border_world_mesh_from_inputs_with_trace_law(
            &inputs,
            BorderTraceLaw::RegionContours,
        )
        .expect("region-contour mesh");
        let bord12_mesh = synthesize_border_world_mesh_from_inputs_with_laws(
            &inputs,
            BorderTraceLaw::RegionContours,
            ClosedLoopSeamLaw::MidpointOnly,
        )
        .expect("BORD12 midpoint-anchor region-contour mesh");
        let baseline_mesh = synthesize_border_world_mesh_from_inputs_with_trace_law(
            &inputs,
            BorderTraceLaw::PairEdgesBaseline,
        )
        .expect("pair-edge baseline mesh");
        let pool = decode_border_pool_strips(&vb_path);
        assert_eq!(pool.len(), 40_959);

        let map_width = mesh.map_width as f64;
        let synth = pool_candidates(&mesh);
        let matches = assign_pool_matches(&pool, &synth, map_width);
        let bord12_synth = pool_candidates(&bord12_mesh);
        let bord12_matches = assign_pool_matches(&pool, &bord12_synth, map_width);
        let baseline_synth = pool_candidates(&baseline_mesh);
        let baseline_matches = assign_pool_matches(&pool, &baseline_synth, map_width);

        let mut land_open_exact = 0usize;
        let mut land_open_total = 0usize;
        let mut water_open_exact = 0usize;
        let mut water_open_total = 0usize;
        let mut sea_region_exact = 0usize;
        let mut water_open_residuals = Vec::new();
        let mut exact_by_pool_strip = vec![false; pool.len()];
        let mut exact_closed_by_pool_strip = vec![false; pool.len()];
        let mut exact_closed_loops = 0usize;
        let mut exact_closed_coastline_loops = 0usize;
        for m in &matches {
            let chain = &synth[m.chain];
            if m.err < 1e-3 {
                exact_by_pool_strip[m.strip] = true;
                if chain.closed {
                    exact_closed_by_pool_strip[m.strip] = true;
                    exact_closed_loops += 1;
                    if chain.water_class == BorderWaterClass::LandWater {
                        exact_closed_coastline_loops += 1;
                    }
                }
            }
            if !chain.closed && chain.water_class == BorderWaterClass::LandLand {
                land_open_total += 1;
                if m.err < 1e-3 {
                    land_open_exact += 1;
                }
            }
            if !chain.closed && chain.water_class == BorderWaterClass::WaterWater {
                water_open_total += 1;
                if m.err < 1e-3 {
                    water_open_exact += 1;
                } else {
                    water_open_residuals.push(m);
                }
            }
            if chain.material == BorderMaterial::SeaRegion && m.err < 1e-3 {
                sea_region_exact += 1;
            }
        }

        let mut bord12_exact_by_pool_strip = vec![false; pool.len()];
        let mut bord12_exact_closed_by_pool_strip = vec![false; pool.len()];
        for m in &bord12_matches {
            if m.err < 1e-3 {
                bord12_exact_by_pool_strip[m.strip] = true;
                if bord12_synth[m.chain].closed {
                    bord12_exact_closed_by_pool_strip[m.strip] = true;
                }
            }
        }
        let bord12_exact_count = bord12_exact_by_pool_strip
            .iter()
            .filter(|&&exact| exact)
            .count();
        let bord12_exact_closed_loops = bord12_exact_closed_by_pool_strip
            .iter()
            .filter(|&&exact| exact)
            .count();
        let bord16_closed_identity_gains: Vec<usize> = exact_closed_by_pool_strip
            .iter()
            .zip(&bord12_exact_closed_by_pool_strip)
            .enumerate()
            .filter_map(|(strip, (after, before))| (*after && !*before).then_some(strip))
            .collect();
        let bord16_closed_identity_losses: Vec<usize> = exact_closed_by_pool_strip
            .iter()
            .zip(&bord12_exact_closed_by_pool_strip)
            .enumerate()
            .filter_map(|(strip, (after, before))| (!*after && *before).then_some(strip))
            .collect();

        let mut baseline_exact_by_pool_strip = vec![false; pool.len()];
        let mut baseline_exact_closed_by_pool_strip = vec![false; pool.len()];
        let mut baseline_exact_closed_loops = 0usize;
        for m in &baseline_matches {
            if m.err < 1e-3 {
                baseline_exact_by_pool_strip[m.strip] = true;
                if baseline_synth[m.chain].closed {
                    baseline_exact_closed_by_pool_strip[m.strip] = true;
                    baseline_exact_closed_loops += 1;
                }
            }
        }
        let gained_exact_closed_loops = exact_closed_by_pool_strip
            .iter()
            .zip(&baseline_exact_closed_by_pool_strip)
            .filter(|(after, before)| **after && !**before)
            .count();
        let lost_exact_closed_loops = exact_closed_by_pool_strip
            .iter()
            .zip(&baseline_exact_closed_by_pool_strip)
            .filter(|(after, before)| !**after && **before)
            .count();
        let newly_exact_closed_from_unmatched = exact_closed_by_pool_strip
            .iter()
            .zip(&baseline_exact_by_pool_strip)
            .filter(|(after_closed, before_exact)| **after_closed && !**before_exact)
            .count();
        let reclassified_exact_open_as_closed = exact_closed_by_pool_strip
            .iter()
            .zip(&baseline_exact_by_pool_strip)
            .zip(&baseline_exact_closed_by_pool_strip)
            .filter(|((after_closed, before_exact), before_closed)| {
                **after_closed && **before_exact && !**before_closed
            })
            .count();
        let baseline_exact_count = baseline_exact_by_pool_strip
            .iter()
            .filter(|&&exact| exact)
            .count();
        let exact_count = exact_by_pool_strip.iter().filter(|&&exact| exact).count();
        let gained_exact_strips = exact_by_pool_strip
            .iter()
            .zip(&baseline_exact_by_pool_strip)
            .filter(|(after, before)| **after && !**before)
            .count();
        let lost_exact_strips = exact_by_pool_strip
            .iter()
            .zip(&baseline_exact_by_pool_strip)
            .filter(|(after, before)| !**after && **before)
            .count();
        let mut pair_edge_lost_exact_identities: Vec<_> = baseline_matches
            .iter()
            .filter(|m| m.err < 1e-3 && !exact_by_pool_strip[m.strip])
            .map(|m| {
                let chain = &baseline_synth[m.chain];
                (
                    m.strip,
                    chain.province_a,
                    chain.province_b,
                    chain.rank,
                    chain.water_class,
                    chain.closed,
                    chain.rows,
                )
            })
            .collect();
        pair_edge_lost_exact_identities.sort_by_key(|identity| identity.0);
        let mut pair_edge_loss_classes: BTreeMap<(BorderRank, BorderWaterClass, bool), usize> =
            BTreeMap::new();
        for identity in &pair_edge_lost_exact_identities {
            *pair_edge_loss_classes
                .entry((identity.3, identity.4, identity.5))
                .or_insert(0) += 1;
        }

        let pool_strip_is_closed = |strip: &PoolStrip| {
            let first = strip.pts.first().expect("pool strip has a first row");
            let last = strip.pts.last().expect("pool strip has a last row");
            (first[0] - last[0]).abs() < 1e-3 && (first[1] - last[1]).abs() < 1e-3
        };
        let pool_closed_total = pool
            .iter()
            .filter(|strip| pool_strip_is_closed(strip))
            .count();
        let pool_closed_unmatched_exact = pool
            .iter()
            .filter(|strip| pool_strip_is_closed(strip) && !exact_by_pool_strip[strip.index])
            .count();
        let bord12_pool_closed_unmatched_exact = pool
            .iter()
            .filter(|strip| pool_strip_is_closed(strip) && !bord12_exact_by_pool_strip[strip.index])
            .count();
        for m in matches
            .iter()
            .filter(|m| m.err < 1e-3 && synth[m.chain].closed)
        {
            assert!(
                pool_strip_is_closed(&pool[m.strip]),
                "synthetic exact-closed chain {} matched engine-open strip {}",
                m.chain,
                m.strip,
            );
        }
        let seam_discriminator = matches
            .iter()
            .find(|m| m.strip == 26_491)
            .expect("engine strip 26491 must have a synthetic match");
        let seam_candidate = &synth[seam_discriminator.chain];
        let seam_strip = &pool[26_491];
        assert_eq!(
            (seam_candidate.province_a, seam_candidate.province_b),
            (8_653, 13_075),
            "strip 26491 province-pair identity",
        );
        assert_eq!(seam_strip.rows, 10, "strip 26491 engine row count");
        assert_eq!(seam_candidate.rows, 10, "strip 26491 synthetic row count");
        assert_eq!(
            seam_strip.pts.first(),
            Some(&[1_083.5, 2_020.5]),
            "strip 26491 engine seam start",
        );
        assert_eq!(
            seam_strip.pts.last(),
            Some(&[1_083.5, 2_020.5]),
            "strip 26491 engine seam end",
        );
        let synthetic_seam_start = if seam_discriminator.rev {
            seam_candidate.pts.last().copied()
        } else {
            seam_candidate.pts.first().copied()
        }
        .expect("strip 26491 synthetic seam start");
        let synthetic_seam_end = if seam_discriminator.rev {
            seam_candidate.pts.first().copied()
        } else {
            seam_candidate.pts.last().copied()
        }
        .expect("strip 26491 synthetic seam end");
        assert_eq!(
            [
                synthetic_seam_start[0] + seam_discriminator.x_shift,
                synthetic_seam_start[1],
            ],
            [1_083.5, 2_020.5],
            "strip 26491 synthetic seam start",
        );
        assert_eq!(
            [
                synthetic_seam_end[0] + seam_discriminator.x_shift,
                synthetic_seam_end[1],
            ],
            [1_083.5, 2_020.5],
            "strip 26491 synthetic seam end",
        );
        eprintln!(
            "BORD16 pair-edge historical comparison: matches={} (pair_edge={}) exact={} (pair_edge={} gained={} lost={}) land_open={}/{} water_open={}/{} sea_region_exact={} exact_closed_loops={} (pair_edge={} gained={} lost={} newly_exact_from_unmatched={} reclassified_open={}) exact_closed_coastline_loops={} pool_closed_unmatched_exact={}/{} synth_closed={}",
            matches.len(),
            baseline_matches.len(),
            exact_count,
            baseline_exact_count,
            gained_exact_strips,
            lost_exact_strips,
            land_open_exact,
            land_open_total,
            water_open_exact,
            water_open_total,
            sea_region_exact,
            exact_closed_loops,
            baseline_exact_closed_loops,
            gained_exact_closed_loops,
            lost_exact_closed_loops,
            newly_exact_closed_from_unmatched,
            reclassified_exact_open_as_closed,
            exact_closed_coastline_loops,
            pool_closed_unmatched_exact,
            pool_closed_total,
            mesh.stats.closed_loop_chains
        );
        eprintln!(
            "BORD16 same-matcher closure comparison: bord12_matches={} bord12_exact={} bord12_exact_closed={} bord12_pool_closed_unmatched={}/{} bord16_matches={} bord16_exact={} bord16_exact_closed={} bord16_pool_closed_unmatched={}/{} identity_gains={:?} identity_losses={:?}",
            bord12_matches.len(),
            bord12_exact_count,
            bord12_exact_closed_loops,
            bord12_pool_closed_unmatched_exact,
            pool_closed_total,
            matches.len(),
            exact_count,
            exact_closed_loops,
            pool_closed_unmatched_exact,
            pool_closed_total,
            bord16_closed_identity_gains,
            bord16_closed_identity_losses,
        );
        eprintln!(
            "BORD16 pair-edge exact identity losses: count={} classes={:?} identities={:?}",
            pair_edge_lost_exact_identities.len(),
            pair_edge_loss_classes,
            pair_edge_lost_exact_identities,
        );
        if std::env::var_os("BORD9_DUMP_WATER_RESIDUALS").is_some() {
            let mut by_length: BTreeMap<u32, usize> = BTreeMap::new();
            for m in &water_open_residuals {
                let chain = &synth[m.chain];
                *by_length.entry(chain.manhattan_length).or_insert(0) += 1;
            }
            eprintln!(
                "BORD9 water residual diagnostics: count={} threshold=1e-3 length_hist={:?}",
                water_open_residuals.len(),
                by_length
            );
            for (row, m) in water_open_residuals.iter().enumerate() {
                let chain = &synth[m.chain];
                let strip = &pool[m.strip];
                let cap_first = strip.pts.first().copied().unwrap_or([f64::NAN, f64::NAN]);
                let cap_last = strip.pts.last().copied().unwrap_or([f64::NAN, f64::NAN]);
                let syn_aligned: Vec<[f64; 2]> = if m.rev {
                    chain.pts.iter().rev().copied().collect()
                } else {
                    chain.pts.clone()
                };
                let syn_first = syn_aligned.first().copied().unwrap_or([f64::NAN, f64::NAN]);
                let syn_last = syn_aligned.last().copied().unwrap_or([f64::NAN, f64::NAN]);
                let syn_first = [syn_first[0] + m.x_shift, syn_first[1]];
                let syn_last = [syn_last[0] + m.x_shift, syn_last[1]];
                let mut worst_row = 0usize;
                let mut worst_dx = 0.0f64;
                let mut worst_dz = 0.0f64;
                let mut worst_err = 0.0f64;
                for (i, (cap, syn)) in strip.pts.iter().zip(syn_aligned.iter()).enumerate() {
                    let dx = cap[0] - (syn[0] + m.x_shift);
                    let dz = cap[1] - syn[1];
                    let err = (dx * dx + dz * dz).sqrt();
                    if err > worst_err {
                        worst_row = i;
                        worst_dx = dx;
                        worst_dz = dz;
                        worst_err = err;
                    }
                }
                eprintln!(
                    "BORD9 water residual {:02}: strip={} chain={} pair={}|{} material={} rank={} water_class={} closed={} manhattan={} rows={} err={:.6} worst_row={} worst_delta=({:.6},{:.6}) rev={} x_shift={:.1} cap=({:.3},{:.3})->({:.3},{:.3}) syn=({:.3},{:.3})->({:.3},{:.3})",
                    row + 1,
                    m.strip,
                    chain.index,
                    chain.province_a,
                    chain.province_b,
                    chain.material.as_str(),
                    chain.rank.as_str(),
                    chain.water_class.as_str(),
                    chain.closed,
                    chain.manhattan_length,
                    chain.rows,
                    m.err,
                    worst_row,
                    worst_dx,
                    worst_dz,
                    m.rev,
                    m.x_shift,
                    cap_first[0],
                    cap_first[1],
                    cap_last[0],
                    cap_last[1],
                    syn_first[0],
                    syn_first[1],
                    syn_last[0],
                    syn_last[1]
                );
            }
        }
        let traced = chains_from_region_contours(&inputs.province_ids, inputs.width, inputs.height)
            .expect("byte diagnostic region contours");
        let (region_labels, _) =
            connected_region_labels(&inputs.province_ids, inputs.width, inputs.height)
                .expect("engine-oracle connected-region labels");
        let mut f32_forward_exact = 0usize;
        let mut f32_reverse_exact = 0usize;
        let mut f32_engine_oriented_exact = 0usize;
        let mut f32_engine_oriented_zero = 0usize;
        let mut higher_owner_forward = 0usize;
        let mut lower_owner_reversed = 0usize;
        let mut owner_attribution_ambiguities = 0usize;
        let mut engine_reversed = 0usize;
        let mut owner_attribution_ambiguity_identities = Vec::new();
        for m in &matches {
            let candidate = &synth[m.chain];
            if candidate.closed || candidate.water_class != BorderWaterClass::WaterWater {
                continue;
            }
            let strip = &pool[m.strip];
            let forward_rows = water_rows_with_f32_window_order(
                &traced[m.chain].chain,
                inputs.width,
                inputs.height,
                false,
            );
            let reverse_rows = water_rows_with_f32_window_order(
                &traced[m.chain].chain,
                inputs.width,
                inputs.height,
                true,
            );
            let forward = emitted_water_centerline(&forward_rows, inputs.border_width * 0.5);
            let reverse = emitted_water_centerline(&reverse_rows, inputs.border_width * 0.5);
            let forward_err = best_centerline_error(&strip.pts, &forward, map_width).0;
            let reverse_err = best_centerline_error(&strip.pts, &reverse, map_width).0;
            f32_forward_exact += usize::from(forward_err < 1e-3);
            f32_reverse_exact += usize::from(reverse_err < 1e-3);
            let engine_oriented_err = if m.rev { reverse_err } else { forward_err };
            engine_reversed += usize::from(m.rev);
            f32_engine_oriented_exact += usize::from(engine_oriented_err < 1e-3);
            f32_engine_oriented_zero += usize::from(engine_oriented_err == 0.0);
            let pool_attribution = majority_side_attribution(
                &strip.sides,
                &inputs.province_ids,
                &region_labels,
                inputs.width,
                inputs.height,
            );
            let synth_attribution = majority_side_attribution(
                &candidate.sides,
                &inputs.province_ids,
                &region_labels,
                inputs.width,
                inputs.height,
            );
            match (pool_attribution, synth_attribution) {
                (
                    SideAttribution::Unique {
                        provinces: pool_provinces,
                        regions: pool_regions,
                        ..
                    },
                    SideAttribution::Unique {
                        provinces: synth_provinces,
                        ..
                    },
                ) if !m.rev
                    && pool_regions.0 > pool_regions.1
                    && synth_provinces == pool_provinces =>
                {
                    higher_owner_forward += 1;
                }
                (
                    SideAttribution::Unique {
                        provinces: pool_provinces,
                        regions: pool_regions,
                        ..
                    },
                    SideAttribution::Unique {
                        provinces: synth_provinces,
                        ..
                    },
                ) if m.rev
                    && pool_regions.0 < pool_regions.1
                    && synth_provinces == (pool_provinces.1, pool_provinces.0) =>
                {
                    lower_owner_reversed += 1;
                }
                (pool_attribution, synth_attribution) => {
                    owner_attribution_ambiguities += 1;
                    owner_attribution_ambiguity_identities.push((
                        m.strip,
                        m.rev,
                        pool_attribution,
                        synth_attribution,
                    ));
                }
            }
        }
        let mut owner_attribution_ambiguity_summary: Vec<_> =
            owner_attribution_ambiguity_identities
                .iter()
                .map(|(strip, reversed, pool_attribution, synth_attribution)| {
                    (
                        *strip,
                        *reversed,
                        matches!(pool_attribution, SideAttribution::Tie { .. }),
                        matches!(synth_attribution, SideAttribution::Tie { .. }),
                    )
                })
                .collect();
        owner_attribution_ambiguity_summary.sort_by_key(|identity| identity.0);
        eprintln!(
                "BORD16 water f32 referee: forward_exact={} reverse_exact={} engine_oriented_exact={} engine_oriented_zero={} engine_reversed={} higher_owner_forward={} lower_owner_reversed={} owner_attribution_ambiguities={} ambiguity_identities={:?}",
                f32_forward_exact,
                f32_reverse_exact,
                f32_engine_oriented_exact,
                f32_engine_oriented_zero,
                engine_reversed,
                higher_owner_forward,
                lower_owner_reversed,
                owner_attribution_ambiguities,
                owner_attribution_ambiguity_identities,
            );
        assert_eq!(
            (
                f32_forward_exact,
                f32_reverse_exact,
                f32_engine_oriented_exact,
                f32_engine_oriented_zero,
            ),
            (8_552, 8_262, 8_578, 8_578),
        );
        assert_eq!(
            (
                higher_owner_forward,
                lower_owner_reversed,
                owner_attribution_ambiguities,
            ),
            (7_872, 698, 8),
        );
        assert_eq!(engine_reversed, 703);
        assert_eq!(
            owner_attribution_ambiguity_summary,
            vec![
                (4_447, true, true, true),
                (7_691, false, true, true),
                (11_119, true, false, false),
                (17_412, false, true, true),
                (25_592, true, false, false),
                (26_711, true, true, true),
                (27_773, true, true, true),
                (40_278, false, true, true),
            ],
        );
        let mut junction_anchor_closed = 0usize;
        let mut before_bytes = 0usize;
        let mut after_bytes = 0usize;
        let mut after_zero = 0usize;
        let mut before_centerline_bytes = 0usize;
        let mut after_centerline_bytes = 0usize;
        let mut after_centerline_zero = 0usize;
        for m in &matches {
            let candidate = &synth[m.chain];
            if m.err >= 1e-3
                || !candidate.closed
                || candidate.water_class != BorderWaterClass::LandWater
            {
                continue;
            }
            let chain = &traced[m.chain].chain;
            let mut points: Vec<[f64; 2]> = Vec::with_capacity(chain.corners.len());
            let mut x_offset = 0.0f64;
            for corner in &chain.corners {
                let mut x = corner.i as f64 + 0.5 + x_offset;
                if let Some(previous) = points.last() {
                    if x - previous[0] > map_width * 0.5 {
                        x_offset -= map_width;
                        x -= map_width;
                    } else if previous[0] - x > map_width * 0.5 {
                        x_offset += map_width;
                        x += map_width;
                    }
                }
                points.push([x, (inputs.height as f64 - 1.5) - corner.j as f64]);
            }
            let old_rows = midpoint_anchor_closed_loop_rows(&points, &chain.junctions);
            let strip = &pool[m.strip];
            if old_rows.len() != strip.rows + 1 {
                continue;
            }
            junction_anchor_closed += 1;
            let reconstructed_engine = engine_vertex_bytes(&strip.sides, false, 0.0);
            assert_eq!(
                reconstructed_engine, strip.bytes,
                "decoded engine side/row layout must preserve every raw VB byte for strip {}",
                strip.index,
            );
            let engine = &strip.bytes;
            let current = engine_vertex_bytes(&candidate.sides, m.rev, m.x_shift);
            let old_sides = emitted_land_sides(
                &old_rows,
                f64::from(inputs.border_width) * 0.5,
                &inputs.heightmap,
            );
            let old = engine_vertex_bytes(&old_sides, m.rev, m.x_shift);
            let current_diff = byte_diff_count(engine, &current);
            let engine_centerline = centerline_bytes(&strip.pts, false, 0.0);
            let current_centerline = centerline_bytes(&candidate.pts, m.rev, m.x_shift);
            let old_centerline_points =
                emitted_land_centerline(&old_rows, f64::from(inputs.border_width) * 0.5);
            let old_centerline = centerline_bytes(&old_centerline_points, m.rev, m.x_shift);
            let current_centerline_diff = byte_diff_count(&engine_centerline, &current_centerline);
            before_bytes += byte_diff_count(engine, &old);
            after_bytes += current_diff;
            after_zero += usize::from(current_diff == 0);
            before_centerline_bytes += byte_diff_count(&engine_centerline, &old_centerline);
            after_centerline_bytes += current_centerline_diff;
            after_centerline_zero += usize::from(current_centerline_diff == 0);
        }
        eprintln!(
                "BORD16 loop byte census closed={} before_diff_bytes={} after_diff_bytes={} after_zero_strips={} before_centerline_diff_bytes={} after_centerline_diff_bytes={} after_centerline_zero_strips={}",
                junction_anchor_closed,
                before_bytes,
                after_bytes,
                after_zero,
                before_centerline_bytes,
                after_centerline_bytes,
                after_centerline_zero,
            );
        assert_eq!(junction_anchor_closed, 48);
        assert_eq!((before_bytes, after_bytes, after_zero), (11_136, 4_958, 0));
        assert_eq!(
            (
                before_centerline_bytes,
                after_centerline_bytes,
                after_centerline_zero,
            ),
            (2_645, 425, 0),
        );
        let mut closed_residuals = Vec::new();
        for strip in pool
            .iter()
            .filter(|strip| pool_strip_is_closed(strip) && !exact_by_pool_strip[strip.index])
        {
            let attribution = majority_side_attribution(
                &strip.sides,
                &inputs.province_ids,
                &region_labels,
                inputs.width,
                inputs.height,
            );
            let SideAttribution::Unique {
                provinces, regions, ..
            } = attribution
            else {
                panic!(
                    "closed engine strip {} side attribution must be unique, got {:?}",
                    strip.index, attribution
                );
            };
            let pair = normalized_pair(provinces.0, provinces.1);
            let class = classify_border_edge(&inputs, pair.0, pair.1)
                .expect("closed engine strip pair classification")
                .water_class;
            closed_residuals.push((strip.index, pair, regions, strip.rows, class));
        }
        assert_eq!(
            closed_residuals,
            vec![
                (
                    13_673,
                    (4_468, 8_898),
                    (11_853, 11_899),
                    42,
                    BorderWaterClass::LandWater,
                ),
                (
                    15_029,
                    (4_912, 5_127),
                    (10_494, 10_214),
                    8,
                    BorderWaterClass::WaterWater,
                ),
                (
                    40_330,
                    (2_970, 13_202),
                    (353, 703),
                    12,
                    BorderWaterClass::LandWater,
                ),
                (
                    40_345,
                    (2_970, 13_203),
                    (353, 475),
                    49,
                    BorderWaterClass::LandWater,
                ),
            ],
        );
        assert_eq!(baseline_matches.len(), 40_324);
        assert_eq!(bord12_matches.len(), 40_630);
        assert_eq!(matches.len(), 40_678);
        assert_eq!(baseline_exact_count, 40_284);
        assert_eq!(bord12_exact_count, 40_606);
        assert_eq!(exact_count, 40_654);
        assert_eq!((gained_exact_strips, lost_exact_strips), (408, 38));
        assert_eq!(
            pair_edge_lost_exact_identities,
            vec![
                (
                    169,
                    51,
                    9_003,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    71
                ),
                (
                    1_389,
                    51,
                    456,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    55
                ),
                (
                    1_950,
                    456,
                    640,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    43
                ),
                (
                    2_557,
                    262,
                    838,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    65
                ),
                (
                    3_855,
                    838,
                    1_264,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    55
                ),
                (
                    7_340,
                    2_353,
                    2_403,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    24
                ),
                (
                    7_713,
                    2_526,
                    2_574,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    44
                ),
                (
                    7_970,
                    2_607,
                    3_836,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    19
                ),
                (
                    8_548,
                    2_795,
                    2_820,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    37
                ),
                (
                    8_619,
                    2_820,
                    2_874,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    33
                ),
                (
                    9_029,
                    2_946,
                    10_145,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    5
                ),
                (
                    9_076,
                    2_914,
                    2_962,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    60
                ),
                (
                    9_112,
                    2_970,
                    13_202,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    3
                ),
                (
                    9_113,
                    2_970,
                    13_202,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    10
                ),
                (
                    9_124,
                    2_970,
                    13_203,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    29
                ),
                (
                    9_183,
                    2_962,
                    2_988,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    79
                ),
                (
                    11_780,
                    2_988,
                    3_832,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    40
                ),
                (
                    11_791,
                    3_046,
                    3_836,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    19
                ),
                (
                    13_674,
                    4_468,
                    8_898,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    13
                ),
                (
                    15_029,
                    4_912,
                    5_127,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    8
                ),
                (
                    16_128,
                    5_257,
                    7_386,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    6
                ),
                (
                    16_129,
                    5_257,
                    7_386,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    21
                ),
                (
                    16_236,
                    5_288,
                    8_956,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    33
                ),
                (
                    16_369,
                    3_832,
                    5_332,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    45
                ),
                (
                    16_442,
                    5_332,
                    5_355,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    64
                ),
                (
                    16_741,
                    5_355,
                    5_450,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    54
                ),
                (
                    16_813,
                    5_450,
                    5_475,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    51
                ),
                (
                    17_767,
                    2_607,
                    5_782,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    61
                ),
                (
                    21_993,
                    7_202,
                    8_363,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    4
                ),
                (
                    21_994,
                    7_202,
                    8_363,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    26
                ),
                (
                    22_234,
                    7_052,
                    7_277,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    72
                ),
                (
                    25_410,
                    640,
                    8_311,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    56
                ),
                (
                    26_224,
                    7_277,
                    8_567,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    75
                ),
                (
                    27_217,
                    4_468,
                    8_898,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    5
                ),
                (
                    27_395,
                    8_567,
                    8_955,
                    BorderRank::Province,
                    BorderWaterClass::WaterWater,
                    false,
                    55
                ),
                (
                    28_785,
                    8_418,
                    9_405,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    18
                ),
                (
                    31_006,
                    2_946,
                    10_145,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    4
                ),
                (
                    40_331,
                    2_970,
                    13_202,
                    BorderRank::Province,
                    BorderWaterClass::LandWater,
                    false,
                    17
                ),
            ],
        );
        assert_eq!(pair_edge_loss_classes.len(), 2);
        assert_eq!(
            pair_edge_loss_classes
                .get(&(BorderRank::Province, BorderWaterClass::LandWater, false,)),
            Some(&13),
        );
        assert_eq!(
            pair_edge_loss_classes.get(&(
                BorderRank::Province,
                BorderWaterClass::WaterWater,
                false,
            )),
            Some(&25),
        );
        assert_eq!((land_open_exact, land_open_total), (27_024, 27_024));
        assert_eq!((water_open_exact, water_open_total), (8_557, 8_578));
        assert_eq!(water_open_residuals.len(), 21);
        assert_eq!(sea_region_exact, 8_557);
        assert_eq!(pool_closed_total, 928);
        assert_eq!(bord12_exact_closed_loops, 876);
        assert_eq!(bord12_pool_closed_unmatched_exact, 52);
        assert_eq!(exact_closed_loops, 924);
        assert_eq!(exact_closed_coastline_loops, exact_closed_loops);
        assert_eq!(pool_closed_unmatched_exact, 4);
        assert_eq!(
            bord16_closed_identity_gains,
            vec![
                335, 1_852, 3_554, 4_612, 4_632, 5_581, 5_726, 5_957, 7_847, 8_851, 9_121, 9_122,
                9_231, 9_282, 10_022, 12_753, 13_563, 13_902, 14_293, 14_456, 16_002, 16_133,
                16_462, 17_496, 18_663, 22_185, 22_651, 22_891, 24_435, 25_390, 25_768, 26_242,
                26_491, 27_363, 30_996, 31_234, 31_515, 36_189, 37_500, 38_288, 38_329, 38_330,
                39_787, 39_849, 39_938, 40_332, 40_333, 40_921,
            ],
        );
        assert!(bord16_closed_identity_losses.is_empty());
        assert_eq!(lost_exact_closed_loops, 0);
        assert_eq!(gained_exact_closed_loops, 212);
        assert_eq!(newly_exact_closed_from_unmatched, 173);
        assert_eq!(reclassified_exact_open_as_closed, 39);
    }

    #[test]
    fn class_index_partition_sums_to_total_and_permutes_quads() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        let partition = mesh.class_index_partition();
        assert_eq!(partition.indices.len(), mesh.indices.len());
        let total: u32 = partition.ranges.iter().map(|range| range.index_count).sum();
        assert_eq!(total as usize, mesh.indices.len());
        let mut cursor = 0u32;
        for range in &partition.ranges {
            assert_eq!(range.start_index, cursor);
            cursor += range.index_count;
        }
        let mut original: Vec<[u32; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]])
            .collect();
        let mut reordered: Vec<[u32; 3]> = partition
            .indices
            .chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]])
            .collect();
        original.sort();
        reordered.sort();
        assert_eq!(original, reordered);
        for range in &partition.ranges {
            let start = range.start_index as usize;
            let end = start + range.index_count as usize;
            for &index in &partition.indices[start..end] {
                let chain = mesh
                    .chains
                    .iter()
                    .find(|chain| {
                        index >= chain.first_vertex
                            && index < chain.first_vertex + chain.vertex_count()
                    })
                    .expect("index maps to a chain");
                assert_eq!(chain.material, range.material);
            }
        }
    }

    #[test]
    fn impassable_adjacency_pair_overrides_country_rank() {
        let mut inputs = synthetic_inputs();
        inputs.impassable_pairs.insert((3, 4));
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        let chain = mesh
            .chains
            .iter()
            .find(|chain| chain.province_a == 3 && chain.province_b == 4)
            .expect("3|4 chain");
        assert_eq!(chain.material, BorderMaterial::Impassable);
        assert_eq!(chain.rank, BorderRank::State);
        assert!(mesh.stats.impassable_adjacency_edges > 0);
    }

    #[test]
    fn impassable_adjacency_pair_overrides_same_state_province_rank() {
        let mut inputs = synthetic_inputs();
        inputs.impassable_pairs.insert((1, 2));
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        let chain = mesh
            .chains
            .iter()
            .find(|chain| chain.province_a == 1 && chain.province_b == 2)
            .expect("1|2 chain");
        assert_eq!(chain.material, BorderMaterial::Impassable);
    }

    #[test]
    fn closed_loop_chain_is_detected_and_closed() {
        let width = 4;
        let height = 4;
        let province_ids = vec![1, 1, 1, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 1, 1, 1];
        let inputs = BorderSynthesisInputs {
            province_ids,
            width,
            height,
            definitions: BTreeMap::from([(1, definition("land")), (2, definition("land"))]),
            state_meta: BTreeMap::from([(10, state(vec![1, 2], false))]),
            province_to_state: BTreeMap::from([(1, 10), (2, 10)]),
            state_owners: BTreeMap::from([(10, "AAA".to_string())]),
            heightmap: IndexImage {
                width,
                height,
                pixels: vec![100; width * height],
            },
            border_width: 1.5,
            unknown_pixels: 0,
            state_files: 1,
            impassable_pairs: BTreeSet::new(),
            impassable_adjacency_non_land_rows: 0,
        };
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        assert_eq!(mesh.stats.closed_loop_chains, 1);
        let chain = mesh
            .chains
            .iter()
            .find(|chain| chain.closed)
            .expect("closed chain");
        assert_eq!(chain.manhattan_length, 8);
        assert_eq!(chain.rows, 9);
        let centerline = mesh.chain_centerline(chain);
        let first = centerline.first().unwrap();
        let last = centerline.last().unwrap();
        assert!((first[0] - last[0]).abs() < 1e-5 && (first[2] - last[2]).abs() < 1e-5);
    }

    #[test]
    fn x_wrap_chain_uses_unwrapped_coordinates() {
        let width = 4;
        let height = 2;
        let province_ids = vec![1, 2, 2, 2, 2, 2, 2, 2];
        let inputs = BorderSynthesisInputs {
            province_ids,
            width,
            height,
            definitions: BTreeMap::from([(1, definition("land")), (2, definition("land"))]),
            state_meta: BTreeMap::from([(10, state(vec![1, 2], false))]),
            province_to_state: BTreeMap::from([(1, 10), (2, 10)]),
            state_owners: BTreeMap::from([(10, "AAA".to_string())]),
            heightmap: IndexImage {
                width,
                height,
                pixels: vec![100; width * height],
            },
            border_width: 1.5,
            unknown_pixels: 0,
            state_files: 1,
            impassable_pairs: BTreeSet::new(),
            impassable_adjacency_non_land_rows: 0,
        };
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        assert!(mesh.stats.x_wrap_edges > 0);
        for chain in &mesh.chains {
            let centerline = mesh.chain_centerline(chain);
            for window in centerline.windows(2) {
                let dx = (window[1][0] - window[0][0]).abs();
                let dz = (window[1][2] - window[0][2]).abs();
                assert!(
                    dx <= 2.0 && dz <= 2.0,
                    "chain ({}, {}) has a seam-spanning step",
                    chain.province_a,
                    chain.province_b
                );
            }
        }
    }

    #[test]
    fn parse_impassable_adjacencies_reads_impassable_pairs_and_skips_sea_and_blank() {
        let dir = temp_test_dir("border_adjacencies");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("adjacencies.csv");
        let mut file = fs::File::create(&path).expect("create adjacencies.csv");
        writeln!(
            file,
            "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment"
        )
        .unwrap();
        writeln!(file, "6402;6522;impassable;-1;-1;-1;-1;-1;;Comment").unwrap();
        writeln!(file, "1234;977;sea;9086;3110;1195;3016;1178;;").unwrap();
        writeln!(file, "6522;6402;impassable;-1;-1;-1;-1;-1;;dupe reversed").unwrap();
        writeln!(file, "10;20;;;;;;;GIBRALTAR_STRAIT;strait row").unwrap();
        writeln!(file, "-1;-1;;-1;-1;-1;-1;-1;;").unwrap();
        drop(file);
        let pairs = parse_impassable_adjacencies(&path).expect("parse");
        assert_eq!(pairs, BTreeSet::from([(6402, 6522)]));
    }

    #[test]
    fn impassable_adjacency_pair_fields_fail_closed_on_typos_or_absence() {
        for (case, row, expected) in [
            (
                "nonnumeric_from",
                "oops;6522;impassable;-1;-1;-1;-1;-1;;",
                "non-numeric From",
            ),
            (
                "nonnumeric_to",
                "6402;oops;impassable;-1;-1;-1;-1;-1;;",
                "non-numeric To",
            ),
            ("missing_to", "6402", "From/To/Type are required"),
            (
                "one_sided_sentinel",
                "-1;6522;;-1;-1;-1;-1;-1;;",
                "non-numeric From",
            ),
        ] {
            let dir = temp_test_dir(case);
            fs::create_dir_all(&dir).expect("create temp dir");
            let path = dir.join("adjacencies.csv");
            fs::write(
                &path,
                format!(
                    "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n{row}\n"
                ),
            )
            .expect("write adjacencies.csv");
            let err = parse_impassable_adjacencies(&path).expect_err("malformed row must fail");
            assert!(
                err.to_string().contains(expected),
                "case {case}: unexpected error {err}"
            );
        }
    }

    #[test]
    fn sea_crossing_row_does_not_reclassify_raster_edges() {
        let mut inputs = synthetic_inputs();
        inputs.impassable_pairs.clear();
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        let chain = mesh
            .chains
            .iter()
            .find(|chain| chain.province_a == 1 && chain.province_b == 2)
            .expect("1|2 chain");
        assert_eq!(chain.material, BorderMaterial::Province);
    }

    #[test]
    fn unknown_adjacency_type_fails_closed_naming_the_type() {
        let dir = temp_test_dir("border_adjacencies_unknown");
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("adjacencies.csv");
        let mut file = fs::File::create(&path).expect("create adjacencies.csv");
        writeln!(
            file,
            "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment"
        )
        .unwrap();
        writeln!(file, "1;2;canal_typo;-1;-1;-1;-1;-1;;").unwrap();
        drop(file);
        let err = parse_impassable_adjacencies(&path).expect_err("must fail");
        assert!(err.to_string().contains("canal_typo"), "{err}");
    }

    #[test]
    fn impassable_adjacency_referencing_sea_province_is_counted_not_applied() {
        let mut inputs = synthetic_inputs();
        inputs.impassable_pairs.insert((2, 5));
        inputs.impassable_adjacency_non_land_rows = 1;
        let mesh = synthesize_border_world_mesh_from_inputs(&inputs).expect("mesh");
        let chain = mesh
            .chains
            .iter()
            .find(|chain| chain.province_a == 2 && chain.province_b == 5)
            .expect("2|5 chain");
        assert_eq!(chain.material, BorderMaterial::SeaCoast);
        assert_eq!(mesh.stats.impassable_adjacency_non_land_rows, 1);
    }

    #[test]
    fn unknown_province_id_fails_closed() {
        let mut inputs = synthetic_inputs();
        inputs.definitions.remove(&4);
        let err = synthesize_border_world_mesh_from_inputs(&inputs).expect_err("must fail");
        assert!(err.to_string().contains("province id 4"), "{err}");
    }

    #[test]
    fn missing_state_for_land_edge_fails_closed() {
        let mut inputs = synthetic_inputs();
        inputs.province_to_state.remove(&4);
        let err = synthesize_border_world_mesh_from_inputs(&inputs).expect_err("must fail");
        assert!(err.to_string().contains("no 1936 state"), "{err}");
    }

    #[test]
    fn resolver_missing_file_fails_closed() {
        let dir = temp_test_dir("border_resolver_missing");
        fs::create_dir_all(&dir).expect("create temp dir");
        let resolver = GameFileResolver::vanilla(&dir).expect("resolver");
        let err = synthesize_border_world_mesh_from_resolver(&resolver).expect_err("must fail");
        assert!(err.to_string().contains("map/default.map"), "{err}");
    }

    fn write_minimal_border_game_files(root: &Path, include_all_colors: bool) {
        fs::create_dir_all(root.join("map/terrain")).unwrap();
        fs::create_dir_all(root.join("common/defines")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        fs::write(
            root.join("map/default.map"),
            "definitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"\nheightmap = \"heightmap.bmp\"\n",
        )
        .unwrap();
        let mut definition_csv = String::from("0;0;0;0;land;false;unknown;0\n");
        definition_csv.push_str("1;255;0;0;land;false;plains;1\n");
        if include_all_colors {
            definition_csv.push_str("2;0;255;0;land;false;plains;1\n");
        }
        fs::write(root.join("map/definition.csv"), definition_csv).unwrap();
        fs::write(
            root.join("map/adjacencies.csv"),
            "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n-1;-1;;-1;-1;-1;-1;-1;;\n",
        )
        .unwrap();
        write_bmp24(
            &root.join("map/provinces.bmp"),
            2,
            1,
            &[[0, 0, 255], [0, 255, 0]],
        );
        write_bmp8(&root.join("map/heightmap.bmp"), 2, 1, &[100, 100]);
        fs::write(
            root.join("common/defines/00_graphics.lua"),
            "NGraphics = {\n\tBORDER_WIDTH = 1.5,\n\tPROVINCE_BORDER_FADE_NEAR = 200,\n\tPROVINCE_BORDER_FADE_FAR = 300,\n\tSTATE_BORDER_FADE_NEAR = 400,\n\tSTATE_BORDER_FADE_FAR = 500,\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("history/states/1-One.txt"),
            "state = {\n\tid = 1\n\tname = \"STATE_1\"\n\tprovinces = { 1 2 }\n\thistory = { owner = AAA }\n}\n",
        )
        .unwrap();
    }

    fn write_bmp24(path: &Path, width: usize, height: usize, pixels: &[[u8; 3]]) {
        assert_eq!(pixels.len(), width * height);
        let row_bytes = (width * 3 + 3) & !3;
        let image_bytes = row_bytes * height;
        let mut bytes = Vec::with_capacity(54 + image_bytes);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(54u32 + image_bytes as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        bytes.extend_from_slice(&(height as i32).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 24]);
        for y in (0..height).rev() {
            let mut row = Vec::with_capacity(row_bytes);
            for x in 0..width {
                let [b, g, r] = pixels[y * width + x];
                row.extend_from_slice(&[b, g, r]);
            }
            row.resize(row_bytes, 0);
            bytes.extend_from_slice(&row);
        }
        fs::write(path, bytes).unwrap();
    }

    fn write_bmp8(path: &Path, width: usize, height: usize, pixels: &[u8]) {
        assert_eq!(pixels.len(), width * height);
        let row_bytes = (width + 3) & !3;
        let image_bytes = row_bytes * height;
        let palette_bytes = 256 * 4;
        let offset = 54 + palette_bytes;
        let mut bytes = Vec::with_capacity(offset + image_bytes);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&((offset + image_bytes) as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(offset as u32).to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        bytes.extend_from_slice(&(height as i32).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 24]);
        for index in 0..256u32 {
            let v = index as u8;
            bytes.extend_from_slice(&[v, v, v, 0]);
        }
        for y in (0..height).rev() {
            let mut row = Vec::with_capacity(row_bytes);
            for x in 0..width {
                row.push(pixels[y * width + x]);
            }
            row.resize(row_bytes, 0);
            bytes.extend_from_slice(&row);
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn resolver_unknown_provinces_bmp_color_fails_closed() {
        let dir = temp_test_dir("border_resolver_unknown_color");
        write_minimal_border_game_files(&dir, false);
        let resolver = GameFileResolver::vanilla(&dir).expect("resolver");
        let err = synthesize_border_world_mesh_from_resolver(&resolver).expect_err("must fail");
        assert!(err.to_string().contains("unknown pixel"), "{err}");
    }

    #[test]
    fn resolver_single_station_records_fail_closed() {
        let dir = temp_test_dir("border_resolver_minimal");
        write_minimal_border_game_files(&dir, true);
        let resolver = GameFileResolver::vanilla(&dir).expect("resolver");
        let error =
            synthesize_border_world_mesh_from_resolver(&resolver).expect_err("must fail closed");
        assert!(
            error
                .to_string()
                .contains("degenerate ribbon direction at row 0"),
            "{error}"
        );
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn gate_vanilla_captured_border_strip_evidence() {
        let game_dir =
            std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")).expect("ignored evidence gate requires BATTLE_PLAN_GAME_DIR");
        const CAPTURED: [(f32, f32, f32, i32, i32); 32] = [
            (2780.750, 9.887, 1483.500, 0, 0),
            (2782.250, 9.950, 1483.500, 1, 0),
            (2780.756, 9.800, 1482.740, 0, 1),
            (2782.244, 9.893, 1482.926, 1, 1),
            (2780.965, 9.790, 1481.903, 0, 2),
            (2782.369, 9.843, 1482.430, 1, 2),
            (2781.329, 9.716, 1481.165, 0, 3),
            (2782.671, 9.784, 1481.835, 1, 3),
            (2781.662, 9.700, 1480.498, 0, 4),
            (2783.004, 9.717, 1481.169, 1, 4),
            (2781.996, 9.700, 1479.831, 0, 5),
            (2783.338, 9.734, 1480.502, 1, 5),
            (2782.329, 9.700, 1479.165, 0, 6),
            (2783.671, 9.767, 1479.835, 1, 6),
            (2782.606, 9.700, 1478.651, 0, 7),
            (2784.061, 9.812, 1479.015, 1, 7),
            (2782.606, 9.700, 1478.349, 0, 8),
            (2784.061, 9.798, 1477.985, 1, 8),
            (2782.329, 9.700, 1477.835, 0, 9),
            (2783.671, 9.711, 1477.165, 1, 9),
            (2781.939, 9.700, 1477.015, 0, 10),
            (2783.394, 9.700, 1476.651, 1, 10),
            (2781.939, 9.700, 1475.985, 0, 11),
            (2783.394, 9.700, 1476.349, 1, 11),
            (2782.329, 9.700, 1475.165, 0, 12),
            (2783.671, 9.700, 1475.835, 1, 12),
            (2782.631, 9.700, 1474.570, 0, 13),
            (2784.035, 9.700, 1475.097, 1, 13),
            (2782.756, 9.700, 1474.074, 0, 14),
            (2784.244, 9.700, 1474.260, 1, 14),
            (2782.750, 9.700, 1473.500, 0, 15),
            (2784.250, 9.700, 1473.500, 1, 15),
        ];
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let mesh = synthesize_border_world_mesh_from_resolver(&resolver).expect("mesh");
        let chain = mesh
            .chains
            .iter()
            .find(|chain| {
                normalized_pair(chain.province_a, chain.province_b) == (527, 9495)
                    && chain.rows == 16
            })
            .expect("the captured Brittany|Loire chain (pair 527|9495, 16 rows)");
        assert_eq!(chain.material, BorderMaterial::State, "state-class border");
        assert!(!chain.closed);
        for (offset, &(x, y, z, u, v)) in CAPTURED.iter().enumerate() {
            let vertex = &mesh.vertices[chain.first_vertex as usize + offset];
            assert_eq!(vertex.uv, [u, v], "uv of vertex {offset}");
            let dx = (vertex.position[0] - x).abs();
            let dy = (vertex.position[1] - y).abs();
            let dz = (vertex.position[2] - z).abs();
            assert!(
                dx < 1e-3 && dy < 1.5e-3 && dz < 1e-3,
                "vertex {offset}: synthesized {:?} != captured ({x}, {y}, {z})",
                vertex.position
            );
        }
    }
}
