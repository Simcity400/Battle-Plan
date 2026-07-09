//! Files-first border geometry synthesis for the game thin border pass.
//!
//! ## BORD5: the geometry law is CAPTURED-ENGINE-DERIVED (2026-07-07)
//!
//! `session_BORDERZOOM_3` recorded a genuine `border.shader` draw (state-class
//! border between provinces 9495 Brittany | 527 Loire, both FRA) together with
//! the engine's ENTIRE resident border vertex pool: 31,824,096 bytes, stride 16
//! (`float3` position + `R16G16` integer uv), decomposing into 40,959 ribbon
//! strips covering 100.0% of the pool. Every strip is one border *chain* (a
//! maximal run of province-boundary lattice edges). The construction below
//! reproduces 31,528 of those strips EXACTLY (max coordinate error 3.3e-4 =
//! f32 noise), including 27,014 of 27,048 land/land chains and 3,901 land/water
//! (coastline) chains:
//!
//! 1. Trace the province-pair boundary staircase on the pixel-corner lattice.
//!    A chain ends at any corner whose pair-boundary degree != 2 (junctions and
//!    endpoints); it passes straight through 3-province corners where the same
//!    pair continues.
//! 2. Place stations along the staircase at Manhattan arc positions
//!    `{0, 0.5, 1.5, 2.5, ..., M-0.5, M}` where `M` = chain edge count
//!    (so an open chain has `M + 2` rows).
//! 3. Apply ONE box-3 smoothing pass `P_i = (P_{i-1} + P_i + P_{i+1}) / 3`
//!    with both endpoints fixed.
//! 4. Extrude the ribbon: per-row direction = central difference
//!    `P_{k+1} - P_{k-1}` (one-sided at the ends); the `uv.x = 0` side sits at
//!    `P + (BORDER_WIDTH/2) * perp` with `perp = (dz, -dx)/|d|`, the
//!    `uv.x = 1` side at `P - (BORDER_WIDTH/2) * perp`. `BORDER_WIDTH = 1.5`
//!    (`NGraphics.BORDER_WIDTH`); all 40,959 captured strips measure exactly
//!    1.5 across (max err 5e-4).
//!
//! Coordinate conventions (pinned by the same capture):
//! * province pixel `(x, y)` center = world `(x, (H-1) - y)`;
//! * corner lattice point `(i, j)` = world `(i + 0.5, (H-1) - j - 0.5)`;
//! * heightmap grid vertex `(i, j)` = texel `(i, (H-1) - j)`, height =
//!   `raw * 0.1`; the captured border vertex heights match BILINEAR
//!   interpolation of that grid to 5e-4 (all 32 vertices of the captured
//!   draw).
//! * `uv = (across_edge 0|1, row index k)` as integers; the compiled border VS
//!   contains exactly one `itof` — the engine feeds integer uv lanes.
//!
//! Pinpointed gaps (NOT invented around — see FIDELITY_LEDGER 2.5):
//! * water|water chains (sea_region material) use a DIFFERENT, unknown
//!   centerline construction (box3 iterated up to 80x refuted on 294/300
//!   sampled sea strips). They are synthesized here for stats/completeness but
//!   the render pass skips the SeaRegion class until that law is derived.
//! * closed-loop chains (no junction corner, ~0.13% of land chains) have an
//!   unverified anchor rule; we start them at their lexicographically smallest
//!   corner and smooth with fixed (duplicated) endpoints, and count them in
//!   `closed_loop_chains`.
//! * the engine's chain DIRECTION (which end is row 0) is not fully pinned;
//!   we orient chains north/west-first, which reproduces the captured Brittany
//!   strip exactly. A flipped chain flips both row order and the L/R sides, so
//!   triangle winding (and back-face culling) is direction-invariant.

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
/// `float3 position` (12) + `int2 uv` (8). The engine packs uv as `R16G16`
/// (stride 16); the VALUES are small integers either way, and the shader input
/// is `int2`, so 32-bit integer lanes are semantically identical.
pub const BORDER_VERTEX_STRIDE_BYTES: usize = 20;

const BORDER_TEXTURES: &[(BorderMaterial, &str)] = &[
    (
        BorderMaterial::Province,
        "map/terrain/border_province_0.dds",
    ),
    (BorderMaterial::State, "map/terrain/border_state_0.dds"),
    (BorderMaterial::Country, "map/terrain/border_country_0.dds"),
    (BorderMaterial::SeaCoast, "map/terrain/border_sea_0.dds"),
    (
        BorderMaterial::SeaRegion,
        "map/terrain/border_sea_region_0.dds",
    ),
    (
        BorderMaterial::Impassable,
        "map/terrain/border_impassable_0.dds",
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
    SeaCoast,
    SeaRegion,
    Impassable,
}

impl BorderMaterial {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Province => "province",
            Self::State => "state",
            Self::Country => "country",
            Self::SeaCoast => "sea_coast",
            Self::SeaRegion => "sea_region",
            Self::Impassable => "impassable",
        }
    }
}

/// One border chain: a maximal run of province-pair boundary lattice edges
/// rendered as one continuous ribbon strip (the captured engine pool is
/// exactly one such strip per chain).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BorderChain {
    pub province_a: u32,
    pub province_b: u32,
    pub state_a: Option<u32>,
    pub state_b: Option<u32>,
    pub rank: BorderRank,
    pub water_class: BorderWaterClass,
    pub material: BorderMaterial,
    /// True when the chain is a closed loop (no pair-degree != 2 corner). The
    /// loop anchor rule is UNVERIFIED against the engine (see module docs).
    pub closed: bool,
    /// Chain edge count `M` (each lattice edge is 1 world unit long).
    pub manhattan_length: u32,
    /// Ribbon row count (`M + 2` for open chains; `M + 2` for loops too — the
    /// start corner is duplicated as the final row to close the ribbon).
    pub rows: u32,
    /// First vertex of this chain in [`BorderWorldMesh::vertices`]; the chain
    /// owns `rows * 2` vertices interleaved `(uv.x=0, uv.x=1)` per row.
    pub first_vertex: u32,
    /// First index of this chain in [`BorderWorldMesh::indices`]; the chain
    /// owns `6 * (rows - 1)` indices.
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

/// One contiguous run of indices belonging to a single border material, as a
/// `DrawIndexed(index_count, start_index, base_vertex=0)` range into the
/// reordered index buffer from [`BorderWorldMesh::class_index_partition`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BorderClassIndexRange {
    pub material: BorderMaterial,
    /// First index into the reordered index buffer (`start_index` for
    /// `DrawIndexed`).
    pub start_index: u32,
    /// Number of indices in this class's run (`index_count` for `DrawIndexed`).
    pub index_count: u32,
}

/// The reordered index buffer grouped by material plus the per-class draw
/// ranges. The reordered indices reference the SAME vertex buffer as
/// [`BorderWorldMesh::vertices`]; only the index ORDER changes so that each
/// material's chains form one contiguous `DrawIndexed` run.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderClassIndexPartition {
    /// Indices reordered so every material's chains are contiguous, in
    /// [`BorderClassIndexPartition::MATERIAL_DRAW_ORDER`].
    pub indices: Vec<u32>,
    /// One range per material that has at least one chain, in draw order.
    pub ranges: Vec<BorderClassIndexRange>,
}

impl BorderClassIndexPartition {
    /// Back-to-front-ish material draw order: coarse/water classes first, then
    /// the finer land ranks, so finer borders overdraw coarser ones where
    /// ribbons overlap at junctions (province last = on top). This is a
    /// rendering-order choice, not a shipped law; it is the same order the CPU
    /// debug rasterizer paints in (`bin/dump_border_synthesis.rs`), kept
    /// consistent so the GPU pass and the debug dump agree on which class wins
    /// an overlap.
    pub const MATERIAL_DRAW_ORDER: [BorderMaterial; 6] = [
        BorderMaterial::SeaRegion,
        BorderMaterial::SeaCoast,
        BorderMaterial::Country,
        BorderMaterial::Impassable,
        BorderMaterial::State,
        BorderMaterial::Province,
    ];
}

impl BorderWorldMesh {
    pub fn to_shader_vertex_layout_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.vertices.len() * BORDER_VERTEX_STRIDE_BYTES);
        for vertex in &self.vertices {
            for value in vertex.position {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    /// Partition the mesh's indices into contiguous per-material runs.
    ///
    /// Each chain owns the contiguous index run
    /// `indices[chain.first_index .. chain.first_index + chain.index_count()]`.
    /// This reorders those per-chain runs by material (in
    /// [`BorderClassIndexPartition::MATERIAL_DRAW_ORDER`]) into a new index
    /// buffer, and returns one draw range per non-empty material.
    ///
    /// Invariant (unit-tested): the per-class `index_count`s sum to the total
    /// index count, the ranges are contiguous and non-overlapping, and the
    /// reordered buffer is a permutation of the original per-chain runs (so it
    /// draws exactly the same triangles, just grouped).
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

    /// The smoothed centerline of one chain (midpoint of each row's two
    /// vertices), for the CPU debug dumps.
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
    /// Raster boundary edges (unit lattice edges between differing provinces).
    pub raster_edges: usize,
    /// Border chains (one ribbon strip each).
    pub chains: usize,
    /// Chains that are closed loops (unverified anchor rule; see module docs).
    pub closed_loop_chains: usize,
    pub vertices: usize,
    pub indices: usize,
    /// Total Manhattan length of all chains (== raster_edges as f64).
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
    /// Distinct province pairs declared `impassable` in `map/adjacencies.csv`.
    pub impassable_adjacency_pairs: usize,
    /// Raster edges reclassified to `impassable` because their exact land/land
    /// province pair is declared `impassable` in `map/adjacencies.csv`.
    pub impassable_adjacency_edges: usize,
    /// `impassable` adjacency rows referencing a non-land province (data
    /// inconsistency); these do not override any coastline edge.
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
    /// Province pairs declared `impassable` in `map/adjacencies.csv`, stored as
    /// unordered `(min, max)` keys. These override the province/state/country
    /// rank for the exact land/land raster edge between the pair.
    impassable_pairs: BTreeSet<(u32, u32)>,
    /// Count of `impassable` adjacency rows whose pair is not a land/land pair
    /// per `definition.csv` (e.g. references a sea/lake province). These do not
    /// override any coastline edge; they are surfaced for validation.
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

    // An `impassable` adjacency row only overrides a land/land raster edge. A
    // row that references a sea/lake province (per definition.csv) is a data
    // inconsistency that must not turn a coastline into an impassable border;
    // it is counted and surfaced instead of silently applied.
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

/// Parse `map/adjacencies.csv` and return the set of province pairs declared
/// `impassable`, keyed as unordered `(min, max)`.
///
/// Header semantics (line 1):
/// `From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment`.
/// Only the `From`, `To`, and `Type` columns are consumed here. Rows are
/// skipped when they are empty, start with `#` (the shipped legend line), or
/// have a non-numeric `From` field (the header row and the `-1;-1;;...`
/// terminator sentinel both fall out here because `From` fails `u32` parsing).
/// A blank `Type` marks a strait/adjacency-rule crossing (e.g.
/// `GIBRALTAR_STRAIT`) and is not a border material, so it is ignored. `sea`
/// rows connect provinces across water and never coincide with a land raster
/// edge, so they are ignored. Any other non-empty `Type` fails closed with the
/// offending value named.
fn parse_impassable_adjacencies(path: &std::path::Path) -> Result<BTreeSet<(u32, u32)>> {
    let text = read_latin1(path)?;
    let mut impassable = BTreeSet::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(';');
        let from_field = fields.next().unwrap_or("").trim();
        let to_field = fields.next().unwrap_or("").trim();
        let type_field = fields.next().unwrap_or("").trim();

        // Non-numeric `From` (header row) or the `-1;-1` terminator sentinel
        // are skipped before any classification: they carry no province pair.
        let Ok(from) = from_field.parse::<u32>() else {
            continue;
        };
        let Ok(to) = to_field.parse::<u32>() else {
            continue;
        };

        match type_field.to_ascii_lowercase().as_str() {
            "impassable" => {
                let pair = if from <= to { (from, to) } else { (to, from) };
                impassable.insert(pair);
            }
            // Blank type: strait/adjacency-rule crossing, not a border material.
            "" => {}
            // Sea crossing: connects provinces across water; never a land edge.
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

/// True iff both provinces of an adjacency pair are `land` in `definition.csv`.
/// Returns `None` if either province id is absent from `definition.csv`.
fn adjacency_pair_is_land_land(
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    a: u32,
    b: u32,
) -> Option<bool> {
    let type_a = definitions.get(&a)?.province_type.as_str();
    let type_b = definitions.get(&b)?.province_type.as_str();
    Some(type_a == "land" && type_b == "land")
}

/// A corner of the pixel lattice. Corner `(i, j)` sits between pixels
/// `(i, j)/(i+1, j)` (above) and `(i, j+1)/(i+1, j+1)` (below), with `i`
/// wrapped modulo the map width. World position: `(i + 0.5, (H-1) - j - 0.5)`.
/// `j` ranges `-1 ..= H-1` (corners on the top/bottom map edges included).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Corner {
    /// Ordered `(j, i)` so the natural ordering is north-most-then-west-most,
    /// the orientation rule that reproduces the captured Brittany strip.
    j: i32,
    i: u32,
}

/// One unit lattice edge of a province boundary.
/// * `Vertical(i, j)`: corners `(i, j)-(i, j+1)`, separating pixels
///   `(i, j+1)` and `((i+1) % W, j+1)`.
/// * `Horizontal(i, j)`: corners `(i, j)-((i+1) % W, j)`, separating pixels
///   `((i+1) % W, j)` and `((i+1) % W, j+1)`.
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
            renderer_note: "BORD5 captured-engine chain law (session_BORDERZOOM_3): stations \
                            {0, 0.5, .., M-0.5, M} + one fixed-end box3 pass + 0.75 central-diff \
                            extrusion; 31,528/40,959 captured strips reproduced exactly"
                .to_string(),
        },
    };

    for (material, _) in BORDER_TEXTURES {
        mesh.stats
            .width_by_material
            .insert(material.as_str().to_string(), inputs.border_width);
    }

    // ---- 1. enumerate raster boundary edges, classified per province pair ----
    let width = inputs.width;
    let height = inputs.height;
    let mut pair_edges: BTreeMap<(u32, u32), Vec<LatticeEdge>> = BTreeMap::new();
    let mut classified: BTreeMap<(u32, u32), ClassifiedBorderEdge> = BTreeMap::new();
    // Per-pixel LandWater flags for the independent coastline cross-check.
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

    // ---- 2. per pair: split edges into chains, emit each as one ribbon ----
    for (&(province_a, province_b), edges) in &pair_edges {
        let edge_class = classified[&(province_a, province_b)];
        let chains = chains_from_pair_edges(edges, width as u32)?;
        for chain_corners in chains {
            emit_chain_ribbon(
                &mut mesh,
                inputs,
                province_a,
                province_b,
                edge_class,
                &chain_corners,
            )?;
        }
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

/// A walked chain: its corner path plus whether it is a closed loop.
struct ChainCorners {
    corners: Vec<Corner>,
    closed: bool,
}

/// Split one pair's boundary edge set into maximal chains.
///
/// Chains end at any corner whose pair-boundary degree != 2 — endpoints
/// (degree 1) and junction/checkerboard corners (degree 3 or 4). This is the
/// rule the captured pool obeys: the engine's strips pass straight through
/// corners where a third province touches but the pair continues (degree 2),
/// and 31,528 captured strips reproduce exactly under it. Leftover edges after
/// all open walks form closed loops.
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

    // Open chains: start every walk from a corner with degree != 2.
    let mut start_corners: Vec<Corner> = adjacency
        .iter()
        .filter(|(_, edge_list)| edge_list.len() != 2)
        .map(|(&corner, _)| corner)
        .collect();
    start_corners.sort();
    for start in start_corners {
        // Collect first to avoid borrowing `adjacency` mutably.
        let outgoing: Vec<usize> = adjacency[&start].clone();
        for edge_index in outgoing {
            if visited[edge_index] {
                continue;
            }
            let corners = walk(start, edge_index, &mut visited, &adjacency);
            chains.push(ChainCorners {
                corners,
                closed: false,
            });
        }
    }

    // Leftovers are closed loops (every corner degree 2). Anchor rule is
    // UNVERIFIED against the engine: start at the lexicographically smallest
    // corner (north-most then west-most) and walk deterministically.
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
            continue; // already consumed by an earlier loop walk
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
            corners,
            closed: true,
        });
    }

    // Deterministic orientation: north/west-most chain end first (reproduces
    // the captured Brittany strip's row order). Full reversal flips both row
    // order and the left/right sides, so winding is direction-invariant.
    for chain in &mut chains {
        if !chain.closed {
            let first = chain.corners.first().expect("chain has corners");
            let last = chain.corners.last().expect("chain has corners");
            if last < first {
                chain.corners.reverse();
            }
        }
    }
    chains.sort_by(|a, b| a.corners.first().cmp(&b.corners.first()));
    Ok(chains)
}

/// Emit one chain as a ribbon strip per the captured-engine law (module docs):
/// half-integer Manhattan stations, one fixed-end box3 pass, 0.75-unit
/// central-difference extrusion, per-vertex bilinear heightmap drape, integer
/// `(side, row)` uv.
fn emit_chain_ribbon(
    mesh: &mut BorderWorldMesh,
    inputs: &BorderSynthesisInputs,
    province_a: u32,
    province_b: u32,
    edge_class: ClassifiedBorderEdge,
    chain: &ChainCorners,
) -> Result<()> {
    let width = inputs.width as f64;
    let height = inputs.height as f64;

    // Corner world positions, unwrapped across the x seam so consecutive
    // corners are always exactly 1 unit apart.
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
    // Stations at Manhattan arc {0, 0.5, 1.5, ..., M-0.5, M}: the captured
    // pool's row count is exactly M + 2 for every reproduced strip.
    let mut stations: Vec<[f64; 2]> = Vec::with_capacity(edge_count + 2);
    stations.push(points[0]);
    for k in 0..edge_count {
        // Station k + 0.5 sits mid-edge k (edges are unit length).
        let a = points[k];
        let b = points[k + 1];
        stations.push([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
    }
    stations.push(points[edge_count]);

    // One box3 pass, endpoints fixed.
    let mut rows: Vec<[f64; 2]> = stations.clone();
    for i in 1..stations.len() - 1 {
        rows[i] = [
            (stations[i - 1][0] + stations[i][0] + stations[i + 1][0]) / 3.0,
            (stations[i - 1][1] + stations[i][1] + stations[i + 1][1]) / 3.0,
        ];
    }

    let row_count = rows.len();
    let half_width = f64::from(inputs.border_width) * 0.5;
    let first_vertex = u32::try_from(mesh.vertices.len())
        .map_err(|_| "border world synthesis blocked: too many vertices for u32 indices")?;
    let first_index = u32::try_from(mesh.indices.len())
        .map_err(|_| "border world synthesis blocked: too many indices for u32")?;

    for k in 0..row_count {
        let prev = &rows[k.saturating_sub(1)];
        let next = &rows[(k + 1).min(row_count - 1)];
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
        // uv.x = 0 side at P + half*(dz, -dx)/|d| — the assignment measured on
        // the captured Brittany strip (all 32 vertices reproduced).
        let perp = [dz / len, -dx / len];
        let row = &rows[k];
        for (side, sign) in [(0i32, 1.0f64), (1i32, -1.0f64)] {
            let x = row[0] + perp[0] * half_width * sign;
            let z = row[1] + perp[1] * half_width * sign;
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

    // The captured engine index pattern per row pair (verts L_k=2k, R_k=2k+1):
    // (L_k, L_{k+1}, R_k), (L_{k+1}, R_{k+1}, R_k).
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

/// Independent coastline cross-check.
///
/// The border classification path already counted `coastline_edges` (the
/// number of raster edges classified `LandWater`). This pass derives the
/// water mask a second time through the local province classification rule
/// (`province_type == "sea" || province_type == "lake"`, with a missing id
/// treated as water via `unwrap_or(true)`) and walks the *same* 4-neighbor
/// raster-edge domain — including the wrapped left/right seam and excluding a
/// bottom-row vertical neighbor — that the synthesis loop walked. An edge is a
/// water-mask coastline edge when exactly one of its two pixels is water.
///
/// `water_mask_coastline_edges` is the count from this pass, and
/// `coastline_disagreement_edges` is the number of raster edges where the
/// synthesized `LandWater` decision disagrees with this mask. Because the
/// synthesis path fails closed on unknown/missing province types before this
/// runs, the two rules should agree on the shipped files (disagreement `0`),
/// but the number is computed by two independent passes rather than the same
/// increment, so a real regression in edge enumeration, wrap handling, or
/// water classification would surface here as a nonzero count.
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

/// Water test for border classification: a province is water when its
/// `definition.csv` type is `sea` or `lake`, and a province id absent from
/// `definition.csv` is treated as water (`unwrap_or(true)`).
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

    // An explicit `impassable` adjacency row (map/adjacencies.csv) between this
    // exact land/land province pair overrides the province/state/country rank,
    // including the same-state province case. This is checked here — inside the
    // land/land branch, before any rank decision — so water edges keep their
    // sea material and every land rank is superseded. This is disjoint from the
    // state-level `impassable = yes` wasteland heuristic below.
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

/// Heightmap drape for a border vertex, per the captured-engine alignment law:
/// heightmap grid vertex `(i, j)` (world `(i, ·, j)`) carries texel
/// `(i, (H-1) - j)` at `raw * 0.1`; a border vertex at world `(x, z)` gets the
/// BILINEAR interpolation of that grid. All 32 vertices of the captured state
/// border draw match this to 5e-4 (f32 noise); the two triangle-split
/// interpolations were refuted (residuals up to 0.013).
///
/// `x` wraps modulo the map width (border chains cross the seam); the texel
/// row is clamped at the top/bottom map edges.
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
    // NOTE the texel-row axis: row_f GROWS as world z shrinks, so `top` (y0) is
    // the numerically smaller row = the larger world z. Bilinear is symmetric,
    // so the naming is cosmetic.
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
    use std::fs;
    use std::io::Write;
    use std::path::Path;

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
        // Rows:
        // 1 1 3 4
        // 2 2 3 4
        // 5 5 5 4
        // 1/2 same state -> province; 1/3 same owner different state -> state;
        // 3/4 different owner -> country; 2/5 land/sea -> coast.
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
    fn chain_row_counts_follow_manhattan_plus_two() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        assert!(!mesh.chains.is_empty());
        for chain in &mesh.chains {
            assert_eq!(
                chain.rows,
                chain.manhattan_length + 2,
                "chain for pair ({},{})",
                chain.province_a,
                chain.province_b
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

    /// The captured-engine answer key: the drawn state border between
    /// provinces 9495 (Brittany) and 527 (Loire) from session_BORDERZOOM_3.
    /// The chain's staircase, traced from vanilla provinces.bmp, has 15
    /// corners; the engine strip has 16 rows whose CENTERLINE (midpoint of the
    /// two ribbon vertices) is embedded below. The synthetic fixture rebuilds
    /// the same staircase shape on a small map and must reproduce the same
    /// centerline offsets (station + box3 law).
    #[test]
    fn captured_brittany_centerline_law_reproduced_on_synthetic_staircase() {
        // Captured centerline rows (world x, z), session_BORDERZOOM_3 draw0.
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
        // The chain's corner path in vanilla lattice coordinates, traced from
        // provinces.bmp (pair 9495|527); expressed as offsets so the synthetic
        // fixture can rebuild it near the origin.
        // World corner sequence (x, z):
        // (2781.5,1483.5) (2781.5,1481.5) (2782.5,1481.5) (2782.5,1479.5)
        // (2783.5,1479.5) (2783.5,1477.5) (2782.5,1477.5) (2782.5,1475.5)
        // (2783.5,1475.5) (2783.5,1473.5)
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
        // Expand the corner path into unit-edge staircase points, run the law.
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
    fn class_index_partition_sums_to_total_and_permutes_quads() {
        let mesh = synthesize_border_world_mesh_from_inputs(&synthetic_inputs()).expect("mesh");
        let partition = mesh.class_index_partition();
        assert_eq!(partition.indices.len(), mesh.indices.len());
        let total: u32 = partition.ranges.iter().map(|range| range.index_count).sum();
        assert_eq!(total as usize, mesh.indices.len());
        // Contiguous, non-overlapping, in draw order.
        let mut cursor = 0u32;
        for range in &partition.ranges {
            assert_eq!(range.start_index, cursor);
            cursor += range.index_count;
        }
        // The reordered buffer is a permutation of the original triangles.
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
        // Every range's indices reference only vertices of chains with that
        // material.
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
        // 3 (owner AAA) vs 4 (owner BBB) would be Country; the adjacency row
        // must override to Impassable.
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
        // 1 and 2 share state 10 (Province rank); the adjacency row overrides.
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
        // 4x4 map: province 2 is a 2x2 island inside province 1 -> the 1|2
        // boundary is a closed loop with no junction corner.
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
        assert_eq!(chain.rows, 10);
        // The ribbon closes: first and last centerline rows coincide.
        let centerline = mesh.chain_centerline(chain);
        let first = centerline.first().unwrap();
        let last = centerline.last().unwrap();
        assert!((first[0] - last[0]).abs() < 1e-5 && (first[2] - last[2]).abs() < 1e-5);
    }

    #[test]
    fn x_wrap_chain_uses_unwrapped_coordinates() {
        // 4x2 map: province 1 is the single top-left pixel, so its boundary
        // chain crosses the x seam (the wrapped vertical edge between columns
        // 3 and 0) and continues onto column 0 — corner unwrapping must keep
        // consecutive ribbon rows adjacent.
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
        // No ribbon step may span more than 2 world units (an un-unwrapped
        // seam chain would produce a ~width-long step).
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
    fn sea_crossing_row_does_not_reclassify_raster_edges() {
        let mut inputs = synthetic_inputs();
        // A `sea` adjacency row between 1 and 2 must NOT become impassable.
        // (parse_impassable_adjacencies already drops sea rows; simulate the
        // parsed result: the pair set stays empty.)
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
        // Pair (2,5) is land/sea; an impassable row for it must not flip the
        // coastline material.
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
    fn resolver_minimal_map_synthesizes_single_chain() {
        let dir = temp_test_dir("border_resolver_minimal");
        write_minimal_border_game_files(&dir, true);
        let resolver = GameFileResolver::vanilla(&dir).expect("resolver");
        let mesh = synthesize_border_world_mesh_from_resolver(&resolver).expect("mesh");
        // 2x1 map, two provinces, same state -> one province chain. The
        // vertical boundary edge column is 1 unit long; but the seam also
        // separates the two provinces (x wrap), so there are exactly two
        // chains (interior + seam).
        assert_eq!(mesh.stats.raster_edges, 2);
        assert_eq!(mesh.stats.chains, 2);
        for chain in &mesh.chains {
            assert_eq!(chain.material, BorderMaterial::Province);
            assert_eq!(chain.manhattan_length, 1);
            assert_eq!(chain.rows, 3);
        }
    }

    /// Full-loop evidence gate against the CAPTURED ENGINE DRAW
    /// (session_BORDERZOOM_3): synthesizing from the real vanilla files must
    /// reproduce ALL 32 vertices of the drawn Brittany|Loire state-border
    /// strip — positions (x, y, z, heightmap drape included) within 1e-3 and
    /// integer uv exactly. Gated on BATTLE_PLAN_GAME_DIR like the other vanilla
    /// evidence gates.
    #[test]
    fn gate_vanilla_captured_border_strip_evidence() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_captured_border_strip_evidence: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
        // (x, y, z, uv.x, uv.y) per captured vertex, borders_frame_draw000,
        // base_vertex 57074, rows interleaved (side 0, side 1).
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
