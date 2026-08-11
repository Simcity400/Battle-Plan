//! CHUNK1 — offline terrain-chunk mesh / LOD / CB1 generator from installed content.
//!
//! The engine draws terrain as a fixed 65×65 integer vertex grid (64×64 cells)
//! tessellated once, re-used for every chunk at every LOD, with per-tile height
//! data and a per-tile constant buffer. Today [`crate::terrain_chunk_synthesis`]
//! CLONES a captured chunk template (VB / IB / input-layout / CB1) and blocks
//! without one at the target LOD — the largest remaining captured dependency.
//! This module regenerates every byte of those buffers from `map/heightmap.bmp`
//! plus the shipped-shader vertex law, validated BYTE-EXACT against the captured
//! templates so the render path downstream is unchanged.
//!
//! # The vertex law, straight from the shipped shader
//!
//! `gfx/FX/pdxmap.shader` (`VS_INPUT_TERRAIN_NOTEXTURE` / `main`, verbatim):
//!
//! ```text
//! VertexStruct VS_INPUT_TERRAIN_NOTEXTURE {
//!     int4 position : POSITION;   // slot0, R16G16B16A16_SINT, stride 8
//!     int2 height   : TEXCOORD0;  // slot1, R16G16_SINT,       stride 4
//! };
//! float2 pos = VertexIn.position.xy * QuadOffset_Scale_IsDetail.z
//!                                   + QuadOffset_Scale_IsDetail.xy;
//! float vSatPosZ     = saturate( VertexIn.position.z );        // z in [0..4]
//! float vUseAltHeight = vSatPosZ * vSnap[ int(position.z - 1) ];
//! pos += vUseAltHeight
//!      * float2( 1 - position.w, position.w )                  // edge direction
//!      * QuadOffset_Scale_IsDetail.z;
//! float vHeight = VertexIn.height.x * (1 - vUseAltHeight)
//!               + VertexIn.height.y * vUseAltHeight;
//! vHeight *= 0.01f;
//! VertexOut.prepos = float3( pos.x, vHeight, pos.y );
//! ```
//! and `gfx/FX/pdxmap.fxh` `ConstantBuffer(1,32)`:
//! ```text
//! float4x4 ShadowMapTextureMatrix;                              // rows 0-3
//! float4   QuadOffset_Scale_IsDetail;                           // row 4
//! float4   vSnap;                                               // row 5
//! float4   vBorderLookup_HeightScale_UseMultisample_SeasonLerp; // row 6
//! ```
//!
//! From those two files, field-by-field:
//! - `position.xy = (col, row)` — the grid coord (`col = i%65`, `row = i//65`).
//! - `position.z` — edge-stitch class in `1..=4` on the odd boundary vertices,
//!   `0` in the interior; it indexes `vSnap` and toggles the alt (neighbour)
//!   height, i.e. removes the T-junction crack when the neighbour tile is a
//!   coarser LOD. `position.w` picks the snap direction along that edge.
//! - `QuadOffset_Scale_IsDetail.xy = (min_x, min_z)`, `.z = scale`
//!   (`tile_size / 64`), `.w = is_detail (0)`.
//! - `vSnap` = the per-draw detail-edge mask (0/1 per edge), runtime-LOD state.
//! - `height.x` = this vertex's height, `height.y` = the alt/neighbour height
//!   used when snapping; world `y = raw_heightmap_byte * 0.1` and the engine
//!   stores it as `i16 = raw * 10` (so `vHeight = i16 * 0.01 = raw * 0.1`).
//!
//! Every generated field is either PURE LAW (grid coords, index winding) or
//! `heightmap.bmp` + law (the two height streams / the CB1 offset+scale). The
//! only field that is genuinely runtime-LOD state, not a file, is `vSnap`
//! (detail edge flags); for a uniform-LOD offline synthesis every interior edge
//! borders a same-size neighbour so there is no T-junction and `vSnap = 0`.
//!
//! The mixed-LOD `vSnap` mask is no longer a bare capture dependency: it is
//! DERIVED from the per-tile LOD map by [`crate::terrain_lod_law`] (LODSNAP) —
//! `vSnap[edge] = 1` iff the neighbour across that edge is coarser. That law is
//! validated byte-exact against the captured corpus (see
//! `terrain_vsnap_law_validate_corpus`), so a caller with the LOD map computes
//! `vSnap` without reading captured CB1. The LOD LEVEL ASSIGNMENT upstream of
//! `vSnap` (which world tile size each region gets as a function of camera) is
//! also derived now — [`crate::terrain_lod_law::terrain_lod_stride_map`]
//! (LODRING), ported from the decompiled `CPdxMap::UpdateTerrainTessellation`
//! and byte-exact on the captured self-LOD (44/44, 47/47, 7/7). So the full
//! terrain draw set is camera-derivable; the one open residual is the engine's
//! frustum coarsening of off-screen neighbours (ledger §2.9 LODRING row).

use crate::*;
use std::collections::BTreeMap;

/// Vertices per grid side (`65`). 64 cells + 1 shared edge vertex.
pub const TERRAIN_CHUNK_GRID_SIDE_VERTS: u32 = 65;
/// Cells per grid side (`64`).
pub const TERRAIN_CHUNK_GRID_SIDE_CELLS: u32 = 64;
/// Total grid vertices (`65 * 65 = 4225`).
pub const TERRAIN_CHUNK_GRID_VERTS: u32 =
    TERRAIN_CHUNK_GRID_SIDE_VERTS * TERRAIN_CHUNK_GRID_SIDE_VERTS;
/// Total indices (`64 * 64 * 6 = 24576`), the fixed `draw_count`.
pub const TERRAIN_CHUNK_INDEX_COUNT: u32 =
    TERRAIN_CHUNK_GRID_SIDE_CELLS * TERRAIN_CHUNK_GRID_SIDE_CELLS * 6;
/// `slot0` byte length (`4225 verts * 8 bytes`).
pub const TERRAIN_CHUNK_SLOT0_BYTES: usize = TERRAIN_CHUNK_GRID_VERTS as usize * 8;
/// `slot1` per-tile window byte length (`4225 verts * 4 bytes`).
pub const TERRAIN_CHUNK_SLOT1_TILE_BYTES: usize = TERRAIN_CHUNK_GRID_VERTS as usize * 4;
/// Index buffer byte length (`24576 * 2`, `R16_UINT`).
pub const TERRAIN_CHUNK_INDEX_BYTES: usize = TERRAIN_CHUNK_INDEX_COUNT as usize * 2;
/// VS CB1 byte length (`7 float4 rows = 112 bytes`).
pub const TERRAIN_CHUNK_CB1_BYTES: usize = 112;

/// DXGI formats the engine's terrain IA layout declares (matches the captured
/// `input_layout` and the shader's `int4`/`int2` inputs).
pub const TERRAIN_CHUNK_SLOT0_POSITION_FORMAT: u32 = 14; // R16G16B16A16_SINT
pub const TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT: u32 = 38; // R16G16_SINT
pub const TERRAIN_CHUNK_INDEX_FORMAT: u32 = 57; // R16_UINT
pub const TERRAIN_CHUNK_SLOT0_STRIDE: u32 = 8;
pub const TERRAIN_CHUNK_SLOT1_STRIDE: u32 = 4;

/// The world size of one grid cell for a given tile size (`tile_size / 64`).
/// This is `QuadOffset_Scale_IsDetail.z` in the shader. Rejects tile sizes that
/// are not an integer multiple of the 64-cell grid, because the engine only
/// ever emits power-of-two tiles (128/256/512) that divide evenly.
pub fn terrain_chunk_scale_for_tile_size(tile_size: u32) -> Result<f32> {
    if tile_size == 0 || tile_size % TERRAIN_CHUNK_GRID_SIDE_CELLS != 0 {
        return Err(format!(
            "terrain mesh law: tile size {tile_size} is not a positive multiple of {} cells",
            TERRAIN_CHUNK_GRID_SIDE_CELLS
        )
        .into());
    }
    Ok((tile_size / TERRAIN_CHUNK_GRID_SIDE_CELLS) as f32)
}

/// The edge-stitch class (`position.z`) and snap direction (`position.w`) for a
/// grid vertex, exactly as the captured `slot0` encodes and the shader consumes.
///
/// - bottom edge (`row == 0`), odd column      → `z = 1`, `w = 0`
/// - top edge    (`row == 64`), odd column     → `z = 2`, `w = 0`
/// - left edge   (`col == 0`), odd row         → `z = 3`, `w = 1`
/// - right edge  (`col == 64`), odd row        → `z = 4`, `w = 1`
/// - everything else                           → `z = 0`, `w = 0`
///
/// `z` indexes `vSnap[z-1]` (four edges) and `w` selects the snap axis via
/// `float2(1-w, w)`: horizontal edges snap along x (`w=0`), vertical edges snap
/// along z (`w=1`). Only ODD boundary vertices carry a class — they are the mid
/// points that a coarser neighbour tile does not have, so they must collapse
/// onto the neighbour's coarse edge to close the T-junction.
pub fn terrain_chunk_vertex_edge_class(col: u32, row: u32) -> (i16, i16) {
    let last = TERRAIN_CHUNK_GRID_SIDE_CELLS; // 64
    if row == 0 && col % 2 == 1 {
        (1, 0)
    } else if row == last && col % 2 == 1 {
        (2, 0)
    } else if col == 0 && row % 2 == 1 {
        (3, 1)
    } else if col == last && row % 2 == 1 {
        (4, 1)
    } else {
        (0, 0)
    }
}

/// Generate the shared `slot0` POSITION stream (33800 bytes). PURE LAW — no
/// file input, identical for every chunk and every LOD (matches the single
/// shared capture hash). Layout per vertex `i` (`R16G16B16A16_SINT`):
/// `(col, row, edge_class, snap_dir)`.
pub fn generate_terrain_chunk_slot0() -> Vec<u8> {
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_SLOT0_BYTES);
    for i in 0..TERRAIN_CHUNK_GRID_VERTS {
        let col = i % TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let row = i / TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let (z, w) = terrain_chunk_vertex_edge_class(col, row);
        out.extend_from_slice(&(col as i16).to_le_bytes());
        out.extend_from_slice(&(row as i16).to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_SLOT0_BYTES);
    out
}

/// Generate the shared index buffer (49152 bytes, `R16_UINT`). PURE LAW —
/// identical for every chunk and LOD. Checkerboard-flipped tessellation: for
/// cell `(cx, cz)` with base vertex `b = cz*65 + cx`, `(cx+cz)` even splits the
/// quad along the `b→b+66` diagonal, odd splits along `b+1→b+65`. Alternating
/// the diagonal per cell cancels the directional bias a uniform split would
/// leave in the lit terrain.
pub fn generate_terrain_chunk_index() -> Vec<u8> {
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_INDEX_BYTES);
    let side = TERRAIN_CHUNK_GRID_SIDE_VERTS as u16; // 65
    for cz in 0..TERRAIN_CHUNK_GRID_SIDE_CELLS as u16 {
        for cx in 0..TERRAIN_CHUNK_GRID_SIDE_CELLS as u16 {
            let b = cz * side + cx;
            let tri: [u16; 6] = if (cx + cz) % 2 == 0 {
                [b, b + 1, b + side + 1, b, b + side + 1, b + side]
            } else {
                [b, b + 1, b + side, b + 1, b + side + 1, b + side]
            };
            for idx in tri {
                out.extend_from_slice(&idx.to_le_bytes());
            }
        }
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_INDEX_BYTES);
    out
}

/// Sample the heightmap raw byte at an integer world position, wrapping in x
/// (the map wraps east-west) and clamping in z. `by = 0` is the TOP image row;
/// world z increases southward so `bmp_row = height - 1 - world_z`. Matches the
/// `load_bmp_indexed` top-down `pixels[y*width + x]` layout.
fn sample_heightmap_raw(heightmap: &IndexImage, world_x: i64, world_z: i64) -> u8 {
    let width = heightmap.width as i64;
    let height = heightmap.height as i64;
    let mut bx = world_x % width;
    if bx < 0 {
        bx += width;
    }
    let by = (height - 1 - world_z).clamp(0, height - 1);
    heightmap.pixels[(by * width + bx) as usize]
}

/// The engine's fixed-point height for a world position: `i16 = raw * 10`
/// (world `y = raw * 0.1`; the shader multiplies the i16 by `0.01`).
fn terrain_chunk_height_i16(heightmap: &IndexImage, world_x: i64, world_z: i64) -> i16 {
    i16::from(sample_heightmap_raw(heightmap, world_x, world_z)) * 10
}

/// Generate one tile's `slot1` height window (16900 bytes, `R16G16_SINT`) from
/// the heightmap. `x` = primary height at this vertex; `y` = the alt/neighbour
/// height used by the T-junction snap. On interior vertices `x == y`. On an odd
/// boundary vertex the alt height is the height of the NEXT EVEN neighbour along
/// that edge — the coarse-neighbour vertex the mid point collapses onto:
/// bottom/top edge → `(col+1, row)`; left/right edge → `(col, row+1)`.
pub fn generate_terrain_chunk_slot1(
    heightmap: &IndexImage,
    min_x: i64,
    min_z: i64,
    tile_size: u32,
) -> Result<Vec<u8>> {
    let scale = terrain_chunk_scale_for_tile_size(tile_size)? as i64;
    let last = TERRAIN_CHUNK_GRID_SIDE_CELLS; // 64
    let mut out = Vec::with_capacity(TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    let world = |col: u32, row: u32| (min_x + col as i64 * scale, min_z + row as i64 * scale);
    for i in 0..TERRAIN_CHUNK_GRID_VERTS {
        let col = i % TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let row = i / TERRAIN_CHUNK_GRID_SIDE_VERTS;
        let (wx, wz) = world(col, row);
        let primary = terrain_chunk_height_i16(heightmap, wx, wz);
        let alt = if row == 0 && col % 2 == 1 {
            let (nx, nz) = world(col + 1, 0);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if row == last && col % 2 == 1 {
            let (nx, nz) = world(col + 1, last);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if col == 0 && row % 2 == 1 {
            let (nx, nz) = world(0, row + 1);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else if col == last && row % 2 == 1 {
            let (nx, nz) = world(last, row + 1);
            terrain_chunk_height_i16(heightmap, nx, nz)
        } else {
            primary
        };
        out.extend_from_slice(&primary.to_le_bytes());
        out.extend_from_slice(&alt.to_le_bytes());
    }
    debug_assert_eq!(out.len(), TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    Ok(out)
}

/// Generate the VS CB1 (112 bytes) for one tile. `detail_edge_flags` is `vSnap`
/// — the per-edge LOD-snap mask; pass `[0.0; 4]` for uniform-LOD synthesis (no
/// T-junctions), or the captured per-tile mask for a mixed-LOD quadtree.
///
/// Layout (7 `float4` rows, little-endian):
/// - rows 0-3 (`ShadowMapTextureMatrix`): zero (the engine leaves it identity-
///   free here; every captured chunk records it all-zero).
/// - row 4 (`QuadOffset_Scale_IsDetail`): `[min_x, min_z, scale, 0]`.
/// - row 5 (`vSnap`): `detail_edge_flags`.
/// - row 6 (`vBorderLookup_HeightScale_UseMultisample_SeasonLerp`):
///   `[0, 0.1, scale, 0]` — `0.1` is the raw→world height scale.
pub fn generate_terrain_chunk_cb1(
    min_x: f32,
    min_z: f32,
    tile_size: u32,
    detail_edge_flags: [f32; 4],
) -> Result<Vec<u8>> {
    let scale = terrain_chunk_scale_for_tile_size(tile_size)?;
    let mut out = vec![0u8; TERRAIN_CHUNK_CB1_BYTES];
    let write_row = |out: &mut [u8], row: usize, v: [f32; 4]| {
        for (k, value) in v.iter().enumerate() {
            let start = row * 16 + k * 4;
            out[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
    };
    write_row(&mut out, 4, [min_x, min_z, scale, 0.0]);
    write_row(&mut out, 5, detail_edge_flags);
    write_row(&mut out, 6, [0.0, 0.1, scale, 0.0]);
    Ok(out)
}

/// The relative path of the heightmap the generator samples, resolved through
/// the mod-aware [`GameFileResolver`] (no bare vanilla joins — a mod that
/// overrides `map/heightmap.bmp` is honoured, matching the mod-support mandate).
pub const TERRAIN_CHUNK_HEIGHTMAP_SOURCE: &str = "map/heightmap.bmp";

/// One captured chunk-draw the generator reproduced (or failed to).
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainMeshLawChunkComparison {
    pub chunk_id: String,
    pub tile_size: u32,
    pub min_x: f32,
    pub min_z: f32,
    /// Byte offset of this tile's `slot1` window inside the captured pool buffer.
    pub slot1_offset: u32,
    pub slot0_identical: bool,
    pub slot1_identical: bool,
    pub index_identical: bool,
    pub cb1_identical: bool,
    /// First differing byte, per stream, when a stream mismatches.
    pub first_mismatch: Option<(&'static str, usize)>,
}

impl TerrainMeshLawChunkComparison {
    pub fn identical(&self) -> bool {
        self.slot0_identical && self.slot1_identical && self.index_identical && self.cb1_identical
    }
}

/// Per-LOD roll-up of the corpus gate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMeshLawLodTally {
    pub tile_size: u32,
    pub draws_checked: usize,
    pub draws_identical: usize,
}

/// The corpus byte-exact gate result for one capture session.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainMeshLawCorpusReport {
    pub session_label: String,
    pub draws_checked: usize,
    pub draws_identical: usize,
    pub per_lod: Vec<TerrainMeshLawLodTally>,
    pub comparisons: Vec<TerrainMeshLawChunkComparison>,
}

impl TerrainMeshLawCorpusReport {
    pub fn all_identical(&self) -> bool {
        self.draws_checked > 0 && self.draws_checked == self.draws_identical
    }
}

/// Locate the VS terrain-draw CB1 binding for a captured chunk draw.
fn terrain_chunk_draw_vs_cb1<'a>(draw: &'a MapFrameDraw) -> Result<&'a MapFrameBufferBinding> {
    draw.constant_buffers
        .iter()
        .find(|b| b.stage == "VS" && b.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or_else(|| "terrain mesh law corpus: chunk draw has no VS terrain CB1".into())
}

/// Read a captured buffer file (VB slot / IB / CB) as raw bytes, verifying the
/// recorded byte length. `bundle_dir` is the capture directory.
fn read_captured_buffer(
    bundle_dir: &std::path::Path,
    file: &str,
    expected_len: usize,
) -> Result<Vec<u8>> {
    let path = bundle_dir.join(file);
    let bytes = std::fs::read(&path)?;
    if bytes.len() != expected_len {
        return Err(format!(
            "terrain mesh law corpus: {} has {} bytes, metadata expects {}",
            path.display(),
            bytes.len(),
            expected_len
        )
        .into());
    }
    Ok(bytes)
}

/// Validate the generator against every terrain-chunk draw in a captured
/// library, BYTE-EXACT. For each chunk it regenerates slot0 (pure law), the
/// slot1 height window (heightmap + law), the index buffer (pure law) and CB1
/// (law; the captured `vSnap` detail-edge mask is reproduced so the CB1 compares
/// byte-identical even for mixed-LOD tiles), then compares to the captured
/// bytes. The slot1 comparison uses ONLY the fetched `[offset, offset+16900)`
/// window — the draw indexes vertices `0..=4224` from that offset with
/// `base_vertex_location = 0`, so the pooled-buffer tail is never fetched and a
/// window match is a true draw-output match (the replay-vertex-fetch-padding
/// pool-tail caveat is satisfied by construction).
pub fn terrain_mesh_law_validate_corpus(
    library: &NativeD3d11TerrainChunkLibrary,
    heightmap: &IndexImage,
    session_label: &str,
) -> Result<TerrainMeshLawCorpusReport> {
    let expected_slot0 = generate_terrain_chunk_slot0();
    let expected_index = generate_terrain_chunk_index();
    let bundle_dir = library.bundle_dir();
    let mut comparisons = Vec::new();
    let mut per_lod: BTreeMap<u32, TerrainMeshLawLodTally> = BTreeMap::new();

    for chunk in library.chunk_records() {
        let bounds = chunk.bounds;
        let tile_size = terrain_chunk_world_tile_size(chunk)?;

        // --- slot0 (shared, pure law) ---
        let slot0_binding = chunk
            .draw
            .vertex_buffers
            .get("0")
            .ok_or("terrain mesh law corpus: chunk draw has no slot-0 vertex buffer")?;
        let slot0_file = slot0_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: slot-0 binding has no file")?;
        let captured_slot0 = read_captured_buffer(bundle_dir, slot0_file, slot0_binding.byte_len)?;
        let slot0_identical = captured_slot0 == expected_slot0;

        // --- index (shared, pure law) ---
        let index_binding = chunk
            .draw
            .index_buffer
            .as_ref()
            .ok_or("terrain mesh law corpus: chunk draw has no index buffer")?;
        let index_file = index_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: index binding has no file")?;
        let captured_index = read_captured_buffer(bundle_dir, index_file, index_binding.byte_len)?;
        let index_identical = captured_index == expected_index;

        // --- slot1 window (heightmap + law) ---
        let slot1_binding = chunk
            .draw
            .vertex_buffers
            .get("1")
            .ok_or("terrain mesh law corpus: chunk draw has no slot-1 vertex buffer")?;
        let slot1_file = slot1_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: slot-1 binding has no file")?;
        let slot1_offset = slot1_binding
            .offset
            .ok_or("terrain mesh law corpus: slot-1 binding has no offset")?;
        let captured_pool = read_captured_buffer(bundle_dir, slot1_file, slot1_binding.byte_len)?;
        let window_start = slot1_offset as usize;
        let window_end = window_start
            .checked_add(TERRAIN_CHUNK_SLOT1_TILE_BYTES)
            .ok_or("terrain mesh law corpus: slot-1 window overflow")?;
        if window_end > captured_pool.len() {
            return Err(format!(
                "terrain mesh law corpus: chunk {} slot-1 window [{window_start}, {window_end}) exceeds pool length {}",
                chunk.chunk_id,
                captured_pool.len()
            )
            .into());
        }
        let captured_slot1 = &captured_pool[window_start..window_end];
        let expected_slot1 = generate_terrain_chunk_slot1(
            heightmap,
            bounds.min_x as i64,
            bounds.min_z as i64,
            tile_size,
        )?;
        let slot1_identical = captured_slot1 == expected_slot1.as_slice();

        // --- CB1 (law; reproduce captured vSnap mask for byte-identity) ---
        let cb1_binding = terrain_chunk_draw_vs_cb1(&chunk.draw)?;
        let cb1_file = cb1_binding
            .file
            .as_ref()
            .ok_or("terrain mesh law corpus: CB1 binding has no file")?;
        let captured_cb1 = read_captured_buffer(bundle_dir, cb1_file, cb1_binding.byte_len)?;
        let detail_flags = terrain_chunk_cb1_detail_edge_flags_from_bytes(&captured_cb1)?;
        let expected_cb1 =
            generate_terrain_chunk_cb1(bounds.min_x, bounds.min_z, tile_size, detail_flags)?;
        let cb1_identical = captured_cb1 == expected_cb1;

        let first_mismatch = if !slot0_identical {
            first_byte_diff(&captured_slot0, &expected_slot0).map(|i| ("slot0", i))
        } else if !slot1_identical {
            first_byte_diff(captured_slot1, &expected_slot1).map(|i| ("slot1", i))
        } else if !index_identical {
            first_byte_diff(&captured_index, &expected_index).map(|i| ("index", i))
        } else if !cb1_identical {
            first_byte_diff(&captured_cb1, &expected_cb1).map(|i| ("cb1", i))
        } else {
            None
        };

        let comparison = TerrainMeshLawChunkComparison {
            chunk_id: chunk.chunk_id.clone(),
            tile_size,
            min_x: bounds.min_x,
            min_z: bounds.min_z,
            slot1_offset,
            slot0_identical,
            slot1_identical,
            index_identical,
            cb1_identical,
            first_mismatch,
        };
        let tally = per_lod.entry(tile_size).or_insert(TerrainMeshLawLodTally {
            tile_size,
            draws_checked: 0,
            draws_identical: 0,
        });
        tally.draws_checked += 1;
        if comparison.identical() {
            tally.draws_identical += 1;
        }
        comparisons.push(comparison);
    }

    let draws_checked = comparisons.len();
    let draws_identical = comparisons.iter().filter(|c| c.identical()).count();
    Ok(TerrainMeshLawCorpusReport {
        session_label: session_label.to_string(),
        draws_checked,
        draws_identical,
        per_lod: per_lod.into_values().collect(),
        comparisons,
    })
}

/// Grid dimensions of the world at one LOD: `(tiles_x, tiles_z)`.
/// `tiles_x = world_width / tile_size`, `tiles_z = world_height / tile_size`.
pub fn terrain_chunk_pool_grid_dims(
    world_width: u32,
    world_height: u32,
    tile_size: u32,
) -> Result<(u32, u32)> {
    if tile_size == 0 || world_width % tile_size != 0 || world_height % tile_size != 0 {
        return Err(format!(
            "terrain mesh law: world {world_width}x{world_height} is not divisible by tile size {tile_size}"
        )
        .into());
    }
    Ok((world_width / tile_size, world_height / tile_size))
}

/// Regenerate a whole per-LOD `slot1` POOL buffer from the heightmap — every
/// world-grid tile's 16900-byte window, row-major (`offset = (tz*tiles_x+tx) *
/// 16900`). This is the full buffer the engine binds; individual draws fetch
/// only their own window at their recorded offset. Byte-exact to the captured
/// pool (verified for 128/256/512 over 5632×2048).
pub fn generate_terrain_chunk_slot1_pool(
    heightmap: &IndexImage,
    world_width: u32,
    world_height: u32,
    tile_size: u32,
) -> Result<Vec<u8>> {
    let (tiles_x, tiles_z) = terrain_chunk_pool_grid_dims(world_width, world_height, tile_size)?;
    let mut pool =
        Vec::with_capacity(tiles_x as usize * tiles_z as usize * TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    for tz in 0..tiles_z {
        for tx in 0..tiles_x {
            let min_x = (tx * tile_size) as i64;
            let min_z = (tz * tile_size) as i64;
            let window = generate_terrain_chunk_slot1(heightmap, min_x, min_z, tile_size)?;
            pool.extend_from_slice(&window);
        }
    }
    Ok(pool)
}

/// Build the synthetic-payload map that replaces every captured terrain-chunk
/// buffer file with a generator-produced, byte-identical payload, so the render
/// path runs `--chunks-from-files` WITHOUT touching a captured template on disk.
///
/// Each payload is keyed by the exact file name the chunk-draw metadata
/// references and carries the same `fnv1a64` the metadata records — so the
/// existing length+hash guard ([`native_d3d11_read_map_frame_buffer_bytes`])
/// accepts them, and if generation ever drifted from the captured law the guard
/// would reject it (fail-closed). Covers: the shared slot0, the shared index,
/// each LOD's full slot1 pool, and each tile's VS CB1 (using the recorded
/// `vSnap` detail-edge mask so mixed-LOD tiles stay byte-identical).
///
/// `world_rect` is the map's world rectangle (tile pool dimensions derive from
/// it). Returns an error if any generated payload's hash does not match the
/// metadata — meaning the capture is outside the proven law and the caller must
/// fail closed rather than silently render wrong geometry.
pub(crate) fn build_terrain_chunk_synthetic_payloads(
    library: &NativeD3d11TerrainChunkLibrary,
    heightmap: &IndexImage,
    world_width: u32,
    world_height: u32,
) -> Result<BTreeMap<String, MapFrameSyntheticPayload>> {
    let mut payloads: BTreeMap<String, MapFrameSyntheticPayload> = BTreeMap::new();
    let slot0 = generate_terrain_chunk_slot0();
    let slot0_hash = fnv1a64_bytes(&slot0);
    let index = generate_terrain_chunk_index();
    let index_hash = fnv1a64_bytes(&index);
    // cache per-LOD pool bytes so we build each pool once
    let mut pools: BTreeMap<u32, (Vec<u8>, String)> = BTreeMap::new();

    let verify = |file: &str, want: Option<&str>, got: &str| -> Result<()> {
        if let Some(want) = want {
            if want != got {
                return Err(format!(
                    "--chunks-from-files blocked: generated {file} hash {got} != recorded {want}; capture is outside the proven mesh law"
                )
                .into());
            }
        }
        Ok(())
    };

    for chunk in library.chunk_records() {
        let tile_size = terrain_chunk_world_tile_size(chunk)?;
        // slot0 (shared)
        if let Some(binding) = chunk.draw.vertex_buffers.get("0") {
            if let Some(file) = binding.file.clone() {
                verify(&file, binding.fnv1a64.as_deref(), &slot0_hash)?;
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: slot0.clone(),
                        fnv1a64: slot0_hash.clone(),
                        provenance: "chunk1 mesh-law slot0 (65x65 int grid)".to_string(),
                    });
            }
        }
        // index (shared)
        if let Some(binding) = chunk.draw.index_buffer.as_ref() {
            if let Some(file) = binding.file.clone() {
                verify(&file, binding.fnv1a64.as_deref(), &index_hash)?;
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: index.clone(),
                        fnv1a64: index_hash.clone(),
                        provenance: "chunk1 mesh-law index (checkerboard tessellation)".to_string(),
                    });
            }
        }
        // slot1 pool (per LOD)
        if let Some(binding) = chunk.draw.vertex_buffers.get("1") {
            if let Some(file) = binding.file.clone() {
                if !pools.contains_key(&tile_size) {
                    let pool = generate_terrain_chunk_slot1_pool(
                        heightmap,
                        world_width,
                        world_height,
                        tile_size,
                    )?;
                    let hash = fnv1a64_bytes(&pool);
                    pools.insert(tile_size, (pool, hash));
                }
                let (pool, hash) = pools.get(&tile_size).expect("pool present");
                verify(&file, binding.fnv1a64.as_deref(), hash)?;
                if binding.byte_len != pool.len() {
                    return Err(format!(
                        "--chunks-from-files blocked: generated LOD {tile_size} slot1 pool is {} bytes, metadata records {}",
                        pool.len(),
                        binding.byte_len
                    )
                    .into());
                }
                payloads
                    .entry(file)
                    .or_insert_with(|| MapFrameSyntheticPayload {
                        bytes: pool.clone(),
                        fnv1a64: hash.clone(),
                        provenance: format!(
                            "chunk1 mesh-law slot1 pool (LOD {tile_size}, heightmap-derived)"
                        ),
                    });
            }
        }
        // CB1 (per tile) — reproduce recorded vSnap mask for byte-identity
        let cb1_binding = terrain_chunk_draw_vs_cb1(&chunk.draw)?;
        if let Some(file) = cb1_binding.file.clone() {
            let captured_cb1 =
                read_captured_buffer(library.bundle_dir(), &file, cb1_binding.byte_len)?;
            let detail_flags = terrain_chunk_cb1_detail_edge_flags_from_bytes(&captured_cb1)?;
            let cb1 = generate_terrain_chunk_cb1(
                chunk.bounds.min_x,
                chunk.bounds.min_z,
                tile_size,
                detail_flags,
            )?;
            let hash = fnv1a64_bytes(&cb1);
            verify(&file, cb1_binding.fnv1a64.as_deref(), &hash)?;
            payloads
                .entry(file)
                .or_insert_with(|| MapFrameSyntheticPayload {
                    bytes: cb1,
                    fnv1a64: hash,
                    provenance: "chunk1 mesh-law CB1 (QuadOffset/scale/heightscale)".to_string(),
                });
        }
    }
    Ok(payloads)
}

fn first_byte_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// Load the heightmap through the mod-aware resolver and run the corpus gate.
pub fn terrain_mesh_law_validate_corpus_with_resolver(
    resolver: &GameFileResolver,
    library: &NativeD3d11TerrainChunkLibrary,
    session_label: &str,
) -> Result<TerrainMeshLawCorpusReport> {
    let heightmap_path =
        resolver.resolve_required(TERRAIN_CHUNK_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
    let heightmap = load_bmp_indexed(&heightmap_path)?;
    terrain_mesh_law_validate_corpus(library, &heightmap, session_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    #[test]
    fn scale_law_matches_captured_lods() {
        assert_eq!(terrain_chunk_scale_for_tile_size(128).unwrap(), 2.0);
        assert_eq!(terrain_chunk_scale_for_tile_size(256).unwrap(), 4.0);
        assert_eq!(terrain_chunk_scale_for_tile_size(512).unwrap(), 8.0);
        assert!(terrain_chunk_scale_for_tile_size(0).is_err());
        assert!(terrain_chunk_scale_for_tile_size(100).is_err());
    }

    #[test]
    fn slot0_is_the_65x65_int_grid_with_edge_classes() {
        let slot0 = generate_terrain_chunk_slot0();
        assert_eq!(slot0.len(), TERRAIN_CHUNK_SLOT0_BYTES);
        let read = |i: usize| {
            let b = &slot0[i * 8..i * 8 + 8];
            [
                i16::from_le_bytes([b[0], b[1]]),
                i16::from_le_bytes([b[2], b[3]]),
                i16::from_le_bytes([b[4], b[5]]),
                i16::from_le_bytes([b[6], b[7]]),
            ]
        };
        // interior: (col,row,0,0)
        assert_eq!(read(0), [0, 0, 0, 0]);
        assert_eq!(read(66), [1, 1, 0, 0]);
        assert_eq!(read(4224), [64, 64, 0, 0]);
        // bottom-edge odd col -> z=1
        assert_eq!(read(1), [1, 0, 1, 0]);
        // left-edge odd row -> z=3,w=1 (vertex 65 = col0,row1)
        assert_eq!(read(65), [0, 1, 3, 1]);
        // right-edge odd row -> z=4,w=1 (vertex 129 = col64,row1)
        assert_eq!(read(129), [64, 1, 4, 1]);
        // top-edge odd col -> z=2 (row64, col1 = 64*65+1 = 4161)
        assert_eq!(read(4161), [1, 64, 2, 0]);
        // exactly 128 boundary-odd vertices carry a class (4 edges * 32 each)
        let classed = (0..TERRAIN_CHUNK_GRID_VERTS as usize)
            .filter(|&i| read(i)[2] != 0)
            .count();
        assert_eq!(classed, 128);
    }

    #[test]
    fn index_buffer_is_checkerboard_tessellation() {
        let ib = generate_terrain_chunk_index();
        assert_eq!(ib.len(), TERRAIN_CHUNK_INDEX_BYTES);
        let idx = |k: usize| u16::from_le_bytes([ib[k * 2], ib[k * 2 + 1]]);
        // cell (0,0) even -> b..b+66 diagonal
        assert_eq!(
            [idx(0), idx(1), idx(2), idx(3), idx(4), idx(5)],
            [0, 1, 66, 0, 66, 65]
        );
        // cell (1,0) odd -> b+1..b+65 diagonal, b=1
        assert_eq!(
            [idx(6), idx(7), idx(8), idx(9), idx(10), idx(11)],
            [1, 2, 66, 2, 67, 66]
        );
    }

    fn flat_heightmap(width: usize, height: usize, value: u8) -> IndexImage {
        IndexImage {
            width,
            height,
            pixels: vec![value; width * height],
        }
    }

    #[test]
    fn slot1_flat_heightmap_is_uniform_fixedpoint() {
        let hm = flat_heightmap(5632, 2048, 89);
        let slot1 = generate_terrain_chunk_slot1(&hm, 2048, 1024, 512).unwrap();
        assert_eq!(slot1.len(), TERRAIN_CHUNK_SLOT1_TILE_BYTES);
        // every u and v = 89 * 10 = 890 on a flat map
        for i in 0..TERRAIN_CHUNK_GRID_VERTS as usize {
            let u = i16::from_le_bytes([slot1[i * 4], slot1[i * 4 + 1]]);
            let v = i16::from_le_bytes([slot1[i * 4 + 2], slot1[i * 4 + 3]]);
            assert_eq!((u, v), (890, 890));
        }
    }

    #[test]
    fn slot1_alt_height_is_next_even_neighbour_on_odd_boundary() {
        // Build a heightmap where height depends on x so the odd-boundary alt
        // (next even neighbour) differs from the vertex's own height.
        let (w, h) = (256usize, 256usize);
        let mut hm = flat_heightmap(w, h, 0);
        for y in 0..h {
            for x in 0..w {
                hm.pixels[y * w + x] = (x % 200) as u8;
            }
        }
        // tile at origin, size 128 -> scale 2. Bottom edge (row 0), odd col 1:
        // own world x = 0 + 1*2 = 2 -> raw 2; alt neighbour col 2 -> world x 4 -> raw 4.
        let slot1 = generate_terrain_chunk_slot1(&hm, 0, 0, 128).unwrap();
        let vert = 1usize; // col 1, row 0
        let u = i16::from_le_bytes([slot1[vert * 4], slot1[vert * 4 + 1]]);
        let v = i16::from_le_bytes([slot1[vert * 4 + 2], slot1[vert * 4 + 3]]);
        assert_eq!(u, 2 * 10);
        assert_eq!(v, 4 * 10, "odd bottom-edge alt height = next even col");
    }

    #[test]
    fn cb1_encodes_offset_scale_heightscale() {
        let cb1 = generate_terrain_chunk_cb1(2048.0, 1024.0, 512, [0.0; 4]).unwrap();
        assert_eq!(cb1.len(), TERRAIN_CHUNK_CB1_BYTES);
        let f =
            |off: usize| f32::from_le_bytes([cb1[off], cb1[off + 1], cb1[off + 2], cb1[off + 3]]);
        // rows 0-3 zero
        for k in 0..16 {
            assert_eq!(f(k * 4), 0.0);
        }
        // row 4 QuadOffset_Scale_IsDetail = [minx, minz, scale, 0]
        assert_eq!([f(64), f(68), f(72), f(76)], [2048.0, 1024.0, 8.0, 0.0]);
        // row 5 vSnap = detail edge flags (0 here)
        assert_eq!([f(80), f(84), f(88), f(92)], [0.0; 4]);
        // row 6 = [0, 0.1, scale, 0]
        assert_eq!([f(96), f(100), f(104), f(108)], [0.0, 0.1, 8.0, 0.0]);
    }

    #[test]
    fn cb1_byte_offsets_agree_with_constants() {
        // The generator's row placement must match the shared byte-offset
        // constants the capture-clone path uses.
        assert_eq!(TERRAIN_DRAW_CB1_QUAD_OFFSET_SCALE_BYTE_OFFSET, 64);
        assert_eq!(TERRAIN_DRAW_CB1_DETAIL_EDGE_FLAGS_BYTE_OFFSET, 80);
    }

    #[test]
    fn ia_layout_constants_match_the_decoded_terrain_grid() {
        // The generated IA formats/strides must equal the captured terrain-grid
        // signature the BORD3 classifier decoded (border_capture_evidence): the
        // engine terrain patch is R16G16B16A16_SINT(14)/stride 8 POSITION +
        // R16G16_SINT(38) TEXCOORD + R16_UINT(57) index.
        use crate::border_capture_evidence::dxgi;
        assert_eq!(TERRAIN_CHUNK_SLOT0_POSITION_FORMAT, dxgi::R16G16B16A16_SINT);
        assert_eq!(TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT, dxgi::R16G16_SINT);
        assert_eq!(TERRAIN_CHUNK_INDEX_FORMAT, dxgi::R16_UINT);
        assert_eq!(TERRAIN_CHUNK_SLOT0_STRIDE, 8);
        assert_eq!(TERRAIN_CHUNK_SLOT1_STRIDE, 4);
        // The shared grid: 65*65 verts, 64*64*6 indices, matching DIAG23B.
        assert_eq!(
            TERRAIN_CHUNK_GRID_VERTS,
            crate::border_capture_evidence::DIAG23B_BORDERS_PASS_LAYOUT.vertex_count
        );
        assert_eq!(
            TERRAIN_CHUNK_INDEX_COUNT,
            crate::border_capture_evidence::DIAG23B_BORDERS_PASS_LAYOUT.index_count
        );
    }

    #[test]
    fn slot1_pool_dims_match_captured_lods() {
        // 5632x2048 world: 512->11x4=44, 256->22x8=176, 128->44x16=704 tiles
        // (the captured pool tile counts).
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 512).unwrap(),
            (11, 4)
        );
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 256).unwrap(),
            (22, 8)
        );
        assert_eq!(
            terrain_chunk_pool_grid_dims(5632, 2048, 128).unwrap(),
            (44, 16)
        );
        assert!(terrain_chunk_pool_grid_dims(5632, 2048, 100).is_err());
    }

    #[test]
    fn slot1_window_is_within_pool_and_never_over_fetches() {
        // The draw indexes vertices 0..=4224 from `offset` (base_vertex 0), so
        // the fetched window is exactly one 16900-byte tile; the pool tail is
        // never read. This is the invariant that makes a window match a true
        // draw-output match despite the under-recorded-pool caveat.
        let max_index = generate_terrain_chunk_index()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .max()
            .unwrap();
        assert_eq!(max_index, (TERRAIN_CHUNK_GRID_VERTS - 1) as u16); // 4224
        assert_eq!((max_index as usize + 1) * 4, TERRAIN_CHUNK_SLOT1_TILE_BYTES);
    }

    #[test]
    fn synthetic_payload_overrides_disk_and_never_reads_the_file() {
        // Negative proof that a generated payload is what gets bound: point the
        // reader at a file that does NOT exist on disk, but supply a matching
        // synthetic payload — it must return the payload bytes without touching
        // the (absent) file. If the mesh-buffer path ignored the payload map,
        // this would error with "file not readable".
        let bytes = generate_terrain_chunk_slot0();
        let hash = fnv1a64_bytes(&bytes);
        let file = "terrain_chunk_map_frame_shared_payload_ia_vertex_buffer_slot0_unnamed_1e5572789d573945.d3d11";
        let mut payloads = BTreeMap::new();
        payloads.insert(
            file.to_string(),
            MapFrameSyntheticPayload {
                bytes: bytes.clone(),
                fnv1a64: hash.clone(),
                provenance: "test".to_string(),
            },
        );
        let out = read_map_frame_resource_payload(
            std::path::Path::new("C:/nonexistent-dir-for-chunk1-test"),
            &payloads,
            file,
            bytes.len(),
            &hash,
            "vertex buffer",
        )
        .expect("payload must be returned without touching disk");
        assert_eq!(out, bytes);
        // And with NO payload, the same absent file errors (disk path taken).
        let empty = BTreeMap::new();
        assert!(read_map_frame_resource_payload(
            std::path::Path::new("C:/nonexistent-dir-for-chunk1-test"),
            &empty,
            file,
            bytes.len(),
            &hash,
            "vertex buffer",
        )
        .is_err());
    }

    /// Corpus byte-exact gate against a real captured session. Optional
    /// overrides:
    ///   TERRAIN_MESH_LAW_CORPUS_CAPTURE=<session dir>
    ///   TERRAIN_MESH_LAW_CORPUS_GAME=<game dir>
    /// It asserts every terrain-chunk draw regenerates byte-identical.
    #[test]
    #[cfg_attr(
        not(gate_terrain_mesh_corpus),
        ignore = "a terrain chunk corpus and the game install (TERRAIN_MESH_LAW_CORPUS_CAPTURE plus TERRAIN_MESH_LAW_CORPUS_GAME, else preserved gate18_final plus the default install)"
    )]
    fn corpus_byte_exact_when_session_available() {
        let (capture, game) = corpus::terrain_mesh_corpus()
            .expect("the terrain mesh corpus gate was true at build time");
        let resolver = GameFileResolver::vanilla(game).expect("vanilla resolver");
        let library = NativeD3d11TerrainChunkLibrary::load_records_only(&capture)
            .expect("load chunk library records");
        let report =
            terrain_mesh_law_validate_corpus_with_resolver(&resolver, &library, "env-corpus")
                .expect("run corpus gate");
        eprintln!(
            "corpus gate: {} draws checked, {} identical; per-LOD {:?}",
            report.draws_checked,
            report.draws_identical,
            report
                .per_lod
                .iter()
                .map(|l| (l.tile_size, l.draws_checked, l.draws_identical))
                .collect::<Vec<_>>()
        );
        assert!(report.draws_checked > 0, "corpus had no chunk draws");
        for c in &report.comparisons {
            assert!(
                c.identical(),
                "chunk {} (size {}) mismatched: {:?}",
                c.chunk_id,
                c.tile_size,
                c.first_mismatch
            );
        }
        assert!(report.all_identical());
    }
}
